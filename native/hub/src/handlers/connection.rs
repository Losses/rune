use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, bail};
use fsio::FsIo;
use log::info;

use database::{
    connection::{LibraryState, check_library_state},
    library_registry::{LibraryLocation, LibraryRegistry},
};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for TestLibraryInitializedRequest {
    type Params = Arc<String>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.config_path.clone()
    }
}

impl Signal for TestLibraryInitializedRequest {
    type Params = Arc<String>;
    type Response = TestLibraryInitializedResponse;

    async fn handle(
        &self,
        config_path: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let media_library_path = dart_signal.path.clone();
        // A replica lives in the app's own directory and is created with its descriptor, while
        // its database is made on first open. Without the descriptor the folder is gone, and
        // choosing a storage mode cannot bring it back.
        let replica =
            LibraryLocation::parse(&media_library_path).and_then(|location| match location {
                LibraryLocation::Replica(id) => {
                    let config_dir = Path::new(&*config_path);
                    let root = LibraryRegistry::load(config_dir)?
                        .replica(id)?
                        .root(config_dir)
                        .context("Replica record has no storage root")?;
                    if !root.join(".rune").join("library.json").is_file() {
                        bail!("Replica folder is missing: {}", root.display());
                    }
                    Ok(true)
                }
                LibraryLocation::Path(_) => Ok(false),
            });
        match replica {
            Ok(true) => {
                return Ok(Some(TestLibraryInitializedResponse {
                    path: media_library_path,
                    success: true,
                    error: None,
                    not_ready: false,
                }));
            }
            Ok(false) => {}
            Err(e) => {
                return Ok(Some(TestLibraryInitializedResponse {
                    path: media_library_path,
                    success: false,
                    error: Some(format!("{e:#?}")),
                    not_ready: false,
                }));
            }
        }
        // The path is one the user is about to open, not the current library,
        // so it gets its own file access, as opening a library does.
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), &media_library_path)?;
        let test_result = check_library_state(&fsio, &media_library_path);

        info!("Testing the library path: {media_library_path}");

        let result = match test_result {
            Ok(state) => match &state {
                LibraryState::Uninitialized => TestLibraryInitializedResponse {
                    path: media_library_path.clone(),
                    success: true,
                    error: None,
                    not_ready: true,
                },
                LibraryState::Initialized(_) => TestLibraryInitializedResponse {
                    path: media_library_path.clone(),
                    success: true,
                    error: None,
                    not_ready: false,
                },
            },
            Err(e) => TestLibraryInitializedResponse {
                path: media_library_path.clone(),
                success: false,
                error: Some(format!("{e:#?}")),
                not_ready: false,
            },
        };

        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use database::library_registry::create_replica;
    use uuid::Uuid;

    async fn test_initialized(config: &Path, path: &str) -> TestLibraryInitializedResponse {
        let request = TestLibraryInitializedRequest {
            path: path.to_owned(),
        };
        request
            .handle(
                Arc::new(config.to_string_lossy().into_owned()),
                None,
                &request,
            )
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn a_replica_is_ready_only_while_its_folder_is_there() -> Result<()> {
        let config = tempfile::TempDir::new()?;
        let id = Uuid::new_v4();
        let mut registry = LibraryRegistry::load(config.path())?;
        create_replica(config.path(), &mut registry, id, "Replica", "owner").await?;
        let name = format!("replica://{id}");

        let ready = test_initialized(config.path(), &name).await;
        assert!(ready.success);
        assert!(!ready.not_ready);

        std::fs::remove_dir_all(
            registry
                .replica(id)?
                .root(config.path())
                .context("replica root")?,
        )?;
        let missing = test_initialized(config.path(), &name).await;
        assert!(!missing.success);
        Ok(())
    }
}
