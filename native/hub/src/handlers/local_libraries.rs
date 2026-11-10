use crate::request_error::ExpectedRequestError;
use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use database::library_registry::{
    LibraryRecordId, LibraryRegistry, ProtocolAdapter, RuneForwardMode,
};
use log::warn;
use uuid::Uuid;

use crate::{
    Session, Signal,
    messages::{
        InvalidLibraryRecordSummary, InvalidRecordReasonSummary, LibraryRecordKindSummary,
        LibraryRecordSummary, LibraryRecordsChanged, ListLibraryRecordsRequest,
        ListLibraryRecordsResponse, ListLocalLibrariesRequest, ListLocalLibrariesResponse,
        LocalLibrary, OpenRecord, RemoveLibraryRecordFailureKind, RemoveLibraryRecordOutcome,
        RemoveLibraryRecordRequest, RemoveLibraryRecordResponse, SetLibraryBackgroundSyncRequest,
        SetLibraryBackgroundSyncResponse,
    },
    node::NodeContext,
    subsonic_binding::delete_subsonic_cover_cache,
    utils::{GlobalParams, ParamsExtractor},
};

async fn record_summaries(
    node: &NodeContext,
) -> Result<(
    Vec<LibraryRecordSummary>,
    Vec<InvalidLibraryRecordSummary>,
    Option<OpenRecord>,
)> {
    let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
    let devices = node.device_scanner.get_all_devices();
    let aliases: HashMap<_, _> = registry
        .libraries
        .iter()
        .filter_map(|record| match &record.protocol {
            ProtocolAdapter::RuneForward {
                owner_fingerprint: fingerprint,
                mode: RuneForwardMode::RemoteControl,
            } => Some((fingerprint.as_str(), record.alias.as_str())),
            _ => None,
        })
        .collect();
    let mut records = Vec::with_capacity(registry.libraries.len());
    for record in &registry.libraries {
        let mut summary = LibraryRecordSummary::from(record);
        if let (
            ProtocolAdapter::SubsonicTranslate { credential, .. },
            LibraryRecordKindSummary::Subsonic { account, .. },
        ) = (&record.protocol, &mut summary.kind)
        {
            // An unreadable credential must not hide every other library; opening this one
            // reports the typed error instead.
            match node.credentials.get(*credential).await {
                Ok(stored) => *account = stored.username,
                Err(error) => warn!(
                    "Could not read the account of {}: {error}",
                    record.record_id.0
                ),
            }
        }
        match &mut summary.kind {
            LibraryRecordKindSummary::Replica {
                owner_fingerprint,
                owner_alias,
                ..
            } => {
                *owner_alias = aliases
                    .get(owner_fingerprint.as_str())
                    .map(|alias| (*alias).to_owned())
                    .or_else(|| {
                        devices
                            .iter()
                            .find(|device| device.fingerprint == *owner_fingerprint)
                            .map(|device| device.alias.clone())
                    });
            }
            LibraryRecordKindSummary::RemoteControl {
                fingerprint,
                device_alias,
                endpoint,
            } => {
                let device = devices
                    .iter()
                    .find(|device| device.fingerprint == *fingerprint);
                *device_alias = device.map(|device| device.alias.clone());
                let hosts = node
                    .cert_validator
                    .read()
                    .await
                    .get_hosts_for_fingerprint(fingerprint)
                    .await;
                *endpoint = hosts.first().cloned();
            }
            _ => {}
        }
        records.push(summary);
    }
    let invalid_records = registry
        .invalid_records()
        .map(|(record, reason)| InvalidLibraryRecordSummary {
            record_id: record.record_id.0.to_string(),
            alias: record.alias.clone(),
            kind: LibraryRecordSummary::from(record).kind,
            reason: InvalidRecordReasonSummary::from(reason),
        })
        .collect();
    let (record_id, generation) = node.capability_library_identity()?;
    Ok((
        records,
        invalid_records,
        record_id.map(|record_id| OpenRecord {
            record_id,
            generation,
        }),
    ))
}

pub(crate) async fn publish_library_records_changed(node: &NodeContext) -> Result<()> {
    let (records, invalid_records, open) = record_summaries(node).await?;
    node.gui_broadcaster.broadcast(&LibraryRecordsChanged {
        records,
        invalid_records,
        open,
    });
    Ok(())
}

impl ParamsExtractor for ListLibraryRecordsRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for ListLibraryRecordsRequest {
    type Params = Arc<NodeContext>;
    type Response = ListLibraryRecordsResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let (records, invalid_records, open) = record_summaries(&node).await?;
        Ok(Some(ListLibraryRecordsResponse {
            call_id: None,
            records,
            invalid_records,
            open,
            last_open_record: LibraryRegistry::load(Path::new(&*node.config_path))?
                .last_open_record
                .map(|record_id| record_id.0.to_string()),
        }))
    }
}

impl ParamsExtractor for RemoveLibraryRecordRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for RemoveLibraryRecordRequest {
    type Params = Arc<NodeContext>;
    type Response = RemoveLibraryRecordResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let Ok(id) = Uuid::parse_str(&request.record_id) else {
            return Ok(Some(RemoveLibraryRecordResponse {
                call_id: None,
                outcome: RemoveLibraryRecordOutcome::Failed(
                    RemoveLibraryRecordFailureKind::RecordNotFound,
                ),
            }));
        };
        let (open_id, _) = node.capability_library_identity()?;
        if open_id
            .as_deref()
            .and_then(|record_id| Uuid::parse_str(record_id).ok())
            == Some(id)
        {
            return Ok(Some(RemoveLibraryRecordResponse {
                call_id: None,
                outcome: RemoveLibraryRecordOutcome::Failed(
                    RemoveLibraryRecordFailureKind::RecordIsOpen,
                ),
            }));
        }
        let config = Path::new(&*node.config_path);
        let registry = LibraryRegistry::load(config)?;
        let record = registry
            .libraries
            .iter()
            .find(|record| record.record_id.0 == id);
        if record.is_none()
            && !registry
                .invalid
                .iter()
                .any(|(record_id, _)| record_id.0 == id)
        {
            return Ok(Some(RemoveLibraryRecordResponse {
                call_id: None,
                outcome: RemoveLibraryRecordOutcome::Failed(
                    RemoveLibraryRecordFailureKind::RecordNotFound,
                ),
            }));
        }
        let library_id = record.and_then(|record| record.catalog_library_id());
        let subsonic_library_id = record.and_then(|record| {
            matches!(&record.protocol, ProtocolAdapter::SubsonicTranslate { .. })
                .then(|| record.catalog_library_id().unwrap_or(record.record_id.0))
        });
        if !LibraryRegistry::remove_record(config, LibraryRecordId(id), &node.credentials).await? {
            return Ok(Some(RemoveLibraryRecordResponse {
                call_id: None,
                outcome: RemoveLibraryRecordOutcome::Failed(
                    RemoveLibraryRecordFailureKind::RecordNotFound,
                ),
            }));
        }
        if let Some(library_id) = library_id {
            node.stop_library_sync(library_id).await;
            node.library_pool.release_background(library_id).await;
        }
        if let Some(library_id) = subsonic_library_id {
            delete_subsonic_cover_cache(config, library_id).await?;
        }
        publish_library_records_changed(&node).await?;
        Ok(Some(RemoveLibraryRecordResponse {
            call_id: None,
            outcome: RemoveLibraryRecordOutcome::Removed,
        }))
    }
}

impl ParamsExtractor for ListLocalLibrariesRequest {
    type Params = Arc<String>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.config_path.clone()
    }
}

impl Signal for ListLocalLibrariesRequest {
    type Params = Arc<String>;
    type Response = ListLocalLibrariesResponse;

    async fn handle(
        &self,
        config: Self::Params,
        _: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        let registry = LibraryRegistry::load(Path::new(&*config))?;
        let libraries = registry
            .catalogs()
            .filter_map(|library| {
                let id = library.catalog_library_id()?;
                Some(LocalLibrary {
                    library_id: id.to_string(),
                    path: library.local_folder_path().unwrap_or_default().to_owned(),
                    alias: library.alias.clone(),
                    owner_fingerprint: library.owner_fingerprint().map(str::to_owned),
                    background_sync: library.background_sync(),
                    connection_access: library.connection_access().map(Into::into),
                    sync_access: library.access_level().map(Into::into),
                })
            })
            .collect();
        Ok(Some(ListLocalLibrariesResponse {
            call_id: None,
            libraries,
        }))
    }
}

impl ParamsExtractor for SetLibraryBackgroundSyncRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for SetLibraryBackgroundSyncRequest {
    type Params = Arc<NodeContext>;
    type Response = SetLibraryBackgroundSyncResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            let library_id = Uuid::parse_str(&request.library_id).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("invalid library UUID: {error}"))
            })?;
            LibraryRegistry::set_background_sync(
                Path::new(&*node.config_path),
                library_id,
                request.enabled,
            )
            .await
        }
        .await;
        if result.is_ok() {
            publish_library_records_changed(&node).await?;
        }
        Ok(Some(SetLibraryBackgroundSyncResponse {
            call_id: None,
            error: result.err().map(|error| format!("{error:#}")),
        }))
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use crate::utils::LocalGuiBroadcaster;
    use database::{
        connection::StorageMode,
        library_registry::{LibraryRecord, create_replica, resolve_library},
    };
    use fsio::FsIo;

    async fn list(config: &Arc<String>) -> Result<Vec<LocalLibrary>> {
        let request = ListLocalLibrariesRequest { call_id: None };
        Ok(request
            .handle(config.clone(), None, &request)
            .await?
            .unwrap()
            .libraries)
    }

    async fn set(
        node: &Arc<NodeContext>,
        library_id: String,
        enabled: bool,
    ) -> Result<Option<String>> {
        let request = SetLibraryBackgroundSyncRequest {
            call_id: None,
            library_id,
            enabled,
        };
        Ok(request
            .handle(node.clone(), None, &request)
            .await?
            .unwrap()
            .error)
    }

    async fn register(config: &Path, replica_id: Uuid) -> Result<(LibraryRecord, LibraryRecord)> {
        let root = config.join("owned").to_string_lossy().into_owned();
        let mut registry = LibraryRegistry::load(config)?;
        let owned = resolve_library(
            &FsIo::new(),
            &mut registry,
            &root,
            "Owned",
            StorageMode::Portable,
        )
        .await?;
        let replica = create_replica(config, &mut registry, replica_id, "Replica", "owner").await?;
        Ok((owned, replica))
    }

    #[tokio::test]
    async fn list_and_set_local_library_options() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config = Arc::new(dir.path().to_string_lossy().into_owned());
        let node = NodeContext::new(&config, &config, Arc::new(LocalGuiBroadcaster), false).await?;
        assert!(list(&config).await?.is_empty());
        let (owned, replica) = register(dir.path(), Uuid::new_v4()).await?;
        let libraries = list(&config).await?;
        assert_eq!(libraries.len(), 2);
        let local = libraries
            .iter()
            .find(|library| library.library_id == owned.catalog_library_id().unwrap().to_string())
            .unwrap();
        assert_eq!(local.alias, "Owned");
        assert_eq!(local.path, owned.display_name().unwrap());
        assert_eq!(local.owner_fingerprint, None);
        assert!(local.background_sync);
        let remote = libraries
            .iter()
            .find(|library| library.library_id == replica.catalog_library_id().unwrap().to_string())
            .unwrap();
        assert_eq!(remote.alias, "Replica");
        assert_eq!(remote.path, "");
        assert_eq!(remote.owner_fingerprint.as_deref(), Some("owner"));
        assert!(remote.background_sync);
        for id in [
            owned.catalog_library_id().unwrap(),
            replica.catalog_library_id().unwrap(),
        ] {
            assert!(set(&node, id.to_string(), false).await?.is_none());
        }
        // Resolving and adding again must retain this node's disabled preference.
        register(dir.path(), replica.catalog_library_id().unwrap()).await?;
        assert!(
            list(&config)
                .await?
                .iter()
                .all(|library| !library.background_sync)
        );
        assert!(
            set(
                &node,
                replica.catalog_library_id().unwrap().to_string(),
                true
            )
            .await?
            .is_none()
        );
        let libraries = list(&config).await?;
        assert_eq!(
            libraries
                .iter()
                .filter(|library| library.background_sync)
                .count(),
            1
        );
        for id in ["invalid".into(), Uuid::new_v4().to_string()] {
            assert!(set(&node, id, false).await?.is_some());
        }
        assert_eq!(LibraryRegistry::load(dir.path())?.libraries.len(), 2);
        Ok(())
    }
}
