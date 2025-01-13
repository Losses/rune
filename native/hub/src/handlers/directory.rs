use std::sync::Arc;

use anyhow::{Context, Result};

use database::actions::directory::get_directory_tree;
use database::actions::directory::DirectoryTree;
use database::connection::MainDbConnection;

use crate::utils::GlobalParams;
use crate::utils::ParamsExtractor;
use crate::{messages::*, Signal};

fn convert_directory_tree(tree: DirectoryTree) -> DirectoryTreeResponse {
    DirectoryTreeResponse {
        name: tree.name,
        path: tree.path,
        children: tree
            .children
            .into_iter()
            .map(convert_directory_tree)
            .collect(),
    }
}

impl ParamsExtractor for FetchDirectoryTreeRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for FetchDirectoryTreeRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = FetchDirectoryTreeResponse;
    async fn handle(
        &self,
        (main_db,): Self::Params,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let root = get_directory_tree(&main_db)
            .await
            .with_context(|| "Failed to fetch directory tree")?;

        Ok(Some(FetchDirectoryTreeResponse {
            root: Some(convert_directory_tree(root)),
        }))
    }
}
