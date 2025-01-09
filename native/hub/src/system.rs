use std::sync::Arc;

use anyhow::Result;
use rinf::DartSignal;
use sysinfo::System;
use sysinfo::Users;

use database::connection::MainDbConnection;

use crate::messages::*;

pub async fn system_info_request(
    _main_db: Arc<MainDbConnection>,
    _: DartSignal<SystemInfoRequest>,
) -> Result<Option<SystemInfoResponse>> {
    let users = Users::new_with_refreshed_list();

    Ok(Some(SystemInfoResponse {
        build_date: option_env!("VERGEN_BUILD_DATE")
            .unwrap_or_default()
            .to_owned(),
        build_sha: option_env!("VERGEN_GIT_SHA").unwrap_or_default().to_owned(),
        build_commit_timestamp: option_env!("VERGEN_GIT_COMMIT_TIMESTAMP")
            .unwrap_or_default()
            .to_owned(),
        build_rustc_semver: option_env!("VERGEN_RUSTC_SEMVER")
            .unwrap_or_default()
            .to_owned(),
        system_name: System::name().unwrap_or_default(),
        system_kernel_version: System::kernel_version().unwrap_or_default(),
        system_os_version: System::os_version().unwrap_or_default(),
        system_host_name: System::host_name().unwrap_or_default(),
        users: users.into_iter().map(|x| x.name().to_owned()).collect(),
    }))
}
