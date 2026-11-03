use crate::request_error::ExpectedRequestError;
use std::{path::Path, sync::Arc};

use anyhow::Result;
use database::library_registry::{LibraryAuthority, LibraryRegistry, LibraryStorage};
use uuid::Uuid;

use crate::{
    Session, Signal,
    messages::{
        ListLocalLibrariesRequest, ListLocalLibrariesResponse, LocalLibrary,
        SetLibraryBackgroundSyncRequest, SetLibraryBackgroundSyncResponse,
    },
    utils::{GlobalParams, ParamsExtractor},
};

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
            .libraries
            .into_iter()
            .map(|library| {
                let LibraryStorage::Local { path } = library.storage;
                LocalLibrary {
                    library_id: library.library_id.to_string(),
                    alias: library.alias,
                    path,
                    owner_fingerprint: match library.authority {
                        LibraryAuthority::Owned => None,
                        LibraryAuthority::Hosted { fingerprint } => Some(fingerprint),
                    },
                    background_sync: library.background_sync,
                }
            })
            .collect();
        Ok(Some(ListLocalLibrariesResponse { libraries }))
    }
}

impl ParamsExtractor for SetLibraryBackgroundSyncRequest {
    type Params = Arc<String>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.config_path.clone()
    }
}

impl Signal for SetLibraryBackgroundSyncRequest {
    type Params = Arc<String>;
    type Response = SetLibraryBackgroundSyncResponse;

    async fn handle(
        &self,
        config: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            let library_id = Uuid::parse_str(&request.library_id).map_err(|error| {
                ExpectedRequestError::invalid_input(format!("invalid library UUID: {error}"))
            })?;
            LibraryRegistry::set_background_sync(Path::new(&*config), library_id, request.enabled)
                .await
        }
        .await;
        Ok(Some(SetLibraryBackgroundSyncResponse {
            error: result.err().map(|error| format!("{error:#}")),
        }))
    }
}

#[cfg(all(test, not(target_os = "android")))]
mod tests {
    use super::*;
    use database::{
        connection::StorageMode,
        library_registry::{LibraryDescriptor, create_replica, resolve_library},
    };
    use fsio::FsIo;

    async fn list(config: &Arc<String>) -> Result<Vec<LocalLibrary>> {
        let request = ListLocalLibrariesRequest {};
        Ok(request
            .handle(config.clone(), None, &request)
            .await?
            .unwrap()
            .libraries)
    }

    async fn set(
        config: &Arc<String>,
        library_id: String,
        enabled: bool,
    ) -> Result<Option<String>> {
        let request = SetLibraryBackgroundSyncRequest {
            library_id,
            enabled,
        };
        Ok(request
            .handle(config.clone(), None, &request)
            .await?
            .unwrap()
            .error)
    }

    async fn register(
        config: &Path,
        replica_id: Uuid,
    ) -> Result<(LibraryDescriptor, LibraryDescriptor)> {
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
        assert!(list(&config).await?.is_empty());
        let (owned, replica) = register(dir.path(), Uuid::new_v4()).await?;
        let libraries = list(&config).await?;
        assert_eq!(libraries.len(), 2);
        let local = libraries
            .iter()
            .find(|library| library.library_id == owned.library_id.to_string())
            .unwrap();
        assert_eq!(local.alias, "Owned");
        let LibraryStorage::Local { path } = &owned.storage;
        assert_eq!(&local.path, path);
        assert_eq!(local.owner_fingerprint, None);
        assert!(local.background_sync);
        let remote = libraries
            .iter()
            .find(|library| library.library_id == replica.library_id.to_string())
            .unwrap();
        let LibraryStorage::Local { path } = &replica.storage;
        assert_eq!(remote.alias, "Replica");
        assert_eq!(&remote.path, path);
        assert_eq!(remote.owner_fingerprint.as_deref(), Some("owner"));
        assert!(remote.background_sync);
        for id in [owned.library_id, replica.library_id] {
            assert!(set(&config, id.to_string(), false).await?.is_none());
        }
        // Resolving and adding again must retain this node's disabled preference.
        register(dir.path(), replica.library_id).await?;
        assert!(
            list(&config)
                .await?
                .iter()
                .all(|library| !library.background_sync)
        );
        assert!(
            set(&config, replica.library_id.to_string(), true)
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
            assert!(set(&config, id, false).await?.is_some());
        }
        assert_eq!(LibraryRegistry::load(dir.path())?.libraries.len(), 2);
        Ok(())
    }
}
