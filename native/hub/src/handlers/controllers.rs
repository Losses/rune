use std::sync::Arc;

use crate::request_error::ExpectedRequestError;
use crate::{
    Session, Signal,
    messages::{
        DetachControllerRequest, DetachControllerResponse, ListControllersRequest,
        ListControllersResponse,
    },
    node::NodeContext,
    utils::{GlobalParams, ParamsExtractor},
};
use anyhow::Result;
use uuid::Uuid;

impl ParamsExtractor for ListControllersRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for ListControllersRequest {
    type Params = Arc<NodeContext>;
    type Response = ListControllersResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        Ok(Some(ListControllersResponse {
            call_id: None,
            controllers: node.session()?.attached_controllers(),
        }))
    }
}

impl ParamsExtractor for DetachControllerRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for DetachControllerRequest {
    type Params = Arc<NodeContext>;
    type Response = DetachControllerResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let id = Uuid::parse_str(&request.id)
            .map_err(|_| ExpectedRequestError::invalid_input("Invalid controller id"))?;
        node.session()?.detach_controller(id).await?;
        Ok(Some(DetachControllerResponse { call_id: None }))
    }
}
