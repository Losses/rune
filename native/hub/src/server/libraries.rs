use crate::access_policy::{Action, ConnectionLevel, LibraryGrants, SyncLevel};
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use discovery::client::CertValidator;
use discovery::endpoint::{DEFAULT_DEVICE_PORT, DeviceAddress, DeviceScheme, device_url};
use std::path::Path;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use axum::{Extension, Json, http::StatusCode};
use database::library_registry::{
    LibraryConnectionAccess, LibraryRecord, LibraryRegistry, ProtocolAdapter, RuneForwardMode,
    create_replica,
};
use discovery::server::UserStatus;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::sync_worker::{PeerEndpoint, peer_endpoints, send_authenticated_request, tls_config};
use super::{ServerManager, generate_or_load_certificates};
use crate::backends::remote::select_renderer_host;
use crate::backends::remote::{RemoteConnectError, token_error};
use crate::handlers::neighbors::select_pinned_pair_host;
use crate::library_pool::LibraryHandle;
use crate::messages::{
    CastLibraryState, ClientSummary, DeviceSyncLevel, PeerLibrary, RegisterLibraryFailureKind,
    RendererAvailability, RendererCandidate,
};
use crate::node::NodeContext;
use crate::request_error::ExpectedRequestError;
use crate::server::api::{
    check_fingerprint, fetch_device_info, fetch_device_info_at_port, reported_status,
};
use tokio::task;
use tokio::time as tokio_time;
use url::Url;

const PAIRED_SERVER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Serialize, Deserialize)]
pub(super) struct ListedLibrary {
    pub(super) library_id: Uuid,
    pub(super) alias: String,
    pub(super) connection: ConnectionLevel,
    pub(super) sync: SyncLevel,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PeerAccess {
    control: bool,
    connection: LibraryGrants<ConnectionLevel>,
    sync: LibraryGrants<SyncLevel>,
    administer: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CastPreflightBody {
    library_ids: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HttpCastLibraryStatus {
    library_id: String,
    state: CastLibraryState,
}

pub(super) async fn peer_access(
    Extension(manager): Extension<Arc<ServerManager>>,
    Extension(peer): Extension<PeerIdentity>,
) -> Result<Json<PeerAccess>, StatusCode> {
    let node = manager
        .node()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut grants = node
        .access_policy
        .grants(&peer.fingerprint)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    grants
        .connection
        .libraries
        .retain(|library_id, _| node.access_policy.library_visible(&peer, *library_id));
    grants
        .sync
        .libraries
        .retain(|library_id, _| node.access_policy.library_visible(&peer, *library_id));
    Ok(Json(PeerAccess {
        control: grants.control,
        connection: grants.connection,
        sync: grants.sync,
        administer: grants.administer,
    }))
}

pub(super) async fn cast_preflight(
    Extension(manager): Extension<Arc<ServerManager>>,
    Extension(peer): Extension<PeerIdentity>,
    Json(request): Json<CastPreflightBody>,
) -> Result<Json<Vec<HttpCastLibraryStatus>>, StatusCode> {
    let node = manager
        .node()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    node.access_policy
        .check(&peer, &Action::Control)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let library_ids =
        parse_library_ids(&request.library_ids).map_err(|_| StatusCode::BAD_REQUEST)?;
    let libraries = cast_library_states(&node, &library_ids)
        .await
        .into_iter()
        .map(|(library_id, state)| HttpCastLibraryStatus {
            library_id: library_id.to_string(),
            state,
        })
        .collect();
    Ok(Json(libraries))
}

/// Parse library ids sent by a controller; a malformed id is `InvalidInput`.
pub(crate) fn parse_library_ids(library_ids: &[String]) -> Result<Vec<Uuid>> {
    library_ids
        .iter()
        .map(|library_id| {
            Uuid::parse_str(library_id).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("Invalid library id: {error}")).into()
            })
        })
        .collect()
}

/// The cast state of each distinct library, in the order first named.
pub(crate) async fn cast_library_states(
    node: &NodeContext,
    library_ids: &[Uuid],
) -> Vec<(Uuid, CastLibraryState)> {
    let mut seen = HashSet::new();
    let mut states = Vec::new();
    for library_id in library_ids {
        if seen.insert(*library_id) {
            states.push((*library_id, cast_library_state(node, *library_id).await));
        }
    }
    states
}

pub(crate) async fn list_renderer_candidates(
    node: Arc<NodeContext>,
    library_ids: Vec<Uuid>,
) -> Result<Vec<RendererCandidate>> {
    let own = node.device_info().await?;
    let (_, _, private_key) =
        generate_or_load_certificates(Path::new(&*node.config_path), &own.alias).await?;
    let endpoints = peer_endpoints(&node, &own.fingerprint).await;
    let mut peers = BTreeMap::<String, Vec<PeerEndpoint>>::new();
    for endpoint in endpoints.into_iter().flatten() {
        peers
            .entry(endpoint.fingerprint.clone())
            .or_default()
            .push(endpoint);
    }
    let mut probes = task::JoinSet::<Result<Option<RendererCandidate>>>::new();
    for (fingerprint, endpoints) in peers {
        let node = node.clone();
        let private_key = private_key.clone();
        let own_fingerprint = own.fingerprint.clone();
        let library_ids = library_ids.clone();
        probes.spawn(async move {
            let fallback = RendererCandidate {
                fingerprint: fingerprint.clone(),
                alias: fingerprint.clone(),
                attached: node
                    .renderer_link()
                    .ok()
                    .flatten()
                    .is_some_and(|link| link.fingerprint() == fingerprint),
                availability: RendererAvailability::Offline,
                blocked_library_id: None,
                library_state: None,
            };
            match tokio_time::timeout(
                Duration::from_secs(3),
                probe_renderer(
                    node,
                    own_fingerprint,
                    private_key,
                    fingerprint,
                    endpoints,
                    library_ids,
                    fallback.clone(),
                ),
            )
            .await
            {
                Ok(Ok(candidate)) => Ok(candidate),
                Ok(Err(error)) => {
                    log::debug!("Renderer candidate probe failed: {error:#}");
                    Ok(Some(fallback))
                }
                Err(_) => Ok(Some(fallback)),
            }
        });
    }
    let mut renderers = Vec::new();
    while let Some(result) = probes.join_next().await {
        match result {
            Ok(Ok(Some(candidate))) => renderers.push(candidate),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => log::debug!("Renderer candidate probe failed: {error:#}"),
            Err(error) => log::debug!("Renderer candidate task failed: {error}"),
        }
    }
    renderers.sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(renderers)
}

/// The devices this device trusts as servers, each with the status it reports for this device.
///
/// A peer this device refuses is left out. So is a peer whose probe fails or times out: its
/// approval cannot be confirmed, and an unreachable peer cannot be browsed anyway.
pub(crate) async fn list_paired_servers(node: &NodeContext) -> Result<Vec<ClientSummary>> {
    let own = node.device_info().await?.fingerprint;
    let validator = node.cert_validator.read().await.clone();
    let trusted = validator.clone_trusted_entries().await;
    let mut probes = task::JoinSet::<Option<ClientSummary>>::new();
    for (fingerprint, hosts) in trusted {
        if fingerprint == own || hosts.is_empty() || refused_here(node, &fingerprint).await {
            continue;
        }
        let validator = validator.clone();
        let own_fingerprint = own.clone();
        probes.spawn(async move {
            match tokio_time::timeout(
                PAIRED_SERVER_PROBE_TIMEOUT,
                probe_paired_server(validator, &own_fingerprint, fingerprint.clone(), hosts),
            )
            .await
            {
                Ok(Ok(server)) => Some(server),
                Ok(Err(error)) => {
                    log::debug!("Paired server probe failed for {fingerprint}: {error:#}");
                    None
                }
                Err(_) => {
                    log::debug!("Paired server probe timed out for {fingerprint}");
                    None
                }
            }
        });
    }
    let mut servers = Vec::new();
    while let Some(result) = probes.join_next().await {
        match result {
            Ok(Some(server)) => servers.push(server),
            Ok(None) => {}
            Err(error) => log::debug!("Paired server probe task failed: {error}"),
        }
    }
    servers.sort_by(|left, right| {
        left.alias
            .cmp(&right.alias)
            .then_with(|| left.fingerprint.cmp(&right.fingerprint))
    });
    Ok(servers)
}

async fn probe_paired_server(
    validator: CertValidator,
    own_fingerprint: &str,
    fingerprint: String,
    hosts: Vec<String>,
) -> Result<ClientSummary> {
    let host = select_pinned_pair_host(hosts, &validator, &fingerprint).await?;
    let config = Arc::new(tls_config(validator, &fingerprint));
    let info = fetch_device_info(&host, Arc::clone(&config)).await?;
    let checked = check_fingerprint(&host, config, own_fingerprint).await?;
    Ok(ClientSummary {
        alias: info.alias,
        fingerprint,
        device_model: info.device_model.unwrap_or_default(),
        status: reported_status(&checked),
    })
}

async fn probe_renderer(
    node: Arc<NodeContext>,
    own_fingerprint: String,
    private_key: String,
    fingerprint: String,
    endpoints: Vec<PeerEndpoint>,
    library_ids: Vec<Uuid>,
    mut candidate: RendererCandidate,
) -> Result<Option<RendererCandidate>> {
    let hosts = endpoints
        .iter()
        .map(|endpoint| {
            device_url(&endpoint.host, endpoint.port, DeviceScheme::Https, "/")
                .map(|url| url.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selected = select_renderer_host(&node, hosts).await?;
    let url = Url::parse(&selected)?;
    let address = DeviceAddress::new(
        url.host_str().context("Invalid peer host")?,
        url.port().unwrap_or(DEFAULT_DEVICE_PORT),
    )?;
    let host = address.host();
    let port = address.port();
    let validator = node.cert_validator.read().await.clone();
    let info = fetch_device_info_at_port(
        host,
        port,
        Arc::new(Arc::new(validator.clone()).into_client_config()),
    )
    .await?;
    if info
        .capabilities
        .as_ref()
        .and_then(|caps| caps.renderer.as_ref())
        .is_none()
    {
        return Ok(None);
    }
    candidate.alias = info.alias;
    let access = match send_authenticated_request(
        &node,
        &own_fingerprint,
        &private_key,
        &fingerprint,
        host,
        port,
        |base, client| client.get(format!("{base}/access")),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            candidate.availability = if is_auth_refusal(&error) {
                RendererAvailability::NotApproved
            } else {
                RendererAvailability::Offline
            };
            return Ok(Some(candidate));
        }
    };
    let access = match access {
        response
            if response.status() == StatusCode::UNAUTHORIZED
                || response.status() == StatusCode::FORBIDDEN =>
        {
            candidate.availability = RendererAvailability::NotApproved;
            return Ok(Some(candidate));
        }
        response if response.status().is_success() => response.json::<PeerAccess>().await,
        _ => {
            candidate.availability = RendererAvailability::Offline;
            return Ok(Some(candidate));
        }
    };
    let access = match access {
        Ok(access) => access,
        Err(_) => {
            candidate.availability = RendererAvailability::Offline;
            return Ok(Some(candidate));
        }
    };
    if !access.control {
        candidate.availability = RendererAvailability::NoControl;
        return Ok(Some(candidate));
    }
    // No other device holds a library kept on this one, so its id is not sent to ask.
    if let Some(kept) = library_ids
        .iter()
        .find(|library_id| node.access_policy.is_device_only(**library_id))
    {
        candidate.availability = RendererAvailability::CantPlay;
        candidate.blocked_library_id = Some(kept.to_string());
        candidate.library_state = Some(CastLibraryState::NotHeld);
        return Ok(Some(candidate));
    }
    if !library_ids.is_empty() {
        let raw_ids = library_ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
        let response = match send_authenticated_request(
            &node,
            &own_fingerprint,
            &private_key,
            &fingerprint,
            host,
            port,
            |base, client| {
                client
                    .post(format!("{base}/cast/preflight"))
                    .json(&serde_json::json!({ "libraryIds": raw_ids }))
            },
        )
        .await
        {
            Ok(response) if response.status().is_success() => response,
            _ => {
                candidate.availability = RendererAvailability::Offline;
                return Ok(Some(candidate));
            }
        };
        let states: Vec<HttpCastLibraryStatus> = response.json().await?;
        if let Some(blocked) = states
            .into_iter()
            .find(|status| status.state != CastLibraryState::Ready)
        {
            candidate.availability = RendererAvailability::CantPlay;
            candidate.blocked_library_id = Some(blocked.library_id);
            candidate.library_state = Some(blocked.state);
            return Ok(Some(candidate));
        }
    }
    candidate.availability = RendererAvailability::Available;
    Ok(Some(candidate))
}

fn is_auth_refusal(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<reqwest::Error>().is_some_and(|error| {
            matches!(
                error.status(),
                Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
            )
        })
    })
}

/// Check whether this node can currently read a registered library for a cast.
pub(crate) async fn cast_library_state(node: &NodeContext, library_id: Uuid) -> CastLibraryState {
    let registry = match LibraryRegistry::load(Path::new(&*node.config_path)) {
        Ok(registry) => registry,
        Err(error) => {
            log::warn!("Could not read the library registry for cast preflight: {error:#}");
            return CastLibraryState::Unreachable;
        }
    };
    let Ok(library) = registry.catalog(library_id) else {
        return CastLibraryState::NotHeld;
    };
    let ProtocolAdapter::RuneForward {
        owner_fingerprint: fingerprint,
        mode: RuneForwardMode::Replica { .. },
    } = &library.protocol
    else {
        return CastLibraryState::Ready;
    };

    let info = match node.device_info().await {
        Ok(info) => info,
        Err(error) => {
            log::warn!("Could not load local identity for cast preflight: {error:#}");
            return CastLibraryState::Unreachable;
        }
    };
    let (_, _, private_key) =
        match generate_or_load_certificates(Path::new(&*node.config_path), &info.alias).await {
            Ok(credentials) => credentials,
            Err(error) => {
                log::warn!("Could not load local credentials for cast preflight: {error:#}");
                return CastLibraryState::Unreachable;
            }
        };
    let endpoints = peer_endpoints(node, &info.fingerprint)
        .await
        .into_iter()
        .flatten()
        .filter(|endpoint| endpoint.fingerprint == *fingerprint)
        .collect::<Vec<_>>();
    if endpoints.is_empty() {
        log::warn!("Cast preflight has no eligible trusted endpoint for {fingerprint}");
    }
    for endpoint in endpoints {
        let response = match send_authenticated_request(
            node,
            &info.fingerprint,
            &private_key,
            fingerprint,
            &endpoint.host,
            endpoint.port,
            |base, client| client.get(format!("{base}/libraries")),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                log::warn!("Cast preflight could not authenticate with {fingerprint}: {error:#}");
                if error.chain().any(|cause| {
                    cause.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                        matches!(
                            error.status(),
                            Some(StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED)
                        )
                    })
                }) {
                    return CastLibraryState::AccessRevoked;
                }
                continue;
            }
        };
        if response.status() == StatusCode::FORBIDDEN
            || response.status() == StatusCode::UNAUTHORIZED
        {
            return CastLibraryState::AccessRevoked;
        }
        if response.status().is_server_error() {
            return CastLibraryState::Unreachable;
        }
        if !response.status().is_success() {
            return CastLibraryState::AccessRevoked;
        }
        let Ok(libraries) = response.json::<Vec<ListedLibrary>>().await else {
            return CastLibraryState::Unreachable;
        };
        return if libraries
            .iter()
            .any(|library| library.library_id == library_id)
        {
            CastLibraryState::Ready
        } else {
            CastLibraryState::AccessRevoked
        };
    }
    CastLibraryState::Unreachable
}

// Pairing was checked by the transport; grants decide library visibility.
fn library_visible_to_peer(
    node: &NodeContext,
    peer: &PeerIdentity,
    library: &LibraryRecord,
) -> bool {
    let Some(library_id) = library.catalog_library_id() else {
        return false;
    };
    node.access_policy.library_visible(peer, library_id)
}

pub(super) async fn list_libraries(
    Extension(manager): Extension<Arc<ServerManager>>,
    Extension(peer): Extension<PeerIdentity>,
) -> Result<Json<Vec<ListedLibrary>>, StatusCode> {
    let node = manager
        .node()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let grants = node
        .access_policy
        .grants(&peer.fingerprint)
        .map_err(|_| StatusCode::FORBIDDEN)?;
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        registry
            .catalogs()
            .filter(|library| matches!(library.protocol, ProtocolAdapter::Native))
            .filter(|library| library_visible_to_peer(&node, &peer, library))
            .filter_map(|library| {
                library
                    .catalog_library_id()
                    .map(|library_id| ListedLibrary {
                        library_id,
                        alias: library.alias.clone(),
                        connection: grants.connection_level(library_id),
                        sync: grants.sync_level(library_id),
                    })
            })
            .collect(),
    ))
}

/// Whether this device holds `fingerprint` as a client it has not approved. A peer that never
/// registered here is not refused.
async fn refused_here(node: &NodeContext, fingerprint: &str) -> bool {
    node.permission_manager
        .read()
        .await
        .verify_by_fingerprint(fingerprint)
        .await
        .is_some_and(|peer| peer.status != UserStatus::Approved)
}

pub(super) async fn list_peer_libraries(
    params: &NodeContext,
    private_key: &str,
    fingerprint: &str,
) -> Result<Vec<PeerLibrary>, RemoteConnectError> {
    let hosts = params
        .cert_validator
        .read()
        .await
        .get_hosts_for_fingerprint(fingerprint)
        .await;
    if hosts.is_empty() {
        return Err(RemoteConnectError::UntrustedEndpoint);
    }
    if refused_here(params, fingerprint).await {
        return Err(RemoteConnectError::NotAuthorized);
    }
    let own = params.device_info().await.map_err(token_error)?;
    // The trusted hosts rather than `peer_endpoints`, which skips a peer this device has not
    // approved: a peer that never paired with this device has no entry here to approve.
    let addresses = hosts
        .iter()
        .filter_map(|host| host.parse::<DeviceAddress>().ok());
    let mut failure = RemoteConnectError::Unreachable;
    for address in addresses {
        let result = async {
            let response = send_authenticated_request(
                params,
                &own.fingerprint,
                private_key,
                fingerprint,
                address.host(),
                address.port(),
                |base, client| client.get(format!("{base}/libraries")),
            )
            .await?
            .error_for_status()?;
            let libraries: Vec<ListedLibrary> = response.json().await?;
            let registry = LibraryRegistry::load(Path::new(&*params.config_path))?;
            Ok::<_, anyhow::Error>(
                libraries
                    .into_iter()
                    .map(|library| PeerLibrary {
                        added: registry
                            .catalog(library.library_id)
                            .is_ok_and(|entry| entry.owner_fingerprint() == Some(fingerprint)),
                        library_id: library.library_id.to_string(),
                        alias: library.alias,
                        connection: library.connection.into(),
                        sync: library.sync.into(),
                    })
                    .collect(),
            )
        }
        .await;
        match result {
            Ok(libraries) => return Ok(libraries),
            Err(error) => {
                failure = token_error(error);
                if !matches!(failure, RemoteConnectError::Unreachable) {
                    return Err(failure);
                }
            }
        }
    }
    Err(failure)
}

/// Add a Portable replica which can be opened through the normal local library path.
pub(crate) async fn add_remote_library(
    config_dir: &Path,
    fingerprint: &str,
    library_id: &str,
    alias: &str,
    connection_access: LibraryConnectionAccess,
    access_level: DeviceSyncLevel,
) -> Result<String> {
    let mut registry = LibraryRegistry::load(config_dir)?;
    let library_id = Uuid::parse_str(library_id).map_err(|error| {
        ExpectedRequestError::invalid_input(format!("Invalid library id: {error}"))
    })?;
    // A replica holds the owner's database, so it needs at least pull access.
    // A record without it could never open or synchronize.
    if matches!(access_level, DeviceSyncLevel::None) {
        return Err(ExpectedRequestError::LibraryRegistrationFailed {
            kind: RegisterLibraryFailureKind::SyncNotShared,
        }
        .into());
    }
    let descriptor =
        create_replica(config_dir, &mut registry, library_id, alias, fingerprint).await?;
    LibraryRegistry::set_access_level(
        config_dir,
        descriptor
            .catalog_library_id()
            .context("Added replica has no catalog")?,
        fingerprint,
        access_level.into(),
    )
    .await?;
    LibraryRegistry::set_connection_access(
        config_dir,
        descriptor
            .catalog_library_id()
            .context("Added replica has no catalog")?,
        fingerprint,
        connection_access,
    )
    .await?;
    Ok(descriptor.record_id.0.to_string())
}

#[derive(Default, Deserialize)]
pub struct LibraryQuery {
    pub library_id: Option<Uuid>,
}

pub(crate) async fn request_library(
    manager: &ServerManager,
    query: LibraryQuery,
) -> Result<LibraryHandle, StatusCode> {
    let node = manager
        .node()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let id = if let Some(id) = query.library_id {
        id
    } else if let Ok(params) = node.current_ui().await {
        params.library_id.ok_or(StatusCode::BAD_REQUEST)?
    } else {
        let ids = node
            .library_pool
            .served_library_ids()
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if ids.len() != 1 {
            return Err(StatusCode::BAD_REQUEST);
        }
        ids[0]
    };
    node.library_pool
        .acquire_served(id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)
}
