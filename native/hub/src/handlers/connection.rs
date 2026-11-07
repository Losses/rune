use std::{path::Path, sync::Arc};

use anyhow::Result;
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
        // A replica lives in the app's own directory and is created initialized.
        let replica =
            LibraryLocation::parse(&media_library_path).and_then(|location| match location {
                LibraryLocation::Replica(id) => {
                    LibraryRegistry::load(Path::new(&*config_path))?.replica(id)?;
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
