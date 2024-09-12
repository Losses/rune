use std::sync::Arc;

use log::{debug, error};
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
) {
    debug!("Fetching directory tree");

    match get_directory_tree(&main_db).await {
        Ok(root) => {
            FetchDirectoryTreeResponse {
                root: Some(convert_directory_tree(root)),
            }
            .send_signal_to_dart();
        }
        Err(_) => {
            error!("Failed to fetch directory tree")
        }
    }
}
