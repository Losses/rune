use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::messages::{
    AddRemoteLibraryRequest, AddRemoteLibraryResponse, ListPeerLibrariesRequest,
    ListPeerLibrariesResponse,
};
use crate::server::{ServerManager, libraries::add_remote_library};
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal};

impl ParamsExtractor for ListPeerLibrariesRequest {
    type Params = Option<Arc<ServerManager>>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.server_manager.get().cloned()
    }
}

impl Signal for ListPeerLibrariesRequest {
    type Params = Option<Arc<ServerManager>>;
    type Response = ListPeerLibrariesResponse;
    async fn handle(
        &self,
        manager: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        // Remote-control sessions have no local server; there is nothing to list from here.
        let Some(manager) = manager else {
            log::warn!("Peer libraries requested without a local server manager");
            return Ok(Some(ListPeerLibrariesResponse {
                libraries: Vec::new(),
                error_kind: None,
            }));
        };
        let response = match manager.list_peer_libraries(&request.fingerprint).await {
            Ok(libraries) => ListPeerLibrariesResponse {
                libraries,
                error_kind: None,
            },
            Err(error) => ListPeerLibrariesResponse {
                libraries: Vec::new(),
                error_kind: Some(error.kind()),
            },
        };
        Ok(Some(response))
    }
}

impl ParamsExtractor for AddRemoteLibraryRequest {
    type Params = Arc<String>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.config_path.clone()
    }
}

impl Signal for AddRemoteLibraryRequest {
    type Params = Arc<String>;
    type Response = AddRemoteLibraryResponse;
    async fn handle(
        &self,
        config: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let response = match add_remote_library(
            Path::new(&*config),
            &request.fingerprint,
            &request.library_id,
            &request.alias,
        )
        .await
        {
            Ok(path) => AddRemoteLibraryResponse { path, error: None },
            Err(error) => AddRemoteLibraryResponse {
                path: String::new(),
                error: Some(format!("{error:#}")),
            },
        };
        Ok(Some(response))
    }
}
