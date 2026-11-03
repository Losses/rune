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

use super::ServerManager;
use super::sync_auth::AuthenticatedFingerprint;
use super::sync_worker::{authenticated_client, peer_endpoints};
use super::utils::device::load_device_info;
use crate::backends::remote::{RemoteConnectError, token_error};
use crate::messages::PeerLibrary;
use crate::utils::GlobalParams;

#[derive(Serialize, Deserialize)]
pub(super) struct ListedLibrary {
    library_id: Uuid,
    alias: String,
}

// Per-library permissions hook: Approved paired devices may currently read every owned library.
fn library_visible_to_peer(_fingerprint: &str, _library: &LibraryDescriptor) -> bool {
    true
}

pub(super) async fn list_libraries(
    Extension(manager): Extension<Arc<ServerManager>>,
    Extension(peer): Extension<AuthenticatedFingerprint>,
) -> Result<Json<Vec<ListedLibrary>>, StatusCode> {
    let registry = LibraryRegistry::load(Path::new(&*manager.global_params.config_path))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        registry
            .libraries
            .into_iter()
            .filter(|library| library.authority == LibraryAuthority::Owned)
            .filter(|library| library_visible_to_peer(&peer.0, library))
            .map(|library| ListedLibrary {
                library_id: library.library_id,
                alias: library.alias,
            })
            .collect(),
    ))
}

pub(super) async fn list_peer_libraries(
    params: &GlobalParams,
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
    let LibraryStorage::Local { path } = descriptor.storage;
    Ok(path)
}
