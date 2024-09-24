use std::sync::Arc;

use anyhow::{Context, Result};
use rinf::DartSignal;

use crate::DirectoryTreeResponse;

use database::actions::directory::get_directory_tree;
use database::actions::directory::DirectoryTree;
use database::connection::MainDbConnection;

use crate::{FetchDirectoryTreeRequest, FetchDirectoryTreeResponse};

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

pub async fn fetch_directory_tree_request(
    main_db: Arc<MainDbConnection>,
    _dart_signal: DartSignal<FetchDirectoryTreeRequest>,
) -> Result<()> {
    let root = get_directory_tree(&main_db)
        .await
        .with_context(|| "Failed to fetch directory tree")?;

    FetchDirectoryTreeResponse {
        root: Some(convert_directory_tree(root)),
    }
    .send_signal_to_dart();

    Ok(())
}
