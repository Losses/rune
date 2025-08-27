use anyhow::Result;
use colored::*;

use discovery::server::{UserStatus, UserSummary};

pub fn print_permission_table(users: &[UserSummary]) {
    for (i, user) in users.iter().enumerate() {
        let index = i + 1;
        let index_str = format!("[{index}]").red().bold();
        let alias = user.alias.cyan().bold();
        let device_info = format!("{} ({})", user.device_model, user.device_type).blue();
        let fingerprint_short: String = user.fingerprint.chars().take(8).collect();
        let fingerprint = fingerprint_short.magenta();
        let status = match user.status {
            UserStatus::Approved => "Approved".green(),
            UserStatus::Pending => "Pending".yellow(),
            UserStatus::Blocked => "Blocked".red(),
        };

        println!("{index_str} {alias} {device_info} {fingerprint} {status}");
    }
}

pub fn validate_index(index: usize, max: usize) -> Result<()> {
    if index < 1 || index > max {
        anyhow::bail!("Invalid index: {} (valid range: 1-{})", index, max);
    }
    Ok(())
}

pub fn parse_status(input: &str) -> Result<UserStatus> {
    match input.to_lowercase().as_str() {
        "approved" => Ok(UserStatus::Approved),
        "pending" => Ok(UserStatus::Pending),
        "blocked" => Ok(UserStatus::Blocked),
        _ => anyhow::bail!("Invalid status: {}", input),
    }
}
