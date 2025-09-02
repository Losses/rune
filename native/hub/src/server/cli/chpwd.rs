use anyhow::Result;
use log::{error, info};
use rpassword::prompt_password;

use hub::server::update_root_password;

use ::discovery::config::get_config_dir;

pub async fn handle_chpwd() -> Result<()> {
    let config_dir = get_config_dir()?;

    loop {
        let pwd = prompt_password("Enter new password: ")?;
        let confirm = prompt_password("Confirm password: ")?;

        if pwd == confirm {
            update_root_password(&config_dir, &pwd).await?;
            info!("Password updated successfully");
            return Ok(());
        }
        error!("Passwords do not match, please try again");
    }
}
