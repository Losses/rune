use anyhow::Result;
use log::info;

use hub::server::utils::permission::{parse_status, print_permission_table, validate_index};

use crate::PermissionAction;

use ::discovery::{config::get_config_dir, server::PermissionManager};

pub async fn handle_permission(action: PermissionAction) -> Result<()> {
    let config_path = get_config_dir()?;
    let pm = PermissionManager::new(config_path)?;

    match action {
        PermissionAction::Ls => {
            let users = pm.list_users().await;
            print_permission_table(&users);
        }
        PermissionAction::Modify { index, status } => {
            let users = pm.list_users().await;
            validate_index(index, users.len())?;
            let user = &users[index - 1];
            let status = parse_status(&status)?;
            pm.change_user_status(&user.fingerprint, status).await?;
            info!("User status updated successfully");
        }
        PermissionAction::Delete { index } => {
            let users = pm.list_users().await;
            validate_index(index, users.len())?;
            let user = &users[index - 1];
            pm.remove_user(&user.fingerprint).await?;
            info!("User deleted successfully");
        }
    }
    Ok(())
}
