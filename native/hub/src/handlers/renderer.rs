use std::sync::Arc;

use anyhow::Result;

use crate::{
    Signal,
    messages::{AttachRendererRequest, DetachRendererRequest},
    node::NodeContext,
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
        _: Option<crate::Session>,
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
        _: Option<crate::Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        node.detach_renderer().await?;
        Ok(None)
    }
}
