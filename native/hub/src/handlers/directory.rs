use anyhow::{Context, Result};

use crate::library_pool::LibraryHandle;
use database::actions::directory::DirectoryTree;
use database::actions::directory::get_directory_tree;

use crate::Session;
use crate::utils::{GlobalParams, ParamsExtractor, require_library};
use crate::{Signal, messages::*};

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
    type Params = (Option<LibraryHandle>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (all_params.library.clone(),)
    }
}

impl Signal for FetchDirectoryTreeRequest {
    type Params = (Option<LibraryHandle>,);
    type Response = FetchDirectoryTreeResponse;
    async fn handle(
        &self,
        (library,): Self::Params,
        _session: Option<Session>,
        _dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let library = require_library(library)?;
        let root = get_directory_tree(&library.main_db)
            .await
            .with_context(|| "Failed to fetch directory tree")?;

        Ok(Some(FetchDirectoryTreeResponse {
            root: convert_directory_tree(root),
        }))
    }
}
