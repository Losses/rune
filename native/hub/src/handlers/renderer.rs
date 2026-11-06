use std::sync::Arc;

use anyhow::Result;

use crate::{
    Session, Signal,
    messages::{
        AttachRendererRequest, DetachRendererRequest, ListRenderersRequest, ListRenderersResponse,
    },
    node::NodeContext,
    server::libraries::{list_renderer_candidates, parse_library_ids},
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for AttachRendererRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for AttachRendererRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        node.attach_renderer(&request.fingerprint).await?;
        Ok(None)
    }
}

impl ParamsExtractor for DetachRendererRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for DetachRendererRequest {
    type Params = Arc<NodeContext>;
    type Response = ();

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        node.detach_renderer().await?;
        Ok(None)
    }
}

impl ParamsExtractor for ListRenderersRequest {
    type Params = Arc<NodeContext>;

    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}

impl Signal for ListRenderersRequest {
    type Params = Arc<NodeContext>;
    type Response = ListRenderersResponse;

    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let library_ids = parse_library_ids(&request.library_ids)?;
        let renderers = list_renderer_candidates(node, library_ids).await?;
        Ok(Some(ListRenderersResponse { renderers }))
    }
}
