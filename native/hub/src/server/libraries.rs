use crate::access_policy::{Action, LibraryLevel};
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::{Extension, Json, http::StatusCode};
use database::library_registry::{
    LibraryAuthority, LibraryDescriptor, LibraryRegistry, LibraryStorage, create_replica,
};
use discovery::server::UserStatus;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::sync_worker::{authenticated_client, peer_endpoints};
use super::utils::device::load_device_info;
use super::{ServerManager, generate_or_load_certificates};
use crate::backends::remote::{RemoteConnectError, token_error};
use crate::library_pool::LibraryHandle;
use crate::messages::{CastLibraryState, DeviceLibraryLevel, PeerLibrary};
use crate::node::NodeContext;

#[derive(Serialize, Deserialize)]
pub(super) struct ListedLibrary {
    pub(super) library_id: Uuid,
    pub(super) alias: String,
    pub(super) level: LibraryLevel,
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
    let Some(library) = registry
        .libraries
        .iter()
        .find(|library| library.library_id == library_id)
    else {
        return CastLibraryState::NotHeld;
    };
    let LibraryAuthority::Hosted { fingerprint } = &library.authority else {
        return CastLibraryState::Ready;
    };

    let info = match load_device_info(Path::new(&*node.config_path)).await {
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
        let (base, client) = match authenticated_client(
            node,
            &info.fingerprint,
            &private_key,
            fingerprint,
            &endpoint.host,
            endpoint.port,
        )
        .await
        {
            Ok(client) => client,
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
        let response = match client.get(format!("{base}/libraries")).send().await {
            Ok(response) => response,
            Err(error) => {
                log::warn!(
                    "Cast preflight could not reach library authority {fingerprint}: {error}"
                );
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
    library: &LibraryDescriptor,
) -> bool {
    node.access_policy
        .check(
            peer,
            &Action::LibraryRead {
                library_id: library.library_id,
            },
        )
        .is_ok()
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
            .libraries
            .into_iter()
            .filter(|library| library.authority == LibraryAuthority::Owned)
            .filter(|library| library_visible_to_peer(&node, &peer, library))
            .map(|library| ListedLibrary {
                library_id: library.library_id,
                alias: library.alias,
                level: grants.level(library.library_id),
            })
            .collect(),
    ))
}

pub(super) async fn list_peer_libraries(
    params: &NodeContext,
    private_key: &str,
    fingerprint: &str,
) -> Result<Vec<PeerLibrary>, RemoteConnectError> {
    if params
        .cert_validator
        .read()
        .await
        .get_hosts_for_fingerprint(fingerprint)
        .await
        .is_empty()
    {
        return Err(RemoteConnectError::UntrustedEndpoint);
    }
    if params
        .permission_manager
        .read()
        .await
        .verify_by_fingerprint(fingerprint)
        .await
        .is_some_and(|peer| peer.status != UserStatus::Approved)
    {
        return Err(RemoteConnectError::NotAuthorized);
    }
    let own = load_device_info(Path::new(&*params.config_path))
        .await
        .map_err(token_error)?;
    let endpoints = peer_endpoints(params, &own.fingerprint)
        .await
        .into_iter()
        .flatten()
        .filter(|endpoint| endpoint.fingerprint == fingerprint);
    let mut failure = RemoteConnectError::Unreachable;
    for endpoint in endpoints {
        let result = async {
            let (base, client) = authenticated_client(
                params,
                &own.fingerprint,
                private_key,
                fingerprint,
                &endpoint.host,
                endpoint.port,
            )
            .await?;
            let response = client
                .get(format!("{base}/libraries"))
                .send()
                .await?
                .error_for_status()?;
            let libraries: Vec<ListedLibrary> = response.json().await?;
            let registry = LibraryRegistry::load(Path::new(&*params.config_path))?;
            Ok::<_, anyhow::Error>(
                libraries
                    .into_iter()
                    .map(|library| PeerLibrary {
                        added: registry.libraries.iter().any(|entry| {
                            entry.library_id == library.library_id
                                && entry.authority
                                    == (LibraryAuthority::Hosted {
                                        fingerprint: fingerprint.to_owned(),
                                    })
                        }),
                        library_id: library.library_id.to_string(),
                        alias: library.alias,
                        access_level: library.level.into(),
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
    access_level: DeviceLibraryLevel,
) -> Result<String> {
    let mut registry = LibraryRegistry::load(config_dir)?;
    let descriptor = create_replica(
        config_dir,
        &mut registry,
        Uuid::parse_str(library_id)?,
        alias,
        fingerprint,
    )
    .await?;
    LibraryRegistry::set_access_level(
        config_dir,
        descriptor.library_id,
        fingerprint,
        access_level.into(),
    )
    .await?;
    let LibraryStorage::Local { path } = descriptor.storage;
    Ok(path)
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
