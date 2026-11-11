use super::libraries::ListedLibrary;
use super::sync::client::LibrarySession;
use super::sync::client::SessionDirection;
use super::sync::client::fetch_missing;
use super::sync::downloads::{DownloadAudioContext, download_requested_audio_until};
use super::sync::{library_base, library_notification};
use super::sync_auth::{Authorization, Challenge, TOKEN_TTL, Token, message};
#[cfg(test)]
use super::utils::device::load_device_info;
use crate::access_policy::{Action, ConnectionLevel, SyncLevel};
use crate::messages::{LibrarySyncStatusUpdated, ShareSyncFailure};
#[cfg(test)]
use crate::utils::GlobalParams;
use crate::utils::subsonic_binding_for_playback;
use crate::{library_pool::LibraryHandle, node::NodeContext};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use database::actions::file::{ManagedAudioVerifier, file_ids_by_uuid, materialized_path};
use database::actions::library::clean_local_orphans;
use database::credentials::CredentialStore;
use database::library_registry::{
    CatalogPlacement, FsAdapter, LibraryAccess, LibraryConnectionAccess, LibraryRecord,
    LibraryRecordId, LibraryRegistry, ProtocolAdapter, RuneForwardMode,
};
use database::sync::Maintenance;
use database::sync::model::RuneModel;
use database::sync::share::{ShareCapabilities, SharePullError, SharePushError};
use database::sync::{LibrarySyncStore, replica::PendingShareChange};
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use discovery::endpoint::endpoint_host_port;
use discovery::server::UserStatus;
use fsio::{OpenDalConfig, OpenDalObjectStore};
use futures::future::BoxFuture;
use futures::future::{AbortHandle, Abortable, Aborted};
use http_request::sync::PageCursor;
use playback::player::{
    LibraryPlaybackSource, PlaybackSourceResolver, RemoteAudioError, RemoteAudioSource,
    SubsonicItem,
};
use rand::rngs::OsRng;
use reqwest::header::HeaderMap;
use reqwest::redirect::Policy;
use rsa::{Pss, RsaPrivateKey, pkcs8::DecodePrivateKey};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time as std_time;
use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    future::Future,
    hash::Hash,
    path::Path,
};
use std::{
    sync::{Arc, Mutex as StdMutex, Weak},
    time::Duration,
};
use sync::SyncPeer;
use sync::protocol::SyncError;
use sync::share::{ObjectStore, StoreError};
use tokio::time as tokio_time;
use tokio::time::{Instant, MissedTickBehavior};
use tokio::{sync::Mutex, task::JoinSet};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const DEFAULT_PEER_PORT: u16 = 7863;
const PEER_CONCURRENCY: usize = 4;
const PEER_RETRY_START: Duration = Duration::from_secs(1);
const PEER_RETRY_CAP: Duration = Duration::from_secs(30);
const TOKEN_SAFETY_MARGIN: Duration = Duration::from_secs(30);
const SHARE_PULL_INTERVAL: Duration = Duration::from_secs(60);
const SHARE_BACKGROUND_PULL_INTERVAL: Duration = Duration::from_secs(15 * 60);
const SHARE_PUSH_DEBOUNCE: Duration = Duration::from_secs(10);
const SHARE_STATISTICS_INTERVAL: Duration = Duration::from_secs(30 * 60);
const SHARE_GC_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const SHARE_RETRY_START: Duration = Duration::from_secs(1);
const SHARE_RETRY_CAP: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct ShareStoreSession {
    store: Arc<dyn ObjectStore>,
    capabilities: ShareCapabilities,
}

trait ShareStoreFactory: Send + Sync {
    fn open<'a>(
        &'a self,
        record: &'a LibraryRecord,
        prefix: &'a str,
        credentials: &'a CredentialStore,
    ) -> futures::future::BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>>;
}

struct OpenDalShareStoreFactory;

impl ShareStoreFactory for OpenDalShareStoreFactory {
    fn open<'a>(
        &'a self,
        record: &'a LibraryRecord,
        prefix: &'a str,
        credentials: &'a CredentialStore,
    ) -> futures::future::BoxFuture<'a, Result<ShareStoreSession, ShareSyncFailure>> {
        Box::pin(async move {
            let config = match record.fs.as_slice() {
                [
                    FsAdapter::WebDav {
                        endpoint,
                        root,
                        username,
                        credential,
                    },
                ] => {
                    let credential = credentials
                        .get(*credential)
                        .await
                        .map_err(|_| ShareSyncFailure::CredentialsRejected)?;
                    if credential.username != *username {
                        return Err(ShareSyncFailure::CredentialsRejected);
                    }
                    let password = String::from_utf8(credential.secret.as_bytes().to_vec())
                        .map_err(|_| ShareSyncFailure::CredentialsRejected)?;
                    OpenDalConfig::WebDav {
                        endpoint: endpoint.clone(),
                        root: root.clone(),
                        username: username.clone(),
                        password,
                    }
                }
                [
                    FsAdapter::S3 {
                        endpoint,
                        bucket,
                        region,
                        root,
                        access_key_id,
                        credential,
                    },
                ] => {
                    let credential = credentials
                        .get(*credential)
                        .await
                        .map_err(|_| ShareSyncFailure::CredentialsRejected)?;
                    if credential.username != *access_key_id {
                        return Err(ShareSyncFailure::CredentialsRejected);
                    }
                    let secret_access_key =
                        String::from_utf8(credential.secret.as_bytes().to_vec())
                            .map_err(|_| ShareSyncFailure::CredentialsRejected)?;
                    OpenDalConfig::S3 {
                        endpoint: endpoint.clone(),
                        bucket: bucket.clone(),
                        region: region.clone(),
                        root: root.clone(),
                        access_key_id: access_key_id.clone(),
                        secret_access_key,
                    }
                }
                _ => return Err(ShareSyncFailure::ProbeFailed),
            };
            let store =
                OpenDalObjectStore::new(config).map_err(|_| ShareSyncFailure::ProbeFailed)?;
            let probed = store
                .probe(prefix)
                .await
                .map_err(classify_share_store_error)?;
            Ok(ShareStoreSession {
                store: Arc::new(store),
                capabilities: ShareCapabilities {
                    conditional_create: probed.conditional_create,
                    conditional_head_write: probed.conditional_put,
                },
            })
        })
    }
}

async fn share_sync_pass(
    library_sync: &LibrarySyncStore,
    library_id: Uuid,
    cache_path: PathBuf,
    session: &ShareStoreSession,
    pass: SharePass,
    notify: impl Fn(),
) -> Result<bool, ShareSyncFailure> {
    let pulled = library_sync
        .pull_share(
            session.store.clone(),
            library_id,
            cache_path.clone(),
            session.capabilities,
        )
        .await
        .map_err(share_pull_failure)?;
    if !pulled.is_empty() {
        notify();
    }
    if pass == SharePass::PullAndPush {
        library_sync
            .push_share(
                session.store.clone(),
                library_id,
                cache_path,
                session.capabilities,
            )
            .await
            .map_err(share_push_failure)?;
    }
    Ok(!pulled.is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SharePass {
    PullOnly,
    PullAndPush,
}

struct ShareSchedule {
    last_pull: Option<Instant>,
    observed_head: Option<u64>,
    last_change: Option<Instant>,
    last_push: Instant,
    pending_change: Option<PendingShareChange>,
}

impl Default for ShareSchedule {
    fn default() -> Self {
        Self {
            last_pull: None,
            observed_head: None,
            last_change: None,
            last_push: Instant::now(),
            pending_change: None,
        }
    }
}

async fn select_share_cancellation<T>(
    cancel: CancellationToken,
    operation: impl Future<Output = T>,
) -> Option<T> {
    tokio::select! {
        _ = cancel.cancelled() => None,
        result = operation => Some(result),
    }
}

fn next_share_backoff(previous: Option<Duration>) -> (Duration, Duration) {
    let wait = previous.unwrap_or(SHARE_RETRY_START);
    (wait, wait.saturating_mul(2).min(SHARE_RETRY_CAP))
}

fn classify_share_store_error(error: StoreError) -> ShareSyncFailure {
    match error {
        StoreError::PermissionDenied => ShareSyncFailure::ReadOnlyShare,
        StoreError::Unauthorized => ShareSyncFailure::CredentialsRejected,
        StoreError::NotFound => ShareSyncFailure::ObjectMissing,
        StoreError::Transient(_) => ShareSyncFailure::StoreUnavailable,
        StoreError::Permanent(_) | StoreError::ClockSkew | StoreError::PreconditionFailed => {
            ShareSyncFailure::ProbeFailed
        }
    }
}

/// A peer endpoint: its fingerprint and base URL.
type PeerKey = (String, String);

#[derive(Clone, Default)]
pub struct SyncTokenStore {
    tokens: Arc<StdMutex<HashMap<PeerKey, StoredSyncToken>>>,
    authorization_locks: Arc<StdMutex<HashMap<PeerKey, Arc<Mutex<()>>>>>,
}

struct StoredSyncToken {
    value: String,
    obtained_at: std_time::Instant,
}

impl SyncTokenStore {
    #[cfg(test)]
    pub(crate) fn insert_stale_for_test(&self, fingerprint: &str, base: &str) {
        self.insert(fingerprint, base, "unknown-to-peer".to_owned());
    }

    /// Serializes authorization per peer endpoint, so concurrent passes against one
    /// peer share a single challenge round trip while other peers are not blocked.
    fn authorization_lock(&self, fingerprint: &str, base: &str) -> Arc<Mutex<()>> {
        let mut locks = self
            .authorization_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        locks
            .entry((fingerprint.to_owned(), base.to_owned()))
            .or_default()
            .clone()
    }

    fn get(&self, fingerprint: &str, base: &str) -> Option<String> {
        let mut tokens = self.tokens.lock().ok()?;
        tokens.retain(|_, token| token.obtained_at.elapsed() < TOKEN_TTL);
        let token = tokens.get(&(fingerprint.to_owned(), base.to_owned()))?;
        (token.obtained_at.elapsed() < TOKEN_TTL.saturating_sub(TOKEN_SAFETY_MARGIN))
            .then(|| token.value.clone())
    }

    fn insert(&self, fingerprint: &str, base: &str, value: String) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.retain(|_, token| token.obtained_at.elapsed() < TOKEN_TTL);
            tokens.insert(
                (fingerprint.to_owned(), base.to_owned()),
                StoredSyncToken {
                    value,
                    obtained_at: std_time::Instant::now(),
                },
            );
        }
    }

    pub(crate) fn invalidate(&self, fingerprint: &str, base: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(&(fingerprint.to_owned(), base.to_owned()));
        }
    }

    pub(crate) fn remove_peer(&self, fingerprint: &str) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.retain(|(peer, _), _| peer != fingerprint);
        }
        if let Ok(mut locks) = self.authorization_locks.lock() {
            locks.retain(|(peer, _), _| peer != fingerprint);
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PeerEndpoint {
    pub fingerprint: String,
    pub trusted_host: String,
    pub host: String,
    pub port: u16,
}

pub(crate) async fn trusted_fingerprint(
    validator: &discovery::client::CertValidator,
    host: &str,
    port: u16,
) -> Result<String> {
    let entries = validator.clone_trusted_entries().await;
    let mut matches = entries.into_iter().filter_map(|(fingerprint, hosts)| {
        hosts
            .iter()
            .any(|candidate| {
                endpoint_host_port(candidate).is_some_and(|(name, trusted_port)| {
                    name == host && trusted_port.is_none_or(|p| p == port)
                })
            })
            .then_some(fingerprint)
    });
    let fingerprint = matches
        .next()
        .ok_or_else(|| anyhow::anyhow!("peer endpoint is not trusted"))?;
    ensure!(
        matches.next().is_none(),
        "peer endpoint has multiple trusted identities"
    );
    Ok(fingerprint)
}

pub(super) async fn peer_endpoints(global: &NodeContext, own: &str) -> Vec<Vec<PeerEndpoint>> {
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

type PeerLibraryKey = (String, Uuid);
type ReplicaLocks = Arc<Mutex<HashMap<PeerLibraryKey, Arc<Mutex<PeerSession>>>>>;

struct PeerSession {
    session: Option<LibrarySession>,
    library: Option<LibraryHandle>,
    client: Option<(String, reqwest::Client)>,
    identity: Option<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    artwork_cursor: http_request::sync::PageCursor,
    last_used: std::time::Instant,
    advertised_level: Option<SyncLevel>,
    push_forbidden: bool,
    direction: SessionDirection,
    read_only_logged: bool,
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
            library: None,
            client: None,
            identity: None,
            artwork_cursor: PageCursor::default(),
            last_used: std_time::Instant::now(),
            advertised_level: None,
            push_forbidden: false,
            direction: SessionDirection::Both,
            read_only_logged: false,
        }
    }
}

pub async fn run(
    global: Weak<NodeContext>,
    notify: Arc<dyn Fn() + Send + Sync>,
    own_fingerprint: String,
    private_key: String,
    cancel: CancellationToken,
) {
    let locks: ReplicaLocks = Arc::default();
    let credentials = LocalCredentials {
        fingerprint: own_fingerprint,
        private_key,
    };
    let metadata = run_loop(
        global.clone(),
        notify.clone(),
        credentials.clone(),
        cancel.clone(),
        true,
        locks.clone(),
    );
    let audio = run_loop(
        global.clone(),
        notify.clone(),
        credentials,
        cancel.clone(),
        false,
        locks,
    );
    tokio::join!(
        join_workers(metadata, audio),
        verify_managed_audio(global.clone(), notify.clone(), cancel.clone()),
        maintain_metadata(global.clone(), cancel.clone()),
        run_share_worker(global, notify, cancel, Arc::new(OpenDalShareStoreFactory))
    );
}

async fn run_share_worker(
    global: Weak<NodeContext>,
    notify: Arc<dyn Fn() + Send + Sync>,
    cancel: CancellationToken,
    factory: Arc<dyn ShareStoreFactory>,
) {
    let mut sessions = HashMap::<Uuid, ShareStoreSession>::new();
    let mut retry = HashMap::<Uuid, (Duration, Instant)>::new();
    let mut schedules = HashMap::<Uuid, ShareSchedule>::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                flush_share_worker(&global, &sessions, &notify).await;
                return;
            },
            _ = interval.tick() => (),
        }
        let Some(global) = global.upgrade() else {
            return;
        };
        let ids = match global.library_pool.served_library_ids().await {
            Ok(ids) => ids,
            Err(error) => {
                log::warn!("Could not discover share libraries: {error:#}");
                continue;
            }
        };
        sessions.retain(|id, _| ids.contains(id));
        retry.retain(|id, _| ids.contains(id));
        schedules.retain(|id, _| ids.contains(id));
        for id in ids {
            if retry.get(&id).is_some_and(|(_, due)| *due > Instant::now()) {
                continue;
            }
            let operation = async {
                let Some(library) = global
                    .library_pool
                    .acquire_served(id)
                    .await
                    .map_err(|_| ShareSyncFailure::StoreUnavailable)?
                else {
                    sessions.remove(&id);
                    retry.remove(&id);
                    schedules.remove(&id);
                    return Ok::<_, ShareSyncFailure>(false);
                };
                if !matches!(library.descriptor.catalog, CatalogPlacement::InShare(_)) {
                    sessions.remove(&id);
                    retry.remove(&id);
                    schedules.remove(&id);
                    return Ok(false);
                }
                let cache_path = PathBuf::from(global.config_path.as_str())
                    .join("share-cache")
                    .join(id.to_string());
                let prefix = library
                    .library_sync
                    .share_prefix(cache_path.clone())
                    .await
                    .map_err(|_| ShareSyncFailure::ProbeFailed)?;
                if let Entry::Vacant(entry) = sessions.entry(id) {
                    let session = factory
                        .open(&library.descriptor, &prefix, &global.credentials)
                        .await?;
                    entry.insert(session);
                }
                let session = sessions
                    .get(&id)
                    .cloned()
                    .ok_or(ShareSyncFailure::ProbeFailed)?;
                let pending = library
                    .library_sync
                    .share_pending_changes(cache_path.clone())
                    .await
                    .map_err(|_| ShareSyncFailure::StoreUnavailable)?;
                let schedule = schedules.entry(id).or_default();
                let now = Instant::now();
                if let Some(pending) = pending {
                    if schedule.observed_head != Some(pending.journal_head) {
                        schedule.observed_head = Some(pending.journal_head);
                        schedule.last_change = Some(now);
                    }
                    schedule.pending_change = Some(pending.change);
                }
                let foreground = global
                    .current_ui()
                    .await
                    .ok()
                    .and_then(|params| params.library_id)
                    == Some(id);
                let poll_interval = if foreground {
                    SHARE_PULL_INTERVAL
                } else {
                    SHARE_BACKGROUND_PULL_INTERVAL
                };
                let pull_due = schedule
                    .last_pull
                    .is_none_or(|last| last.elapsed() >= poll_interval);
                let push_due = schedule.last_pull.is_none()
                    || schedule.last_push.elapsed() >= SHARE_GC_INTERVAL
                    || pending.is_some_and(|_| match schedule.pending_change {
                        Some(PendingShareChange::StatisticsOnly) => {
                            schedule.last_push.elapsed() >= SHARE_STATISTICS_INTERVAL
                        }
                        Some(PendingShareChange::Content) => schedule
                            .last_change
                            .is_some_and(|last| last.elapsed() >= SHARE_PUSH_DEBOUNCE),
                        None => false,
                    });
                if !pull_due && !push_due {
                    return Ok(false);
                }
                let pass_notify = pass_notification(&global, &library, &notify).await;
                let pulled = share_sync_pass(
                    &library.library_sync,
                    library.library_id,
                    cache_path,
                    &session,
                    if push_due {
                        SharePass::PullAndPush
                    } else {
                        SharePass::PullOnly
                    },
                    || pass_notify(),
                )
                .await?;
                schedule.last_pull = Some(now);
                if push_due {
                    schedule.last_push = now;
                }
                Ok(pulled || push_due)
            };
            let Some(result) = select_share_cancellation(cancel.clone(), operation).await else {
                return;
            };
            match result {
                Ok(did_work) => {
                    retry.remove(&id);
                    if did_work {
                        global.gui_broadcaster.broadcast(&LibrarySyncStatusUpdated {
                            library_id: id.to_string(),
                            share_failure: None,
                        });
                    }
                }
                Err(failure) => {
                    let previous = retry.get(&id).map(|(delay, _)| *delay);
                    let (wait, next_delay) = next_share_backoff(previous);
                    retry.insert(id, (next_delay, Instant::now() + wait));
                    global.gui_broadcaster.broadcast(&LibrarySyncStatusUpdated {
                        library_id: id.to_string(),
                        share_failure: Some(failure),
                    });
                }
            }
        }
    }
}

async fn flush_share_worker(
    global: &Weak<NodeContext>,
    sessions: &HashMap<Uuid, ShareStoreSession>,
    notify: &Arc<dyn Fn() + Send + Sync>,
) {
    let Some(global) = global.upgrade() else {
        return;
    };
    let Ok(ids) = global.library_pool.served_library_ids().await else {
        return;
    };
    for id in ids {
        let Some(session) = sessions.get(&id) else {
            continue;
        };
        let Ok(Some(library)) = global.library_pool.acquire_served(id).await else {
            continue;
        };
        if !matches!(library.descriptor.catalog, CatalogPlacement::InShare(_)) {
            continue;
        }
        let cache_path = PathBuf::from(global.config_path.as_str())
            .join("share-cache")
            .join(id.to_string());
        let pass_notify = pass_notification(&global, &library, notify).await;
        if let Err(error) = share_sync_pass(
            &library.library_sync,
            library.library_id,
            cache_path,
            session,
            SharePass::PullAndPush,
            || pass_notify(),
        )
        .await
        {
            log::warn!("Final share publication failed: {error:?}");
        }
    }
}

fn share_pull_failure(error: SharePullError) -> ShareSyncFailure {
    match error {
        SharePullError::ShareChanged => ShareSyncFailure::ShareChanged,
        SharePullError::ReadOnly => ShareSyncFailure::ReadOnlyShare,
        SharePullError::CredentialsRejected => ShareSyncFailure::CredentialsRejected,
        SharePullError::MissingObject(_) => ShareSyncFailure::ObjectMissing,
        SharePullError::Transient(_) | SharePullError::Store(_) => {
            ShareSyncFailure::StoreUnavailable
        }
        SharePullError::Invalid(_) | SharePullError::Cancelled => ShareSyncFailure::ProbeFailed,
    }
}

fn share_push_failure(error: SharePushError) -> ShareSyncFailure {
    match error {
        SharePushError::ShareChanged => ShareSyncFailure::ShareChanged,
        SharePushError::ReadOnly => ShareSyncFailure::ReadOnlyShare,
        SharePushError::CredentialsRejected => ShareSyncFailure::CredentialsRejected,
        SharePushError::MissingObject => ShareSyncFailure::ObjectMissing,
        SharePushError::Failed(_) | SharePushError::NeedsBase | SharePushError::StaleHead => {
            ShareSyncFailure::StoreUnavailable
        }
    }
}

#[cfg(test)]
#[path = "sync_worker_tests.rs"]
mod share_worker_tests;

async fn eligible_libraries(global: &NodeContext) -> Vec<Uuid> {
    match global.library_pool.served_library_ids().await {
        Ok(ids) => ids,
        Err(error) => {
            log::warn!("Could not load background libraries: {error:#}");
            Vec::new()
        }
    }
}

async fn maintain_metadata(global: Weak<NodeContext>, cancel: tokio_util::sync::CancellationToken) {
    let mut maintenance = HashMap::<Uuid, Maintenance>::new();
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = cancel.cancelled() => return, _ = interval.tick() => () }
        let Some(global) = global.upgrade() else {
            return;
        };
        let ids = eligible_libraries(&global).await;
        maintenance.retain(|id, _| ids.contains(id));
        for id in ids {
            let library = tokio::select! {
                _ = cancel.cancelled() => return,
                library = global.library_pool.acquire_served_background(id) => library,
            };
            let library = match library {
                Ok(Some(library)) => library,
                Ok(None) => continue,
                Err(error) => {
                    log::warn!("Could not acquire library for maintenance: {error:#}");
                    continue;
                }
            };
            let state = maintenance.entry(id).or_default();
            for _ in 0..32 {
                let result = tokio::select! {
                    _ = cancel.cancelled() => return,
                    result = library.library_sync.maintain(state) => result,
                };
                if let Err(error) = result {
                    log::warn!("Metadata maintenance failed: {error:#}");
                    break;
                }
                if let Err(error) = clean_local_orphans(&library.main_db).await {
                    log::warn!("Local metadata cleanup failed: {error:#}");
                    break;
                }
                tokio::task::yield_now().await;
            }
        }
    }
}

async fn pass_notification(
    global: &NodeContext,
    library: &LibraryHandle,
    ui_notify: &Arc<dyn Fn() + Send + Sync>,
) -> Arc<dyn Fn() + Send + Sync> {
    if global
        .current_ui()
        .await
        .ok()
        .and_then(|params| params.library_id)
        == Some(library.library_id)
    {
        ui_notify.clone()
    } else {
        library_notification(global, &library.display_name.clone())
    }
}

async fn verify_managed_audio(
    global: Weak<NodeContext>,
    notify: Arc<dyn Fn() + Send + Sync>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut verifiers = HashMap::<Uuid, ManagedAudioVerifier>::new();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = cancel.cancelled() => return, _ = interval.tick() => {} }
        let Some(global) = global.upgrade() else {
            return;
        };
        let ids = eligible_libraries(&global).await;
        verifiers.retain(|id, _| ids.contains(id));
        for id in ids {
            let result = async {
                let Some(library) = global.library_pool.acquire_served_background(id).await? else {
                    return Ok(());
                };
                let step = verifiers
                    .entry(id)
                    .or_default()
                    .step(
                        &library.main_db,
                        &library.fsio,
                        Path::new(&library.media_root),
                        4 * 1024 * 1024,
                    )
                    .await?;
                if step.invalidated {
                    (pass_notification(&global, &library, &notify).await)();
                }
                Ok::<_, anyhow::Error>(())
            };
            let result = tokio::select! {
                _ = cancel.cancelled() => return,
                result = result => result,
            };
            if let Err(error) = result {
                log::warn!("Managed audio verification failed: {error:#}");
            }
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
    global: Weak<NodeContext>,
    notify: Arc<dyn Fn() + Send + Sync>,
    credentials: LocalCredentials,
    cancel: tokio_util::sync::CancellationToken,
    metadata: bool,
    locks: ReplicaLocks,
) {
    let discover = || async {
        let Some(global) = global.upgrade() else {
            return Vec::new();
        };
        let ids = eligible_libraries(&global).await;
        let peers = peer_endpoints(&global, &credentials.fingerprint).await;
        ids.into_iter()
            .flat_map(|id| peers.iter().cloned().map(move |hosts| (id, hosts)))
            .collect()
    };
    let work = |(id, hosts): (Uuid, Vec<PeerEndpoint>)| {
        let global = global.clone();
        let notify = notify.clone();
        let credentials = credentials.clone();
        let cancel = cancel.clone();
        let locks = locks.clone();
        async move {
            let key = (hosts[0].fingerprint.clone(), id);
            let Some(global) = global.upgrade() else {
                return (key, Duration::from_secs(30), false);
            };
            let library_cancel = global.sync_library_token(id).await;
            let library = tokio::select! {
                _ = cancel.cancelled() => return (key, Duration::from_secs(30), false),
                _ = library_cancel.cancelled() => return (key, Duration::from_secs(30), false),
                library = global.library_pool.acquire_served_background(id) => library,
            };
            let library = match library {
                Ok(Some(library)) => library,
                Ok(None) => return (key, Duration::from_secs(30), true),
                Err(error) => {
                    log::warn!("Could not acquire background library: {error:#}");
                    return (key, Duration::from_secs(30), true);
                }
            };
            let notify = pass_notification(&global, &library, &notify).await;
            for endpoint in hosts {
                let result = tokio::select! {
                    _ = cancel.cancelled() => return (key, Duration::from_secs(30), false),
                    _ = library_cancel.cancelled() => {
                        locks.lock().await.remove(&(endpoint.fingerprint.clone(), id));
                        return (key, Duration::from_secs(30), false);
                    },
                    _ = wait_for_revocation(Arc::downgrade(&global), &endpoint.fingerprint, &endpoint.trusted_host) => Err(anyhow::anyhow!("peer permission revoked")),
                    result = async {
                        let options = PassOptions { credentials: &credentials, metadata, locks: &locks, cancel: &cancel };
                        let operation = sync_one(&global, &library, &notify, &endpoint, options);
                        if metadata {
                            tokio::time::timeout(Duration::from_secs(60), operation).await.map_err(|_| anyhow::anyhow!("peer operation timed out"))?
                        } else {
                            operation.await
                        }
                    } => result,
                };
                match classify_peer_pass(result) {
                    Ok(complete) => return (key, complete.retry_delay(), false),
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
            (key, Duration::from_secs(30), true)
        }
    };
    schedule_peers(
        discover,
        work,
        |(id, hosts)| (hosts[0].fingerprint.clone(), *id),
        if metadata { PEER_CONCURRENCY } else { 1 },
        cancel.clone(),
        || async {
            if metadata {
                let Some(global) = global.upgrade() else {
                    return;
                };
                let ids = eligible_libraries(&global).await;
                locks.lock().await.retain(|(_, id), entry| {
                    ids.contains(id)
                        && (Arc::strong_count(entry) > 1
                            || entry.try_lock().map_or(true, |state| {
                                state.last_used.elapsed() < Duration::from_secs(900)
                            }))
                });
            }
        },
    )
    .await;
}

type PeerWorkResult<Q> = (Q, Duration, bool);
type PeerTaskResult<Q> = (Q, Result<PeerWorkResult<Q>, Aborted>);

async fn schedule_peers<P, Q, D, DF, W, WF, K, C, CF>(
    discover: D,
    work: W,
    key: K,
    limit: usize,
    cancel: tokio_util::sync::CancellationToken,
    cleanup: C,
) where
    D: Fn() -> DF,
    DF: Future<Output = Vec<P>>,
    W: Fn(P) -> WF,
    WF: Future<Output = PeerWorkResult<Q>> + Send + 'static,
    K: Fn(&P) -> Q,
    Q: Clone + Eq + Hash + Send + 'static,
    C: Fn() -> CF,
    CF: Future<Output = ()>,
{
    let mut due = HashMap::<Q, Instant>::new();
    let mut backoff = HashMap::<Q, PeerFailureBackoff>::new();
    let mut running = HashMap::<Q, AbortHandle>::new();
    // Discovery and cleanup may await locks held by an in-flight peer pass.
    let mut tasks: JoinSet<PeerTaskResult<Q>> = JoinSet::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut next_due = None;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tasks.shutdown().await;
                return;
            },
            _ = tick.tick() => {},
            _ = async { match next_due { Some(deadline) => tokio::time::sleep_until(deadline).await, None => std::future::pending().await } } => {},
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                let (key, result) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        log::error!("Background peer task failed: {error}");
                        tasks.shutdown().await;
                        return;
                    }
                };
                running.remove(&key);
                if let Ok((key, wait, failed)) = result {
                    let policy = backoff.entry(key.clone()).or_default();
                    let wait = if failed {
                        policy.next_failure_delay()
                    } else {
                        policy.reset();
                        wait
                    };
                    due.insert(key, Instant::now() + wait);
                }
            }
        }
        let peers = tokio::select! {
            _ = cancel.cancelled() => {
                tasks.shutdown().await;
                return;
            },
            peers = discover() => peers,
        };
        let eligible: HashSet<_> = peers.iter().map(&key).collect();
        due.retain(|fingerprint, _| eligible.contains(fingerprint));
        backoff.retain(|fingerprint, _| eligible.contains(fingerprint));
        for (key, handle) in &running {
            if !eligible.contains(key) {
                handle.abort();
            }
        }
        for peer in peers {
            if tasks.len() >= limit {
                break;
            }
            let fingerprint = key(&peer);
            if !running.contains_key(&fingerprint)
                && due
                    .get(&fingerprint)
                    .is_none_or(|deadline| *deadline <= Instant::now())
            {
                let (handle, registration) = AbortHandle::new_pair();
                running.insert(fingerprint.clone(), handle);
                let future = work(peer);
                tasks.spawn(
                    async move { (fingerprint, Abortable::new(future, registration).await) },
                );
            }
        }
        next_due = if tasks.len() < limit {
            due.iter()
                .filter(|(fingerprint, deadline)| {
                    eligible.contains(*fingerprint)
                        && !running.contains_key(*fingerprint)
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

#[derive(Default)]
struct PeerFailureBackoff {
    next_delay: Option<Duration>,
}

impl PeerFailureBackoff {
    fn next_failure_delay(&mut self) -> Duration {
        let delay = self.next_delay.unwrap_or(PEER_RETRY_START);
        self.next_delay = Some(delay.saturating_mul(2).min(PEER_RETRY_CAP));
        delay
    }

    fn reset(&mut self) {
        self.next_delay = None;
    }
}

fn outbound_eligible(status: Option<UserStatus>) -> bool {
    matches!(status, Some(UserStatus::Approved))
}

/// Whether a peer may exchange `action`-direction changes of `library` with this node:
/// the library's authority always may; any other peer needs the matching grant here.
async fn peer_may_sync(
    global: &NodeContext,
    library: &LibraryHandle,
    fingerprint: &str,
    action: Action,
) -> bool {
    let is_authority = matches!(
        &library.descriptor.protocol,
        ProtocolAdapter::RuneForward { owner_fingerprint: authority, mode: RuneForwardMode::Replica { .. } } if authority == fingerprint
    );
    let peer = global
        .permission_manager
        .read()
        .await
        .verify_by_fingerprint(fingerprint)
        .await
        .map(|user| PeerIdentity {
            fingerprint: user.fingerprint,
            public_key: user.public_key,
        });
    is_authority || peer.is_some_and(|peer| global.access_policy.check(&peer, &action).is_ok())
}

pub(super) async fn wait_for_revocation(global: Weak<NodeContext>, fingerprint: &str, host: &str) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Some(global) = global.upgrade() else {
            return;
        };
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PeerPass {
    Complete,
    Incomplete,
    LibraryNotServed,
}
impl PeerPass {
    pub(super) fn retry_delay(self) -> Duration {
        if self == Self::Incomplete {
            Duration::from_millis(100)
        } else {
            Duration::from_secs(30)
        }
    }
}

fn classify_peer_pass(result: Result<bool>) -> Result<PeerPass> {
    match result {
        Ok(true) => Ok(PeerPass::Complete),
        Ok(false) => Ok(PeerPass::Incomplete),
        Err(error) if error.downcast_ref::<SyncError>() == Some(&SyncError::NamespaceNotServed) => {
            Ok(PeerPass::LibraryNotServed)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct TestPeerSession {
    locks: ReplicaLocks,
}

#[cfg(test)]
impl TestPeerSession {
    pub(super) async fn pass(
        &self,
        params: Arc<GlobalParams>,
        private_key: &str,
        endpoint: PeerEndpoint,
        metadata: bool,
    ) -> Result<PeerPass> {
        let global = params.node.clone();
        let credentials = LocalCredentials {
            fingerprint: load_device_info(Path::new(&*global.config_path))
                .await?
                .fingerprint,
            private_key: private_key.into(),
        };
        let library = global
            .library_pool
            .acquire_background(params.library_id.unwrap())
            .await?;
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
        classify_peer_pass(
            sync_one(
                &global,
                &library,
                &notify,
                &endpoint,
                PassOptions {
                    credentials: &credentials,
                    metadata,
                    locks: &self.locks,
                    cancel: &params.node.token,
                },
            )
            .await,
        )
    }
}

#[cfg(test)]
pub(super) async fn test_peer_pass(
    params: Arc<GlobalParams>,
    private_key: &str,
    endpoint: PeerEndpoint,
    metadata: bool,
) -> Result<PeerPass> {
    TestPeerSession::default()
        .pass(params, private_key, endpoint, metadata)
        .await
}

struct PassOptions<'a> {
    credentials: &'a LocalCredentials,
    metadata: bool,
    locks: &'a ReplicaLocks,
    cancel: &'a tokio_util::sync::CancellationToken,
}

async fn sync_one(
    global: &Arc<NodeContext>,
    library: &LibraryHandle,
    notify: &Arc<dyn Fn() + Send + Sync>,
    endpoint: &PeerEndpoint,
    options: PassOptions<'_>,
) -> Result<bool> {
    let PassOptions {
        credentials,
        metadata,
        locks,
        cancel,
    } = options;
    if metadata {
        let lock = {
            let mut locks = locks.lock().await;
            locks
                .entry((endpoint.fingerprint.clone(), library.library_id))
                .or_insert_with(|| Arc::new(Mutex::new(PeerSession::default())))
                .clone()
        };
        let mut state = lock.lock().await;
        state.last_used = std_time::Instant::now();
        let expected_base = peer_base(&endpoint.host, endpoint.port);
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
            let mut grant_changes = global.access_policy.changes();
            let user = global
                .permission_manager
                .read()
                .await
                .verify_by_fingerprint(&endpoint.fingerprint)
                .await;
            if user
                .as_ref()
                .is_some_and(|user| user.status != UserStatus::Approved)
            {
                return Ok(true);
            }
            let library_id = library.library_id;
            // Their changes are applied here only from the authority or a SyncPush grant;
            // ours are sent only to the authority or a peer granted SyncPull.
            let may_receive = peer_may_sync(
                global,
                library,
                &endpoint.fingerprint,
                Action::SyncPush { library_id },
            )
            .await;
            let may_send = peer_may_sync(
                global,
                library,
                &endpoint.fingerprint,
                Action::SyncPull { library_id },
            )
            .await;
            let target = MetadataTarget {
                global,
                fingerprint: &endpoint.fingerprint,
                may_receive,
                may_send,
                cancel,
            };
            let operation = metadata_pass(target, library, notify, &mut state, &base, &client);
            let result = tokio::select! {
                biased;
                _ = grant_changes.changed() => { state.session = None; state.library = None; return Ok(false); },
                result = operation => result,
            };
            if result.as_ref().err().is_some_and(metadata_unauthorized) {
                invalidate_sync_token(global, &endpoint.fingerprint, &endpoint.host, endpoint.port);
                state.client = None;
                if attempt == 0 {
                    continue;
                }
            }
            return result;
        }
        unreachable!()
    } else {
        let local = library.library_sync.identity().await?;
        let playback_queue = global.session_queue_for_library(library.library_id).await?;
        for attempt in 0..2 {
            let (base, client) = authenticated_client(
                global,
                &credentials.fingerprint,
                &credentials.private_key,
                &endpoint.fingerprint,
                &endpoint.host,
                endpoint.port,
            )
            .await?;
            let base = library_base(&base, local.namespace);
            let peer = http_request::sync::MetadataPeer::<RuneModel>::new(&base, client);
            let result = async {
                let remote = peer.identity().await?;
                ensure!(
                    local.namespace == remote.namespace,
                    "paired catalog namespace mismatch"
                );
                let report = download_requested_audio_until(
                    (*library.main_db).clone(),
                    &library.recommend_db,
                    library.fsio.clone(),
                    library.media_root.clone().into(),
                    DownloadAudioContext {
                        node_id: global.node_id.parse()?,
                        remote: &peer,
                        notify: notify.clone(),
                        playback_queue: &playback_queue,
                    },
                    tokio_time::Instant::now() + Duration::from_secs(60),
                )
                .await?;
                Ok(!report.deadline_reached)
            }
            .await;
            if result.as_ref().err().is_some_and(metadata_unauthorized) {
                invalidate_sync_token(global, &endpoint.fingerprint, &endpoint.host, endpoint.port);
                if attempt == 0 {
                    continue;
                }
            }
            return result;
        }
        unreachable!()
    }
}

fn metadata_unauthorized(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<sync::protocol::SyncError>()
            .is_some_and(|error| matches!(error, SyncError::Unauthorized))
            || cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|error| error.status() == Some(reqwest::StatusCode::UNAUTHORIZED))
    })
}

struct MetadataTarget<'a> {
    global: &'a NodeContext,
    fingerprint: &'a str,
    may_receive: bool,
    may_send: bool,
    cancel: &'a tokio_util::sync::CancellationToken,
}

async fn metadata_pass(
    target: MetadataTarget<'_>,
    library: &LibraryHandle,
    notify: &Arc<dyn Fn() + Send + Sync>,
    state: &mut PeerSession,
    base: &str,
    client: &reqwest::Client,
) -> Result<bool> {
    let MetadataTarget {
        global,
        fingerprint,
        may_receive,
        may_send,
        cancel,
    } = target;
    let local = library.library_sync.identity().await?;
    let listings: Vec<ListedLibrary> = client
        .get(format!("{base}/libraries"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let advertised = listings
        .iter()
        .find(|entry| entry.library_id == local.namespace)
        .map(|entry| entry.sync);
    let level = advertised
        // Hosted replicas are not advertised. Probe their authenticated routes;
        // a push denial below switches this peer to pull-only.
        .unwrap_or(SyncLevel::PullPush);
    if state.advertised_level != Some(level) {
        state.push_forbidden = false;
        state.advertised_level = Some(level);
    }
    if let Some(level) = advertised {
        let access = if state.push_forbidden {
            LibraryAccess::Read
        } else {
            match level {
                SyncLevel::None => LibraryAccess::NotShared,
                SyncLevel::Pull => LibraryAccess::Read,
                SyncLevel::PullPush => LibraryAccess::ReadWrite,
            }
        };
        if LibraryRegistry::set_access_level(
            Path::new(&*global.config_path),
            local.namespace,
            fingerprint,
            access,
        )
        .await?
        {
            notify();
        }
        let connection_access = match listings
            .iter()
            .find(|entry| entry.library_id == local.namespace)
            .map(|entry| entry.connection)
        {
            Some(ConnectionLevel::None) | None => LibraryConnectionAccess::None,
            Some(ConnectionLevel::View) => LibraryConnectionAccess::View,
            Some(ConnectionLevel::ViewEdit) => LibraryConnectionAccess::ViewEdit,
        };
        if LibraryRegistry::set_connection_access(
            Path::new(&*global.config_path),
            local.namespace,
            fingerprint,
            connection_access,
        )
        .await?
        {
            notify();
        }
    }
    if level == SyncLevel::None {
        state.session = None;
        state.library = None;
        return Err(SyncError::NamespaceNotServed.into());
    }
    let read_only = level == SyncLevel::Pull || state.push_forbidden || !may_send;
    if read_only && !state.read_only_logged {
        log::info!(
            "Library {} is read-only on peer; synchronization is pull-only",
            local.namespace
        );
        state.read_only_logged = true;
    }
    if !read_only {
        state.read_only_logged = false;
    }
    if !may_receive {
        log::info!(
            "Skipping changes from peer {fingerprint} for library {}: peer is not the authority and has no SyncPush grant",
            local.namespace
        );
    }
    if read_only && !may_receive {
        state.session = None;
        state.library = None;
        return Ok(true);
    }
    let direction = if read_only {
        SessionDirection::Pull
    } else if !may_receive {
        SessionDirection::Push
    } else {
        SessionDirection::Both
    };
    if direction != state.direction {
        state.session = None;
        state.direction = direction;
    }

    let base = library_base(base, local.namespace);
    let peer = http_request::sync::MetadataPeer::<RuneModel>::new(&base, client.clone());
    let remote = peer.identity().await?;
    ensure!(
        local.namespace == remote.namespace,
        "paired catalog namespace mismatch"
    );
    let identity = (remote.namespace, remote.replica, remote.generation);
    if state.identity != Some(identity) {
        state.session = None;
        state.artwork_cursor = PageCursor::default();
        state.identity = Some(identity);
    }
    if state.session.is_none() {
        let mut session = LibrarySession::new(library.library_sync.clone())?;
        session.set_direction(direction);
        state.session = Some(session);
        state.library = Some(library.clone());
    }
    let result = state
        .session
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("missing metadata session"))?
        .advance_cancellable(&peer, notify.clone(), cancel)
        .await;
    let complete = match result {
        Ok(complete) => complete,
        Err(error)
            if error
                .downcast_ref::<SyncError>()
                .is_some_and(|error| matches!(error, SyncError::Forbidden))
                && direction != SessionDirection::Pull =>
        {
            state.push_forbidden = true;
            if LibraryRegistry::set_access_level(
                Path::new(&*global.config_path),
                local.namespace,
                fingerprint,
                LibraryAccess::Read,
            )
            .await?
            {
                notify();
            }
            state.session = None;
            state.library = None;
            if !state.read_only_logged {
                log::info!(
                    "Library {} refused push; synchronization is pull-only",
                    local.namespace
                );
                state.read_only_logged = true;
            }
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    if complete {
        state.session = None;
        state.library = None;
        let deadline = tokio_time::Instant::now() + Duration::from_secs(10);
        let (attached, _) = fetch_missing(
            &library.main_db,
            &library.library_sync.record_store(),
            &base,
            client,
            &mut state.artwork_cursor,
            deadline,
        )
        .await?;
        if attached > 0 {
            notify();
        }
        if state.artwork_cursor.is_complete() {
            state.artwork_cursor = PageCursor::default();
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) async fn authenticated_client(
    global: &NodeContext,
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
    let base = peer_base(host, peer_port);
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .build()?;
    let token = obtain_or_reuse_token(
        &global.sync_tokens,
        &client,
        &base,
        own,
        private_key,
        fingerprint,
    )
    .await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {token}").parse()?,
    );
    let validator = global.cert_validator.read().await.clone();
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config(validator, fingerprint))
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .build()?;
    Ok((base, client))
}

/// Send one authenticated peer request, renewing a cached token once if the
/// peer no longer recognizes it.
pub(super) async fn send_authenticated_request<F>(
    global: &NodeContext,
    own: &str,
    private_key: &str,
    fingerprint: &str,
    host: &str,
    peer_port: u16,
    request: F,
) -> Result<reqwest::Response>
where
    F: Fn(&str, &reqwest::Client) -> reqwest::RequestBuilder,
{
    let (base, client) =
        authenticated_client(global, own, private_key, fingerprint, host, peer_port).await?;
    let response = request(&base, &client).send().await?;
    if response.status() != reqwest::StatusCode::UNAUTHORIZED {
        return Ok(response);
    }

    invalidate_sync_token(global, fingerprint, host, peer_port);
    let (base, client) =
        authenticated_client(global, own, private_key, fingerprint, host, peer_port).await?;
    Ok(request(&base, &client).send().await?)
}

pub(crate) async fn obtain_or_reuse_token(
    store: &SyncTokenStore,
    client: &reqwest::Client,
    base: &str,
    own: &str,
    private_key: &str,
    fingerprint: &str,
) -> Result<String> {
    let token = match store.get(fingerprint, base) {
        Some(token) => token,
        None => {
            let lock = store.authorization_lock(fingerprint, base);
            let _authorization = lock.lock().await;
            match store.get(fingerprint, base) {
                Some(token) => token,
                None => {
                    let token = obtain_token(client, base, own, private_key, fingerprint).await?;
                    store.insert(fingerprint, base, token.clone());
                    token
                }
            }
        }
    };
    Ok(token)
}

pub(crate) fn invalidate_sync_token(
    global: &NodeContext,
    fingerprint: &str,
    host: &str,
    port: u16,
) {
    global
        .sync_tokens
        .invalidate(fingerprint, &peer_base(host, port));
}

/// The base URL of a peer's HTTPS server, bracketing IPv6 hosts.
pub(crate) fn peer_base(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("https://[{host}]:{port}")
    } else {
        format!("https://{host}:{port}")
    }
}

pub(crate) async fn obtain_token(
    client: &reqwest::Client,
    base: &str,
    own: &str,
    private_key: &str,
    fingerprint: &str,
) -> Result<String> {
    let mut retries = 0;
    loop {
        match obtain_token_once(client, base, own, private_key, fingerprint).await {
            Ok(token) => return Ok(token),
            Err(error) if is_too_many_requests(&error) && retries < 2 => {
                retries += 1;
                tokio_time::sleep(Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

async fn obtain_token_once(
    client: &reqwest::Client,
    base: &str,
    own: &str,
    private_key: &str,
    fingerprint: &str,
) -> Result<String> {
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
    Ok(token.token)
}

fn is_too_many_requests(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<reqwest::Error>()
            .is_some_and(|error| error.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS))
    })
}

fn is_authentication_denied(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<reqwest::Error>().is_some_and(|error| {
            matches!(
                error.status(),
                Some(reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN)
            )
        })
    })
}
pub(super) struct AudioCredentials {
    pub client: reqwest::Client,
    pub generation: u64,
}

/// Caller-owned, pinned request capability for one immutable content address.
#[derive(Clone)]
pub(super) struct RenewableAudio {
    pub global: Weak<NodeContext>,
    pub own: String,
    pub private_key: String,
    pub endpoint: PeerEndpoint,
    pub url: String,
    pub access_url: String,
    pub size: u64,
    pub client: Arc<tokio::sync::Mutex<AudioCredentials>>,
    pub cancel: tokio_util::sync::CancellationToken,
}

impl RenewableAudio {
    async fn check_authorization(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            anyhow::bail!(RemoteAudioError::AccessRevoked);
        }
        let global = self.global.upgrade().context("Node has shut down")?;
        let status = global
            .permission_manager
            .read()
            .await
            .verify_by_fingerprint(&self.endpoint.fingerprint)
            .await
            .map(|user| user.status);
        let trusted = global
            .cert_validator
            .read()
            .await
            .get_hosts_for_fingerprint(&self.endpoint.fingerprint)
            .await;
        if !outbound_eligible(status) || !trusted.contains(&self.endpoint.trusted_host) {
            self.cancel.cancel();
            anyhow::bail!(RemoteAudioError::AccessRevoked);
        }
        Ok(())
    }

    pub(super) async fn check_library_access(&self) -> Result<()> {
        self.check_authorization().await?;
        let (client, generation) = {
            let credentials = tokio::select! {
                _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
                credentials = self.client.lock() => credentials,
            };
            (credentials.client.clone(), credentials.generation)
        };
        let response = tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
            response = client.head(&self.access_url).send() => response.map_err(RemoteAudioError::unreachable)?,
        };
        let response = if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED
                | reqwest::StatusCode::FORBIDDEN
                | reqwest::StatusCode::NOT_FOUND
        ) {
            self.refresh_if_current(generation).await?;
            let client = self.client.lock().await.client.clone();
            tokio::select! {
                _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
                response = client.head(&self.access_url).send() => response.map_err(RemoteAudioError::unreachable)?,
            }
        } else {
            response
        };
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED
                | reqwest::StatusCode::FORBIDDEN
                | reqwest::StatusCode::NOT_FOUND
        ) {
            self.cancel.cancel();
            anyhow::bail!(RemoteAudioError::AccessRevoked);
        }
        if response.status() != reqwest::StatusCode::OK {
            anyhow::bail!(RemoteAudioError::unreachable(format!(
                "unexpected status {}",
                response.status()
            )));
        }
        Ok(())
    }
    pub(super) async fn refresh_if_current(&self, observed_generation: u64) -> Result<()> {
        let mut client = tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
            client = self.client.lock() => client,
        };
        if client.generation != observed_generation {
            return Ok(());
        }
        self.refresh_locked(&mut client).await
    }
    async fn refresh_locked(&self, credentials: &mut AudioCredentials) -> Result<()> {
        self.check_authorization().await?;
        let global = self.global.upgrade().context("Node has shut down")?;
        invalidate_sync_token(
            &global,
            &self.endpoint.fingerprint,
            &self.endpoint.host,
            self.endpoint.port,
        );
        let result = tokio::select! {
            _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
            result = authenticated_client(&global, &self.own, &self.private_key, &self.endpoint.fingerprint, &self.endpoint.host, self.endpoint.port) => result,
        };
        let result = match result {
            Err(error) if is_authentication_denied(&error) => tokio::select! {
                _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
                result = authenticated_client(&global, &self.own, &self.private_key, &self.endpoint.fingerprint, &self.endpoint.host, self.endpoint.port) => result,
            },
            result => result,
        };
        match result {
            Ok((_, renewed)) => {
                credentials.client = renewed;
                credentials.generation = credentials.generation.wrapping_add(1);
                Ok(())
            }
            Err(error) => {
                if is_authentication_denied(&error) {
                    self.cancel.cancel();
                    anyhow::bail!(RemoteAudioError::AccessRevoked);
                }
                self.check_authorization()
                    .await
                    .context("audio authorization check failed during renewal")?;
                Err(error)
            }
        }
    }
    async fn request(&self, range: Option<(u64, Option<u64>)>) -> Result<reqwest::Response> {
        self.check_authorization().await?;
        let (client, generation) = {
            let credentials = tokio::select! {
                _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
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
        let response = tokio::select! { _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked), result = send(client.clone()) => result.map_err(RemoteAudioError::unreachable)? };
        let response = if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED
                | reqwest::StatusCode::FORBIDDEN
                | reqwest::StatusCode::NOT_FOUND
        ) {
            let renewed = {
                let mut credentials = tokio::select! {
                    _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked),
                    credentials = self.client.lock() => credentials,
                };
                if credentials.generation == generation {
                    self.refresh_locked(&mut credentials).await?;
                }
                credentials.client.clone()
            };
            tokio::select! { _ = self.cancel.cancelled() => anyhow::bail!(RemoteAudioError::AccessRevoked), result = send(renewed) => result.map_err(RemoteAudioError::unreachable)? }
        } else {
            response
        };
        if matches!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED
                | reqwest::StatusCode::FORBIDDEN
                | reqwest::StatusCode::NOT_FOUND
        ) {
            self.cancel.cancel();
            anyhow::bail!(RemoteAudioError::AccessRevoked);
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
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            anyhow::bail!(RemoteAudioError::unreachable(format!(
                "unexpected status {}",
                response.status()
            )));
        }
        let expected = format!("bytes {start}-{last}/{size}");
        if response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            != Some(expected.as_str())
        {
            anyhow::bail!(RemoteAudioError::unreachable(format!(
                "content range does not match {expected}"
            )));
        }
        last - start + 1
    } else {
        if response.status() != reqwest::StatusCode::OK {
            anyhow::bail!(RemoteAudioError::unreachable(format!(
                "unexpected status {}",
                response.status()
            )));
        }
        size
    };
    if response.content_length() != Some(expected_length) {
        anyhow::bail!(RemoteAudioError::Incomplete);
    }
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
    use axum::http::StatusCode;
    use database::sync::model::RuneModel;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::net::TcpListener;
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;
    use tokio::sync::{Notify, Semaphore};
    use tokio_util::sync::CancellationToken;

    #[derive(Clone, Copy)]
    enum ContentionPhase {
        Discovery,
        Cleanup,
    }

    struct SchedulerContention {
        pool: Mutex<()>,
        rounds: AtomicUsize,
        started: Semaphore,
        entered: Semaphore,
        release: Semaphore,
        finished: Semaphore,
    }

    impl Default for SchedulerContention {
        fn default() -> Self {
            Self {
                pool: Mutex::new(()),
                rounds: AtomicUsize::new(0),
                started: Semaphore::new(0),
                entered: Semaphore::new(0),
                release: Semaphore::new(0),
                finished: Semaphore::new(0),
            }
        }
    }

    async fn assert_peer_progress(phase: ContentionPhase) {
        let state = Arc::new(SchedulerContention::default());
        let cancel = CancellationToken::new();
        let worker = tokio::spawn({
            let state = state.clone();
            let cancel = cancel.clone();
            async move {
                schedule_peers(
                    || {
                        let state = state.clone();
                        async move {
                            if state.rounds.fetch_add(1, Ordering::SeqCst) > 0
                                && matches!(phase, ContentionPhase::Discovery)
                            {
                                state.entered.add_permits(1);
                                let _guard = state.pool.lock().await;
                            }
                            vec![0]
                        }
                    },
                    |peer| {
                        let state = state.clone();
                        async move {
                            let guard = state.pool.lock().await;
                            state.started.add_permits(2);
                            state.release.acquire().await.unwrap().forget();
                            drop(guard);
                            state.finished.add_permits(1);
                            (peer, Duration::from_secs(30), false)
                        }
                    },
                    |peer| *peer,
                    1,
                    cancel,
                    || {
                        let state = state.clone();
                        async move {
                            if matches!(phase, ContentionPhase::Cleanup) {
                                state.entered.add_permits(1);
                                state.started.acquire().await.unwrap().forget();
                                let _guard = state.pool.lock().await;
                            }
                        }
                    },
                )
                .await;
            }
        });
        let result = tokio_time::timeout(Duration::from_secs(3), async {
            state.started.acquire().await.unwrap().forget();
            state.entered.acquire().await.unwrap().forget();
            state.release.add_permits(1);
            state.finished.acquire().await.unwrap().forget();
        })
        .await;
        cancel.cancel();
        worker.abort();
        let _ = worker.await;
        assert!(
            result.is_ok(),
            "peer work stopped progressing during scheduler housekeeping"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn peer_progress_during_discovery() {
        assert_peer_progress(ContentionPhase::Discovery).await;
    }

    #[tokio::test(start_paused = true)]
    async fn peer_progress_during_cleanup() {
        assert_peer_progress(ContentionPhase::Cleanup).await;
    }

    #[test]
    fn peer_failure_backoff_doubles_caps_and_resets_after_success() {
        let mut backoff = PeerFailureBackoff::default();
        let delays = (0..7)
            .map(|_| backoff.next_failure_delay())
            .collect::<Vec<_>>();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30].map(Duration::from_secs));

        backoff.reset();
        assert_eq!(backoff.next_failure_delay(), PEER_RETRY_START);
    }

    #[tokio::test]
    async fn newly_due_peer_runs_while_another_is_stalled() {
        use std::sync::atomic::AtomicBool;
        let available = Arc::new(AtomicBool::new(false));
        let started = Arc::new(Notify::new());
        let second = Arc::new(Notify::new());
        let cancel = CancellationToken::new();
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
                            (peer, Duration::from_secs(30), false)
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
            axum::routing::post(|| async { StatusCode::UNAUTHORIZED }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let ready = reqwest::get(format!("{base}/sync/metadata")).await.unwrap();
        assert_eq!(ready.status(), StatusCode::METHOD_NOT_ALLOWED);
        let peer =
            http_request::sync::MetadataPeer::<RuneModel>::new(&base, reqwest::Client::new());
        let error = anyhow::Error::new(peer.identity().await.unwrap_err());
        assert!(metadata_unauthorized(&error), "{error:#}");
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
        use axum::{
            Router,
            body::Body,
            http::{
                HeaderValue,
                header::{CONTENT_LENGTH, CONTENT_RANGE},
            },
            response::Response,
            routing::get,
        };

        fn response(status: StatusCode, content_range: Option<&'static str>) -> Response {
            let mut response = Response::new(Body::from("data"));
            *response.status_mut() = status;
            response
                .headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_static("4"));
            if let Some(content_range) = content_range {
                response
                    .headers_mut()
                    .insert(CONTENT_RANGE, HeaderValue::from_static(content_range));
            }
            response
        }

        let app = Router::new()
            .route(
                "/wrong-status",
                get(|| async { response(StatusCode::OK, None) }),
            )
            .route(
                "/wrong-range",
                get(|| async { response(StatusCode::PARTIAL_CONTENT, Some("bytes 0-3/8")) }),
            )
            .route(
                "/valid-range",
                get(|| async { response(StatusCode::PARTIAL_CONTENT, Some("bytes 4-7/8")) }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for (path, should_accept) in [
            ("/wrong-status", false),
            ("/wrong-range", false),
            ("/valid-range", true),
        ] {
            let response = client
                .get(format!("http://{address}{path}"))
                .send()
                .await
                .unwrap();
            let accepted = validate_audio_response(&response, 8, Some((4, Some(7)))).is_ok();
            let body = response.bytes().await.unwrap();
            assert_eq!(body.as_ref(), b"data");
            assert_eq!(accepted, should_accept);
        }
        server.abort();
    }

    #[tokio::test]
    async fn blocked_audio_does_not_stop_independent_metadata_worker() {
        use tokio::io::AsyncReadExt;
        let audio_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let audio_addr = audio_listener.local_addr().unwrap();
        let cancel = CancellationToken::new();
        let audio_started = Arc::new(Notify::new());
        let audio_server = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                let (_stream, _) = audio_listener.accept().await.unwrap();
                cancel.cancelled().await;
            }
        });
        let (done_tx, done_rx) = oneshot::channel();
        let audio = {
            let cancel = cancel.clone();
            let audio_started = audio_started.clone();
            async move {
                let mut stream = TcpStream::connect(audio_addr).await.unwrap();
                audio_started.notify_one();
                let mut byte = [0];
                tokio::select! { _ = cancel.cancelled() => {}, _ = stream.read_exact(&mut byte) => panic!("audio stream should remain stalled") }
            }
        };
        let metadata = {
            let audio_started = audio_started.clone();
            async move {
                audio_started.notified().await;
                done_tx.send(*b"ok").unwrap();
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
    }

    #[test]
    fn outbound_pair_permissions() {
        assert!(!outbound_eligible(None));
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
pub(crate) fn tls_config(
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
pub struct PairedAudioResolver(pub Weak<NodeContext>, pub String);
impl PlaybackSourceResolver for PairedAudioResolver {
    fn resolve_track(
        &self,
        library_id: String,
        track: String,
    ) -> BoxFuture<'static, Result<playback::player::LibraryTrackKey>> {
        let node = self.0.clone();
        Box::pin(async move {
            let owner = node.upgrade().context("Node stopped")?;
            let library_id = library_id.parse()?;
            let registry = LibraryRegistry::load(Path::new(&*owner.config_path))?;
            let record = registry
                .record(LibraryRecordId(library_id))
                .or_else(|_| registry.catalog(library_id))
                .map_err(|_| anyhow::Error::new(RemoteAudioError::LibraryUnavailable))?;
            if matches!(&record.protocol, ProtocolAdapter::SubsonicTranslate { .. }) {
                Ok(playback::player::LibraryTrackKey::Subsonic(track))
            } else {
                Ok(playback::player::LibraryTrackKey::Rune(
                    Uuid::parse_str(&track).context("Invalid Rune library track UUID")?,
                ))
            }
        })
    }

    fn for_library(
        &self,
        library_id: String,
    ) -> BoxFuture<'static, Result<Option<LibraryPlaybackSource>>> {
        let node = self.0.clone();
        Box::pin(async move {
            let owner = node.upgrade().context("Node stopped")?;
            let library = owner
                .library_pool
                .acquire(library_id.parse()?)
                .await
                .map_err(|_| anyhow::anyhow!(RemoteAudioError::LibraryUnavailable))?;
            Ok(Some(LibraryPlaybackSource {
                fsio: library.fsio.clone(),
                resolver: Arc::new(PairedAudioResolver(node, library_id)),
            }))
        })
    }
    fn resolve_id(&self, file_uuid: Uuid) -> BoxFuture<'static, Result<Option<i32>>> {
        let node = self.0.clone();
        let library_id = self.1.clone();
        Box::pin(async move {
            let owner = node.upgrade().context("Node stopped")?;
            let library = owner
                .library_pool
                .acquire(library_id.parse()?)
                .await
                .map_err(|_| anyhow::anyhow!(RemoteAudioError::LibraryUnavailable))?;
            let ids = file_ids_by_uuid(library.main_db.as_ref(), &[file_uuid]).await?;
            Ok(ids.get(&file_uuid).copied())
        })
    }
    fn local_path(&self, id: i32) -> BoxFuture<'static, Result<Option<PathBuf>>> {
        Box::pin(resolve_materialized_path(
            self.0.clone(),
            self.1.clone(),
            id,
        ))
    }
    fn requires_stream(&self, id: i32) -> BoxFuture<'static, Result<bool>> {
        let node = self.0.clone();
        let library_id = self.1.clone();
        Box::pin(async move {
            Ok(resolve_materialized_path(node, library_id, id)
                .await?
                .is_none())
        })
    }
    fn resolve(&self, id: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        let node = self.0.clone();
        let library_id = self.1.clone();
        Box::pin(async move {
            let owner = node.upgrade().context("Node stopped")?;
            let library = owner
                .library_pool
                .acquire(library_id.parse()?)
                .await
                .map_err(|_| anyhow::anyhow!(RemoteAudioError::LibraryUnavailable))?;
            owner
                .manager()?
                .resolve_remote_audio(&library, owner.token.child_token(), id)
                .await
        })
    }

    fn resolve_subsonic(
        &self,
        item: SubsonicItem,
    ) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        let node = self.0.clone();
        Box::pin(async move {
            let owner = node.upgrade().context("Node stopped")?;
            let binding = subsonic_binding_for_playback(&owner, item.library_id).await?;
            binding.playback_source(item).await
        })
    }
}

async fn resolve_materialized_path(
    node: Weak<NodeContext>,
    library_id: String,
    id: i32,
) -> Result<Option<PathBuf>> {
    let owner = node.upgrade().context("Node stopped")?;
    let library = owner.library_pool.acquire(library_id.parse()?).await?;
    materialized_path(
        library.main_db.as_ref(),
        &library.fsio,
        Path::new(&library.media_root),
        id,
    )
    .await
}
