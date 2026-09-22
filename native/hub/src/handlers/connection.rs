use std::sync::Arc;

use anyhow::Result;
use log::info;

use database::connection::{LibraryState, check_library_state};
use fsio::FsIo;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for TestLibraryInitializedRequest {
    type Params = (Arc<FsIo>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.fsio),)
    }
}

impl Signal for TestLibraryInitializedRequest {
    type Params = (Arc<FsIo>,);
    type Response = TestLibraryInitializedResponse;

    async fn handle(
        &self,
        (fsio,): Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let media_library_path = dart_signal.path.clone();
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
