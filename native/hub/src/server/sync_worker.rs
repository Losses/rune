use super::sync::http::SharedLibrary;
use super::sync_auth::{Authorization, Challenge, Token, message};
use crate::utils::GlobalParams;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use database::actions::file::{ManagedAudioVerifier, materialized_path, resolve_local_id};
use database::actions::library::clean_local_orphans;
use database::sync::model::RuneModel;
use discovery::server::UserStatus;
use rand::rngs::OsRng;
use rsa::{Pss, RsaPrivateKey, pkcs8::DecodePrivateKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use sync::SyncPeer;

const DEFAULT_PEER_PORT: u16 = 7863;
const PEER_CONCURRENCY: usize = 4;

#[derive(Clone, Debug)]
pub(super) struct PeerEndpoint {
    pub fingerprint: String,
    pub trusted_host: String,
    pub host: String,
    pub port: u16,
}

fn endpoint_host_port(value: &str) -> Option<(String, Option<u16>)> {
    if let Ok(addr) = value.parse::<std::net::SocketAddr>() {
        return (addr.port() != 0).then(|| (addr.ip().to_string(), Some(addr.port())));
    }
    let host = value.trim_matches(|c| c == '[' || c == ']');
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Some((host.to_owned(), None));
    }
    if let Some((name, port)) = value.rsplit_once(':')
        && !name.is_empty()
        && !name.contains(':')
        && let Ok(port) = port.parse::<u16>()
    {
        return (port != 0).then(|| (name.to_owned(), Some(port)));
    }
    (!value.is_empty() && !value.contains(':')).then(|| (value.to_owned(), None))
}

pub(super) async fn peer_endpoints(global: &GlobalParams, own: &str) -> Vec<Vec<PeerEndpoint>> {
    let trusted = global
        .cert_validator
        .read()
        .await
        .clone_trusted_entries()
        .await;
    let discovered = global.device_scanner.get_all_devices();
    let mut peers = Vec::new();
    for (fingerprint, hosts) in trusted {
        if fingerprint == own {
            continue;
        }
        let status = global
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&fingerprint)
            .await
            .map(|u| u.status);
        if !outbound_eligible(status) {
            continue;
        }
        let advertised = discovered
            .iter()
            .find(|device| device.fingerprint == fingerprint)
            .and_then(|device| device.api_port);
        let endpoints: Vec<_> = hosts
            .into_iter()
            .filter_map(|trusted_host| {
                let (host, explicit) = endpoint_host_port(&trusted_host)?;
                Some(PeerEndpoint {
                    fingerprint: fingerprint.clone(),
                    trusted_host,
                    host,
                    port: explicit.or(advertised).unwrap_or(DEFAULT_PEER_PORT),
                })
            })
            .collect();
        if !endpoints.is_empty() {
            peers.push(endpoints);
        }
    }
    peers
}

type ReplicaLocks = Arc<
    tokio::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<PeerSession>>>>,
>;

struct PeerSession {
    session: Option<super::sync::client::LibrarySession>,
    client: Option<(String, reqwest::Client)>,
    identity: Option<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    artwork_cursor: http_request::sync::PageCursor,
    last_used: std::time::Instant,
}

#[derive(Clone)]
struct LocalCredentials {
    fingerprint: String,
    private_key: String,
}
impl Default for PeerSession {
    fn default() -> Self {
        Self {
            session: None,
            client: None,
            identity: None,
            artwork_cursor: http_request::sync::PageCursor::default(),
            last_used: std::time::Instant::now(),
        }
    }
}

pub async fn run(
    global: Arc<GlobalParams>,
    library: SharedLibrary,
    notify: Arc<dyn Fn() + Send + Sync>,
    own_fingerprint: String,
    private_key: String,
    cancel: tokio_util::sync::CancellationToken,
) {
    let locks: ReplicaLocks = Arc::default();
    let credentials = LocalCredentials {
        fingerprint: own_fingerprint,
        private_key,
    };
    let metadata = run_loop(
        global.clone(),
        library.clone(),
        notify.clone(),
        credentials.clone(),
        cancel.clone(),
        true,
        locks.clone(),
    );
    let audio = run_loop(
        global.clone(),
        library.clone(),
        notify.clone(),
        credentials,
        cancel.clone(),
        false,
        locks,
    );
    tokio::join!(
        join_workers(metadata, audio),
        verify_managed_audio(global.clone(), notify, cancel.clone()),
        maintain_metadata(library, global.main_db.clone(), cancel)
    );
}

async fn maintain_metadata(
    library: SharedLibrary,
    db: Arc<database::connection::MainDbConnection>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut maintenance = database::sync::Maintenance::default();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = cancel.cancelled() => return, _ = interval.tick() => () }
        for _ in 0..32 {
            let store = library.0.read().await.clone();
            let result = tokio::select! {
                _ = cancel.cancelled() => return,
                result = store.maintain(&mut maintenance) => result,
            };
            if let Err(error) = result {
                log::warn!("Metadata maintenance failed: {error:#}");
                break;
            }
            if let Err(error) = clean_local_orphans(&db).await {
                log::warn!("Local metadata cleanup failed: {error:#}");
                break;
            }
            tokio::task::yield_now().await;
        }
    }
}

async fn verify_managed_audio(
    global: Arc<GlobalParams>,
    notify: Arc<dyn Fn() + Send + Sync>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut verifier = ManagedAudioVerifier::default();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = cancel.cancelled() => return, _ = interval.tick() => {} }
        let result = tokio::select! {
            _ = cancel.cancelled() => return,
            result = verifier.step(&global.main_db, &global.fsio, std::path::Path::new(&*global.lib_path), 4 * 1024 * 1024) => result,
        };
        match result {
            Ok(step) if step.invalidated => notify(),
            Ok(_) => {}
            Err(error) => log::warn!("Managed audio verification failed: {error:#}"),
        }
    }
}

async fn join_workers<M, A>(metadata: M, audio: A)
where
    M: std::future::Future<Output = ()>,
    A: std::future::Future<Output = ()>,
{
    tokio::join!(metadata, audio);
}

async fn run_loop(
    global: Arc<GlobalParams>,
    library: SharedLibrary,
    notify: Arc<dyn Fn() + Send + Sync>,
    credentials: LocalCredentials,
    cancel: tokio_util::sync::CancellationToken,
    metadata: bool,
    locks: ReplicaLocks,
) {
    let discover = || peer_endpoints(&global, &credentials.fingerprint);
    let work = |hosts: Vec<PeerEndpoint>| {
        let global = global.clone();
        let library = library.clone();
        let notify = notify.clone();
        let credentials = credentials.clone();
        let cancel = cancel.clone();
        let locks = locks.clone();
        async move {
            let fingerprint = hosts[0].fingerprint.clone();
            for endpoint in hosts {
                let result = tokio::select! {
                    _ = cancel.cancelled() => return (fingerprint, Duration::from_secs(30)),
                    _ = wait_for_revocation(&global, &endpoint.fingerprint, &endpoint.trusted_host) => Err(anyhow::anyhow!("peer permission revoked")),
                    result = async {
                        let operation = sync_one(&global, &library, &notify, &credentials, &endpoint, metadata, &locks);
                        if metadata {
                            tokio::time::timeout(Duration::from_secs(60), operation).await.map_err(|_| anyhow::anyhow!("peer operation timed out"))?
                        } else {
                            operation.await
                        }
                    } => result,
                };
                match result {
                    Ok(complete) => {
                        return (
                            fingerprint,
                            if complete {
                                Duration::from_secs(30)
                            } else {
                                Duration::from_millis(100)
                            },
                        );
                    }
                    Err(error) => log::warn!(
                        "Background {} with paired device failed: {error:#}",
                        if metadata {
                            "metadata synchronization"
                        } else {
                            "audio download"
                        }
                    ),
                }
            }
            (fingerprint, Duration::from_secs(30))
        }
    };
    schedule_peers(
        discover,
        work,
        |hosts| hosts[0].fingerprint.clone(),
        if metadata { PEER_CONCURRENCY } else { 1 },
        cancel.clone(),
        || async {
            if metadata {
                locks.lock().await.retain(|_, entry| {
                    Arc::strong_count(entry) > 1
                        || entry.try_lock().map_or(true, |state| {
                            state.last_used.elapsed() < Duration::from_secs(900)
                        })
                });
            }
        },
    )
    .await;
}

async fn schedule_peers<P, D, DF, W, WF, K, C, CF>(
    discover: D,
    work: W,
    key: K,
    limit: usize,
    cancel: tokio_util::sync::CancellationToken,
    cleanup: C,
) where
    D: Fn() -> DF,
    DF: std::future::Future<Output = Vec<P>>,
    W: Fn(P) -> WF,
    WF: std::future::Future<Output = (String, Duration)>,
    K: Fn(&P) -> String,
    C: Fn() -> CF,
    CF: std::future::Future<Output = ()>,
{
    use futures::{StreamExt, stream::FuturesUnordered};
    use std::collections::{HashMap, HashSet};
    use tokio::time::Instant;
    let mut due = HashMap::<String, Instant>::new();
    let mut running = HashSet::new();
    let mut tasks = FuturesUnordered::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next_due = None;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tick.tick() => {},
            _ = async { match next_due { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => {},
            Some((fingerprint, wait)) = tasks.next(), if !tasks.is_empty() => {
                running.remove(&fingerprint);
                due.insert(fingerprint, Instant::now() + wait);
            }
        }
        let peers = tokio::select! {
            _ = cancel.cancelled() => return,
            peers = discover() => peers,
        };
        let eligible: HashSet<_> = peers.iter().map(&key).collect();
        due.retain(|fingerprint, _| eligible.contains(fingerprint));
        for peer in peers {
            if tasks.len() >= limit {
                break;
            }
            let fingerprint = key(&peer);
            if !running.contains(&fingerprint)
                && due
                    .get(&fingerprint)
                    .is_none_or(|deadline| *deadline <= Instant::now())
            {
                running.insert(fingerprint);
                tasks.push(work(peer));
            }
        }
        next_due = if tasks.len() < limit {
            due.iter()
                .filter(|(fingerprint, deadline)| {
                    eligible.contains(*fingerprint)
                        && !running.contains(*fingerprint)
                        && **deadline > Instant::now()
                })
                .map(|(_, deadline)| *deadline)
                .min()
        } else {
            None
        };
        cleanup().await;
    }
}

fn outbound_eligible(status: Option<UserStatus>) -> bool {
    matches!(status, None | Some(UserStatus::Approved))
}

pub(super) async fn wait_for_revocation(global: &GlobalParams, fingerprint: &str, host: &str) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let status = global
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(fingerprint)
            .await
            .map(|u| u.status);
        let approved = outbound_eligible(status);
        let trusted = global
            .cert_validator
            .read()
            .await
            .get_hosts_for_fingerprint(fingerprint)
            .await;
        if !approved || !trusted.iter().any(|candidate| candidate == host) {
            return;
        }
    }
}

async fn sync_one(
    global: &Arc<GlobalParams>,
    library: &SharedLibrary,
    notify: &Arc<dyn Fn() + Send + Sync>,
    credentials: &LocalCredentials,
    endpoint: &PeerEndpoint,
    metadata: bool,
    locks: &ReplicaLocks,
) -> Result<bool> {
    if metadata {
        let lock = {
            let mut locks = locks.lock().await;
            ensure!(
                locks.len() < 64 || locks.contains_key(&endpoint.fingerprint),
                "paired session capacity reached"
            );
            locks
                .entry(endpoint.fingerprint.clone())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(PeerSession::default())))
                .clone()
        };
        let mut state = lock.lock().await;
        state.last_used = std::time::Instant::now();
        let expected_base = if endpoint.host.contains(':') {
            format!("https://[{}]:{}", endpoint.host, endpoint.port)
        } else {
            format!("https://{}:{}", endpoint.host, endpoint.port)
        };
        if state
            .client
            .as_ref()
            .is_some_and(|(base, _)| *base != expected_base)
        {
            state.client = None;
        }
        for attempt in 0..2 {
            if state.client.is_none() {
                state.client = Some(
                    authenticated_client(
                        global,
                        &credentials.fingerprint,
                        &credentials.private_key,
                        &endpoint.fingerprint,
                        &endpoint.host,
                        endpoint.port,
                    )
                    .await?,
                );
            }
            let (base, client) = state
                .client
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("missing authenticated client"))?
                .clone();
            let result = metadata_pass(global, library, notify, &mut state, &base, &client).await;
            if result.as_ref().err().is_some_and(metadata_unauthorized) {
                state.client = None;
                if attempt == 0 {
                    continue;
                }
            }
            return result;
        }
        unreachable!()
    } else {
        let (base, client) = authenticated_client(
            global,
            &credentials.fingerprint,
            &credentials.private_key,
            &endpoint.fingerprint,
            &endpoint.host,
            endpoint.port,
        )
        .await?;
        let peer = http_request::sync::MetadataPeer::<RuneModel>::new(&base, client);
        let remote = peer.identity().await?;
        let local = library.0.read().await.identity().await?;
        ensure!(
            local.namespace == remote.namespace,
            "paired catalog namespace mismatch"
        );
        let report = super::sync::downloads::download_requested_audio_until(
            (*global.main_db).clone(),
            &global.recommend_db,
            global.fsio.clone(),
            (&*global.lib_path).into(),
            global.node_id.parse()?,
            &peer,
            notify.clone(),
            tokio::time::Instant::now() + Duration::from_secs(60),
        )
        .await?;
        Ok(!report.deadline_reached)
    }
}

fn metadata_unauthorized(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<sync::protocol::SyncError>()
            .is_some_and(|error| matches!(error, sync::protocol::SyncError::Unauthorized))
            || cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.status() == Some(reqwest::StatusCode::UNAUTHORIZED))
    })
}

async fn metadata_pass(
    global: &Arc<GlobalParams>,
    library: &SharedLibrary,
    notify: &Arc<dyn Fn() + Send + Sync>,
    state: &mut PeerSession,
    base: &str,
    client: &reqwest::Client,
) -> Result<bool> {
    let peer = http_request::sync::MetadataPeer::<RuneModel>::new(base, client.clone());
    let remote = peer.identity().await?;
    let local = library.0.read().await.identity().await?;
    ensure!(
        local.namespace == remote.namespace,
        "paired catalog namespace mismatch"
    );
    let identity = (remote.namespace, remote.replica, remote.generation);
    if state.identity != Some(identity) {
        state.session = None;
        state.artwork_cursor = http_request::sync::PageCursor::default();
        state.identity = Some(identity);
    }
    if state.session.is_none() {
        state.session = Some(super::sync::client::LibrarySession::new(
            library.0.read().await.clone(),
        )?);
    }
    let complete = state
        .session
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing metadata session"))?
        .advance(&peer, notify.clone())
        .await?;
    if complete {
        state.session = None;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let (attached, _) = super::sync::client::fetch_missing(
            &global.main_db,
            &library.0.read().await.record_store(),
            base,
            client,
            &mut state.artwork_cursor,
            deadline,
        )
        .await?;
        if attached > 0 {
            notify();
        }
        if state.artwork_cursor.is_complete() {
            state.artwork_cursor = http_request::sync::PageCursor::default();
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) async fn authenticated_client(
    global: &GlobalParams,
    own: &str,
    private_key: &str,
    fingerprint: &str,
    host: &str,
    peer_port: u16,
) -> Result<(String, reqwest::Client)> {
    let validator = global.cert_validator.read().await.clone();
    let trusted = validator.get_hosts_for_fingerprint(fingerprint).await;
    ensure!(
        trusted.iter().any(|candidate| endpoint_host_port(candidate)
            .is_some_and(|(trusted_host, trusted_port)| trusted_host == host
                && trusted_port.is_none_or(|port| port == peer_port))),
        "peer endpoint is not trusted"
    );
    let tls = tls_config(validator, fingerprint);
    let base = if host.contains(':') {
        format!("https://[{host}]:{peer_port}")
    } else {
        format!("https://{host}:{peer_port}")
    };
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let challenge: Challenge =
        bounded_json(client.get(format!("{base}/sync/challenge")).send().await?).await?;
    ensure!(
        challenge.server_fingerprint == fingerprint,
        "peer identity mismatch"
    );
    let private = RsaPrivateKey::from_pkcs8_pem(private_key)?;
    let digest = Sha256::digest(message(fingerprint, &challenge.nonce));
    let signature = private.sign_with_rng(&mut OsRng, Pss::new::<Sha256>(), &digest)?;
    let token: Token = bounded_json(
        client
            .post(format!("{base}/sync/authorize"))
            .json(&Authorization {
                nonce: challenge.nonce,
                fingerprint: own.into(),
                signature: URL_SAFE_NO_PAD.encode(signature),
            })
            .send()
            .await?,
    )
    .await?;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", token.token).parse()?,
    );
    let validator = global.cert_validator.read().await.clone();
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config(validator, fingerprint))
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    Ok((base, client))
}
pub(super) struct AudioCredentials {
    pub client: reqwest::Client,
    pub generation: u64,
}

/// Caller-owned, pinned request capability for one immutable content address.
#[derive(Clone)]
pub(super) struct RenewableAudio {
    pub global: Arc<GlobalParams>,
    pub own: String,
    pub private_key: String,
    pub endpoint: PeerEndpoint,
    pub url: String,
    pub size: u64,
    pub client: Arc<tokio::sync::Mutex<AudioCredentials>>,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl RenewableAudio {
    async fn check_authorization(&self) -> Result<()> {
        ensure!(!self.cancel.is_cancelled(), "audio access revoked");
        let status = self
            .global
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&self.endpoint.fingerprint)
            .await
            .map(|user| user.status);
        let trusted = self
            .global
            .cert_validator
            .read()
            .await
            .get_hosts_for_fingerprint(&self.endpoint.fingerprint)
            .await;
        if !outbound_eligible(status) || !trusted.contains(&self.endpoint.trusted_host) {
            self.cancel.cancel();
            anyhow::bail!("audio access revoked");
        }
        Ok(())
    }
    pub(super) async fn refresh(&self) -> Result<()> {
        let mut client = tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"),
            client = self.client.lock() => client,
        };
        self.refresh_locked(&mut client).await
    }
    async fn refresh_locked(&self, credentials: &mut AudioCredentials) -> Result<()> {
        self.check_authorization().await?;
        let result = tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"),
            result = authenticated_client(&self.global, &self.own, &self.private_key, &self.endpoint.fingerprint, &self.endpoint.host, self.endpoint.port) => result,
        };
        match result {
            Ok((_, renewed)) => {
                credentials.client = renewed;
                credentials.generation = credentials.generation.wrapping_add(1);
                Ok(())
            }
            Err(error) => {
                if error.chain().any(|cause| {
                    cause
                        .downcast_ref::<reqwest::Error>()
                        .is_some_and(|e| e.status() == Some(reqwest::StatusCode::UNAUTHORIZED))
                }) {
                    self.cancel.cancel();
                }
                Err(error)
            }
        }
    }
    async fn request(&self, range: Option<(u64, Option<u64>)>) -> Result<reqwest::Response> {
        self.check_authorization().await?;
        let (client, generation) = {
            let credentials = tokio::select! {
                _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"),
                credentials = self.client.lock() => credentials,
            };
            (credentials.client.clone(), credentials.generation)
        };
        let send = |client: reqwest::Client| {
            let mut request = client.get(&self.url);
            if let Some((start, end)) = range {
                request = request.header(
                    reqwest::header::RANGE,
                    format!(
                        "bytes={start}-{}",
                        end.map(|v| v.to_string()).unwrap_or_default()
                    ),
                );
            }
            request.send()
        };
        let response = tokio::select! { _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"), result = send(client.clone()) => result? };
        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let renewed = {
                let mut credentials = tokio::select! {
                    _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"),
                    credentials = self.client.lock() => credentials,
                };
                if credentials.generation == generation {
                    self.refresh_locked(&mut credentials).await?;
                }
                credentials.client.clone()
            };
            tokio::select! { _ = self.cancel.cancelled() => anyhow::bail!("audio access revoked"), result = send(renewed) => result? }
        } else {
            response
        };
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.cancel.cancel();
            anyhow::bail!("audio authorization denied after renewal");
        }
        validate_audio_response(&response, self.size, range)?;
        Ok(response)
    }
}

fn validate_audio_response(
    response: &reqwest::Response,
    size: u64,
    range: Option<(u64, Option<u64>)>,
) -> Result<()> {
    let expected_length = if let Some((start, end)) = range {
        ensure!(start < size, "audio range starts beyond content");
        let last = end.unwrap_or(size - 1).min(size - 1);
        ensure!(last >= start, "invalid audio range");
        ensure!(
            response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            "peer did not honor audio range"
        );
        let expected = format!("bytes {start}-{last}/{size}");
        ensure!(
            response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
                == Some(expected.as_str()),
            "invalid audio content range"
        );
        last - start + 1
    } else {
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "audio request failed: {}",
            response.status()
        );
        size
    };
    ensure!(
        response.content_length() == Some(expected_length),
        "invalid audio response length"
    );
    Ok(())
}

impl playback::player::AudioRequest for RenewableAudio {
    fn get(
        &self,
        range: Option<(u64, Option<u64>)>,
    ) -> futures::future::BoxFuture<'static, Result<reqwest::Response>> {
        let this = self.clone();
        Box::pin(async move { this.request(range).await })
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use database::sync::model::RuneModel;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn newly_due_peer_runs_while_another_is_stalled() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let available = Arc::new(AtomicBool::new(false));
        let started = Arc::new(tokio::sync::Notify::new());
        let second = Arc::new(tokio::sync::Notify::new());
        let cancel = tokio_util::sync::CancellationToken::new();
        let worker = tokio::spawn({
            let available = available.clone();
            let started = started.clone();
            let second = second.clone();
            let cancel = cancel.clone();
            async move {
                schedule_peers(
                    || {
                        let mut peers = vec![String::from("first")];
                        if available.load(Ordering::SeqCst) {
                            peers.push(String::from("second"));
                        }
                        async move { peers }
                    },
                    |peer| {
                        let started = started.clone();
                        let second = second.clone();
                        async move {
                            if peer == "first" {
                                started.notify_one();
                                std::future::pending::<()>().await;
                            }
                            second.notify_one();
                            (peer, Duration::from_secs(30))
                        }
                    },
                    |peer: &String| peer.clone(),
                    4,
                    cancel,
                    || async {},
                )
                .await;
            }
        });
        started.notified().await;
        available.store(true, Ordering::SeqCst);
        let result = tokio::time::timeout(Duration::from_secs(2), second.notified()).await;
        cancel.cancel();
        worker.await.unwrap();
        assert!(
            result.is_ok(),
            "second peer did not start while first was stalled"
        );
    }

    #[tokio::test]
    async fn metadata_unauthorized_is_distinct_from_other_failures() {
        let app = axum::Router::new().route(
            "/sync/metadata",
            axum::routing::post(|| async { axum::http::StatusCode::UNAUTHORIZED }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let peer =
            http_request::sync::MetadataPeer::<RuneModel>::new(&base, reqwest::Client::new());
        let error = anyhow::Error::new(peer.identity().await.unwrap_err());
        assert!(metadata_unauthorized(&error));
        assert!(!metadata_unauthorized(&anyhow::anyhow!("network failure")));
        server.abort();
    }

    #[test]
    fn explicit_and_advertised_endpoint_hosts_preserve_ports() {
        assert_eq!(
            endpoint_host_port("127.0.0.1:4321"),
            Some(("127.0.0.1".into(), Some(4321)))
        );
        assert_eq!(
            endpoint_host_port("[::1]:5432"),
            Some(("::1".into(), Some(5432)))
        );
        assert_eq!(endpoint_host_port("::1"), Some(("::1".into(), None)));
        assert_eq!(
            endpoint_host_port("peer.example:7654"),
            Some(("peer.example".into(), Some(7654)))
        );
        assert_eq!(endpoint_host_port("peer.example:0"), None);
    }

    #[tokio::test]
    async fn audio_range_requires_expected_status_and_content_range() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata",
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-3/8\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata",
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-7/8\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 1024];
                let _ = socket.read(&mut request).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for should_accept in [false, false, true] {
            let response = client
                .get(format!("http://{address}/audio"))
                .send()
                .await
                .unwrap();
            assert_eq!(
                validate_audio_response(&response, 8, Some((4, Some(7)))).is_ok(),
                should_accept
            );
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn blocked_audio_does_not_stop_independent_metadata_worker() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let audio_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let metadata_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let audio_addr = audio_listener.local_addr().unwrap();
        let metadata_addr = metadata_listener.local_addr().unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let audio_started = Arc::new(tokio::sync::Notify::new());
        let audio_server = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                let (_stream, _) = audio_listener.accept().await.unwrap();
                cancel.cancelled().await;
            }
        });
        let metadata_server = tokio::spawn(async move {
            let (mut stream, _) = metadata_listener.accept().await.unwrap();
            stream.write_all(b"ok").await.unwrap();
        });
        let (done_tx, done_rx) = oneshot::channel();
        let audio = {
            let cancel = cancel.clone();
            let audio_started = audio_started.clone();
            async move {
                let mut stream = tokio::net::TcpStream::connect(audio_addr).await.unwrap();
                audio_started.notify_one();
                let mut byte = [0];
                tokio::select! { _ = cancel.cancelled() => {}, _ = stream.read_exact(&mut byte) => panic!("audio stream should remain stalled") }
            }
        };
        let metadata = {
            let audio_started = audio_started.clone();
            async move {
                audio_started.notified().await;
                let mut stream = tokio::net::TcpStream::connect(metadata_addr).await.unwrap();
                let mut bytes = [0; 2];
                stream.read_exact(&mut bytes).await.unwrap();
                done_tx.send(bytes).unwrap();
            }
        };
        let workers = tokio::spawn(join_workers(metadata, audio));
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), done_rx)
                .await
                .unwrap()
                .unwrap(),
            *b"ok"
        );
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(2), workers)
            .await
            .unwrap()
            .unwrap();
        audio_server.await.unwrap();
        metadata_server.await.unwrap();
    }

    #[test]
    fn outbound_pair_permissions() {
        assert!(outbound_eligible(None));
        assert!(outbound_eligible(Some(UserStatus::Approved)));
        assert!(!outbound_eligible(Some(UserStatus::Blocked)));
        assert!(!outbound_eligible(Some(UserStatus::Pending)));
    }
}

async fn bounded_json<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let mut response = response.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= 8192usize.saturating_sub(bytes.len()),
            "authentication response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes)?)
}

#[derive(Debug)]
struct PinnedPeer {
    validator: Arc<discovery::client::CertValidator>,
    fingerprint: String,
}
impl ServerCertVerifier for PinnedPeer {
    fn verify_server_cert(
        &self,
        cert: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        name: &rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        let (_, parsed) = x509_parser::parse_x509_certificate(cert.as_ref())
            .map_err(|_| rustls::Error::General("invalid peer certificate".into()))?;
        let actual = discovery::ssl::calculate_base85_fingerprint(parsed.public_key().raw)
            .map_err(|_| rustls::Error::General("invalid peer key".into()))?;
        if actual != self.fingerprint {
            return Err(rustls::Error::General("peer fingerprint mismatch".into()));
        }
        let _ = (intermediates, name, ocsp, now);
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.validator
            .verify_tls12_signature(message, cert, signature)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.validator
            .verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.validator.supported_verify_schemes()
    }
}
fn tls_config(
    validator: discovery::client::CertValidator,
    fingerprint: &str,
) -> rustls::ClientConfig {
    rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedPeer {
            validator: Arc::new(validator),
            fingerprint: fingerprint.into(),
        }))
        .with_no_client_auth()
}

/// Resolves an unmaterialized catalog track at playback time using paired peers.
pub struct PairedAudioResolver(pub std::sync::Weak<super::ServerManager>);
impl playback::player::PlaybackSourceResolver for PairedAudioResolver {
    fn resolve_id(&self, id: i32) -> futures::future::BoxFuture<'static, Result<Option<i32>>> {
        let manager = self.0.clone();
        Box::pin(async move {
            let manager = manager
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("library closed"))?;
            let params = &manager.global_params;
            resolve_local_id(params.main_db.as_ref(), id).await
        })
    }
    fn local_path(
        &self,
        id: i32,
    ) -> futures::future::BoxFuture<'static, Result<Option<std::path::PathBuf>>> {
        Box::pin(resolve_materialized_path(self.0.clone(), id))
    }
    fn requires_stream(&self, id: i32) -> futures::future::BoxFuture<'static, Result<bool>> {
        let manager = self.0.clone();
        Box::pin(async move { Ok(resolve_materialized_path(manager, id).await?.is_none()) })
    }
    fn resolve(
        &self,
        id: i32,
    ) -> futures::future::BoxFuture<'static, Result<playback::player::RemoteAudioSource>> {
        let manager = self.0.clone();
        Box::pin(async move {
            let manager = manager
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("library closed"))?;
            manager.resolve_remote_audio(id).await
        })
    }
}

async fn resolve_materialized_path(
    manager: std::sync::Weak<super::ServerManager>,
    id: i32,
) -> Result<Option<std::path::PathBuf>> {
    let manager = manager
        .upgrade()
        .ok_or_else(|| anyhow::anyhow!("library closed"))?;
    let params = &manager.global_params;
    materialized_path(
        params.main_db.as_ref(),
        &params.fsio,
        std::path::Path::new(params.lib_path.as_str()),
        id,
    )
    .await
}
