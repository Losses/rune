use crate::request_error::ExpectedRequestError;
use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use database::library_registry::{
    LibraryRecordId, LibraryRegistry, ProtocolAdapter, RuneForwardMode, ServiceKind,
};
use log::warn;
use uuid::Uuid;

use crate::{
    Session, Signal,
    messages::{
        InvalidLibraryRecordSummary, InvalidRecordReasonSummary, LibraryRecordKindSummary,
        LibraryRecordSummary, LibraryRecordsChanged, ListLibraryRecordsRequest,
        ListLibraryRecordsResponse, ListLocalLibrariesRequest, ListLocalLibrariesResponse,
        LocalLibrary, OpenRecord, RemoveLibraryRecordFailureKind, RemoveLibraryRecordRequest,
        RemoveLibraryRecordResponse, SetOnlineLyricsRequest, SetOnlineLyricsResponse,
    },
    node::NodeContext,
    service::{
        folder::{delete_left_service_library_folders, delete_service_library_folder},
        looks_up_lyrics_online, online_lyrics,
    },
    utils::{GlobalParams, ParamsExtractor, held_service_binding},
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
        // A library whose sign-in lives in its binding is signed in only while this device
        // holds a binding that holds one.
        if record
            .protocol
            .service()
            .is_some_and(ServiceKind::signs_in_on_this_device)
        {
            summary.signed_in = Some(
                node.services
                    .held(record.record_id)
                    .is_some_and(|binding| binding.signed_in()),
            );
        }
        // A library whose lyrics come from online says whether the user allowed it to look them
        // up, which is kept on this device.
        if record
            .protocol
            .service()
            .is_some_and(looks_up_lyrics_online)
        {
            summary.online_lyrics = Some(online_lyrics::allowed(
                Path::new(&*node.config_path),
                record.record_id.0,
            ));
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
    let (record_id, generation, _) = node.capability_library_identity()?;
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
            return Err(removal_failure(
                RemoveLibraryRecordFailureKind::RecordNotFound,
            ));
        };
        let (open_id, _, _) = node.capability_library_identity()?;
        if open_id
            .as_deref()
            .and_then(|record_id| Uuid::parse_str(record_id).ok())
            == Some(id)
        {
            return Err(removal_failure(
                RemoveLibraryRecordFailureKind::RecordIsOpen,
            ));
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
            return Err(removal_failure(
                RemoveLibraryRecordFailureKind::RecordNotFound,
            ));
        }
        let library_id = record.and_then(|record| record.catalog_library_id());
        let service_library_id = record.and_then(|record| {
            record
                .protocol
                .service()
                .map(|_| record.catalog_library_id().unwrap_or(record.record_id.0))
        });
        if !LibraryRegistry::remove_record(config, LibraryRecordId(id), &node.credentials).await? {
            return Err(removal_failure(
                RemoveLibraryRecordFailureKind::RecordNotFound,
            ));
        }
        if let Some(library_id) = library_id {
            node.stop_library_sync(library_id).await;
            node.library_pool.release_background(library_id).await;
        }
        if let Some(library_id) = service_library_id {
            // Cancel any active cover downloads before deleting the cache
            node.cancel_cover_downloads_for_library(library_id).await;
        }
        // A removed service library's binding and sign-in go with it, before what it kept on
        // this device is deleted. What the user made in Rune stays, unless the request asks for
        // it to go too.
        node.services.retire(LibraryRecordId(id)).await;
        // The record is gone, so a folder left behind only takes room: the removal still answers,
        // and the UI hears of it.
        if let Some(library_id) = service_library_id
            && let Err(error) =
                delete_service_library_folder(config, library_id, !request.delete_device_data).await
        {
            warn!("Could not delete the folder of service library {library_id}: {error}");
        }
        // A reset also deletes what the user made in Rune in the service libraries removed
        // before, which outlived their library records.
        if request.delete_device_data {
            delete_left_device_data(config).await;
        }
        // The item playing here may belong to the library that went away.
        node.item_capabilities_changed();
        publish_library_records_changed(&node).await?;
        Ok(Some(RemoveLibraryRecordResponse { call_id: None }))
    }
}

/// Deletes the records the service libraries removed before left, those of no library the
/// registry in `config` still holds. What cannot be deleted is only logged, as the removal that
/// asked for it is done.
async fn delete_left_device_data(config: &Path) {
    let registry = match LibraryRegistry::load(config) {
        Ok(registry) => registry,
        Err(error) => {
            warn!("Could not read the library registry to delete the records left: {error:#}");
            return;
        }
    };
    let known = registry
        .libraries
        .iter()
        .chain(registry.invalid_records().map(|(record, _)| record))
        .flat_map(|record| [Some(record.record_id.0), record.catalog_library_id()])
        .flatten()
        .collect();
    if let Err(error) = delete_left_service_library_folders(config, known).await {
        warn!("Could not delete the records removed service libraries left: {error}");
    }
}

/// A library record that could not be removed, of `kind`.
fn removal_failure(kind: RemoveLibraryRecordFailureKind) -> anyhow::Error {
    ExpectedRequestError::LibraryRecordRemovalFailed { kind }.into()
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

impl ParamsExtractor for SetOnlineLyricsRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for SetOnlineLyricsRequest {
    type Params = Arc<NodeContext>;
    type Response = SetOnlineLyricsResponse;

    /// Keeps whether the service library of the record may look its songs' lyrics up online,
    /// for a library whose lyrics come from there. A binding this device holds for it follows
    /// at once, and the UI hears what the library's items support now and the records again.
    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let record_id = Uuid::parse_str(&request.record_id)
            .map(LibraryRecordId)
            .map_err(|error| {
                ExpectedRequestError::invalid_input(format!("invalid record UUID: {error}"))
            })?;
        let config_dir = Path::new(&*node.config_path).to_owned();
        let registry = LibraryRegistry::load(&config_dir)?;
        let offered = registry
            .record(record_id)
            .ok()
            .and_then(|record| record.protocol.service())
            .is_some_and(looks_up_lyrics_online);
        if !offered {
            return Err(ExpectedRequestError::invalid_input(
                "The record is not a library whose lyrics come from online",
            )
            .into());
        }
        let enabled = request.enabled;
        tokio::task::spawn_blocking(move || {
            online_lyrics::set_allowed(&config_dir, record_id.0, enabled)
        })
        .await??;
        if let Some(binding) = held_service_binding(&node, record_id.0)? {
            binding.set_online_lyrics(enabled);
        }
        node.item_capabilities_changed();
        publish_library_records_changed(&node).await?;
        Ok(Some(SetOnlineLyricsResponse { call_id: None }))
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use database::{
        connection::StorageMode,
        credentials::{Credential, CredentialStoreError, Secret},
        library_registry::{
            CatalogPlacement, FsAdapter, LibraryRecord, create_replica, resolve_library,
        },
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
    async fn lists_every_catalog_library_once_across_repeated_registration() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config = Arc::new(dir.path().to_string_lossy().into_owned());
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
        let remote = libraries
            .iter()
            .find(|library| library.library_id == replica.catalog_library_id().unwrap().to_string())
            .unwrap();
        assert_eq!(remote.alias, "Replica");
        assert_eq!(remote.path, "");
        assert_eq!(remote.owner_fingerprint.as_deref(), Some("owner"));
        // Resolving and adding again keeps one entry per library.
        register(dir.path(), replica.catalog_library_id().unwrap()).await?;
        let mut again: Vec<_> = list(&config)
            .await?
            .into_iter()
            .map(|library| library.library_id)
            .collect();
        again.sort();
        let mut expected = vec![
            owned.catalog_library_id().unwrap().to_string(),
            replica.catalog_library_id().unwrap().to_string(),
        ];
        expected.sort();
        assert_eq!(again, expected);
        assert_eq!(LibraryRegistry::load(dir.path())?.libraries.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn failed_credential_removal_keeps_record_listed_until_retry() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let config = Arc::new(dir.path().to_string_lossy().into_owned());
        crate::test_node::disable_sync_listener(dir.path())?;
        let node = NodeContext::new(
            &config,
            &config,
            Arc::new(LocalGuiBroadcaster),
            false,
            RunMode::App,
            Arc::new(::visualizer::Visualizer::new()),
        )
        .await?;
        let credential = node
            .credentials
            .put(Credential {
                username: "user".into(),
                secret: Secret::new(b"secret".to_vec()),
            })
            .await?;
        let record = LibraryRecord {
            record_id: LibraryRecordId(Uuid::new_v4()),
            alias: "WebDAV".into(),
            library_id: Some(Uuid::new_v4()),
            protocol: ProtocolAdapter::Native,
            fs: vec![FsAdapter::WebDav {
                endpoint: "http://example.test".into(),
                root: "/music/".into(),
                username: "user".into(),
                credential,
            }],
            catalog: CatalogPlacement::Redirected(Uuid::new_v4()),
            background_sync: true,
        };
        LibraryRegistry::update(dir.path(), |registry| {
            registry.libraries.push(record.clone());
            Ok(())
        })
        .await?;
        let listed = || async {
            let request = ListLibraryRecordsRequest { call_id: None };
            let response = request.handle(node.clone(), None, &request).await?.unwrap();
            Ok::<_, anyhow::Error>(
                response
                    .records
                    .iter()
                    .any(|listed| listed.record_id == record.record_id.0.to_string()),
            )
        };
        let remove = || async {
            let request = RemoveLibraryRecordRequest {
                record_id: record.record_id.0.to_string(),
                call_id: None,
            };
            request.handle(node.clone(), None, &request).await
        };

        let key_file = dir.path().join("credentials.key");
        let key = std::fs::read(&key_file)?;
        std::fs::write(&key_file, &key[..key.len() - 1])?;
        assert!(remove().await.is_err());
        assert!(listed().await?);
        assert!(
            LibraryRegistry::load(dir.path())?
                .record(record.record_id)
                .is_ok()
        );

        std::fs::write(&key_file, &key)?;
        assert!(remove().await?.is_some());
        assert!(!listed().await?);
        assert!(LibraryRegistry::load(dir.path())?.libraries.is_empty());
        assert!(matches!(
            node.credentials.get(credential).await,
            Err(CredentialStoreError::CredentialMissing(_))
        ));
        Ok(())
    }
}
