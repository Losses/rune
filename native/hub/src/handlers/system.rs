use std::sync::Arc;

use anyhow::Result;
#[cfg(not(target_os = "ios"))]
use sysinfo::System;
#[cfg(not(target_os = "ios"))]
use sysinfo::Users;

use ::database::connection::MainDbConnection;

use crate::{
    Session, Signal,
    messages::*,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for SystemInfoRequest {
    type Params = (Arc<MainDbConnection>,);

    fn extract_params(&self, all_params: &GlobalParams) -> Self::Params {
        (Arc::clone(&all_params.main_db),)
    }
}

impl Signal for SystemInfoRequest {
    type Params = (Arc<MainDbConnection>,);
    type Response = SystemInfoResponse;

    async fn handle(
        &self,
        (_main_db,): Self::Params,
        _session: Option<Session>,
        _: &Self,
    ) -> Result<Option<Self::Response>> {
        // sysinfo does not build for iOS, so the system fields stay empty there
        #[cfg(not(target_os = "ios"))]
        let (system_name, system_kernel_version, system_os_version, system_host_name, users) = (
            System::name().unwrap_or_default(),
            System::kernel_version().unwrap_or_default(),
            System::os_version().unwrap_or_default(),
            System::host_name().unwrap_or_default(),
            Users::new_with_refreshed_list()
                .into_iter()
                .map(|x| x.name().to_owned())
                .collect(),
        );
        #[cfg(target_os = "ios")]
        let (system_name, system_kernel_version, system_os_version, system_host_name, users) = (
            "iOS".to_owned(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
        );

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
            system_name,
            system_kernel_version,
            system_os_version,
            system_host_name,
            users,
        }))
    }
}
