use anyhow::Result;
use log::info;

use database::connection::{LibraryState, check_library_state};

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for TestLibraryInitializedRequest {
    type Params = ();

    fn extract_params(&self, _: &GlobalParams) -> Self::Params {}
}

impl Signal for TestLibraryInitializedRequest {
    type Params = ();
    type Response = TestLibraryInitializedResponse;

    async fn handle(
        &self,
        _: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let media_library_path = dart_signal.path.clone();
        let test_result = check_library_state(&media_library_path);

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
