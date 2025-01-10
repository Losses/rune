use std::sync::Arc;

use anyhow::Result;

use database::connection::{check_library_state, LibraryState, MainDbConnection};

use crate::{
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
    Signal,
};

impl ParamsExtractor for TestLibraryInitializedRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for TestLibraryInitializedRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = TestLibraryInitializedResponse;

    async fn handle(
        &self,
        (_main_db,): Self::Params,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let media_library_path = dart_signal.path.clone();
        let test_result = check_library_state(&media_library_path);

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
                error: Some(format!("{:#?}", e)),
                not_ready: false,
            },
        };

        Ok(Some(result))
    }
}
