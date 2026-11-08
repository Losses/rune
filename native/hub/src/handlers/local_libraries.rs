use crate::request_error::ExpectedRequestError;
use std::{path::Path, sync::Arc};

use anyhow::Result;
use database::library_registry::LibraryRegistry;
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
            .catalogs()
            .filter_map(|library| {
                let id = library.catalog_library_id()?;
                Some(LocalLibrary {
                    library_id: id.to_string(),
                    path: library.app_name()?,
                    alias: library.alias.clone(),
                    owner_fingerprint: library.owner_fingerprint().map(str::to_owned),
                    background_sync: library.background_sync(),
                    connection_access: library.connection_access().map(Into::into),
                    sync_access: library.access_level().map(Into::into),
                })
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
        library_registry::{LibraryRecord, create_replica, resolve_library},
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
        assert!(list(&config).await?.is_empty());
        let (owned, replica) = register(dir.path(), Uuid::new_v4()).await?;
        let libraries = list(&config).await?;
        assert_eq!(libraries.len(), 2);
        let local = libraries
            .iter()
            .find(|library| library.library_id == owned.catalog_library_id().unwrap().to_string())
            .unwrap();
        assert_eq!(local.alias, "Owned");
        assert_eq!(local.path, owned.app_name().unwrap());
        assert_eq!(local.owner_fingerprint, None);
        assert!(local.background_sync);
        let remote = libraries
            .iter()
            .find(|library| library.library_id == replica.catalog_library_id().unwrap().to_string())
            .unwrap();
        assert_eq!(remote.alias, "Replica");
        assert_eq!(
            remote.path,
            format!("replica://{}", replica.catalog_library_id().unwrap())
        );
        assert_eq!(remote.owner_fingerprint.as_deref(), Some("owner"));
        assert!(remote.background_sync);
        for id in [
            owned.catalog_library_id().unwrap(),
            replica.catalog_library_id().unwrap(),
        ] {
            assert!(set(&config, id.to_string(), false).await?.is_none());
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
                &config,
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
            assert!(set(&config, id, false).await?.is_some());
        }
        assert_eq!(LibraryRegistry::load(dir.path())?.libraries.len(), 2);
        Ok(())
    }
}
