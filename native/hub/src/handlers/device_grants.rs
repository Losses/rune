use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use database::library_registry::{LibraryAccess, LibraryRegistry};
use discovery::server::UserStatus;
use uuid::Uuid;

use crate::{
    Session, Signal,
    access_policy::{DeviceGrants, LibraryLevel},
    messages::{
        DeviceGrantsInfo, DeviceLibraryAccess, DeviceLibraryGrant, DeviceLibraryLevel,
        DevicePreset, GetDeviceGrantsRequest, GetDeviceGrantsResponse, SetDeviceGrantsRequest,
        SetDeviceGrantsResponse,
    },
    node::NodeContext,
    request_error::{ExpectedRequestError, classify},
    utils::{GlobalParams, ParamsExtractor},
};

impl From<DeviceLibraryLevel> for LibraryLevel {
    fn from(level: DeviceLibraryLevel) -> Self {
        match level {
            DeviceLibraryLevel::NotShared => Self::NotShared,
            DeviceLibraryLevel::Read => Self::Read,
            DeviceLibraryLevel::ReadWrite => Self::ReadWrite,
        }
    }
}
impl From<LibraryLevel> for DeviceLibraryLevel {
    fn from(level: LibraryLevel) -> Self {
        match level {
            LibraryLevel::NotShared => Self::NotShared,
            LibraryLevel::Read => Self::Read,
            LibraryLevel::ReadWrite => Self::ReadWrite,
        }
    }
}
impl From<DeviceLibraryLevel> for LibraryAccess {
    fn from(level: DeviceLibraryLevel) -> Self {
        match level {
            DeviceLibraryLevel::NotShared => Self::NotShared,
            DeviceLibraryLevel::Read => Self::Read,
            DeviceLibraryLevel::ReadWrite => Self::ReadWrite,
        }
    }
}
impl From<LibraryAccess> for DeviceLibraryLevel {
    fn from(level: LibraryAccess) -> Self {
        match level {
            LibraryAccess::NotShared => Self::NotShared,
            LibraryAccess::Read => Self::Read,
            LibraryAccess::ReadWrite => Self::ReadWrite,
        }
    }
}
impl DevicePreset {
    pub(crate) fn grants(self) -> DeviceGrants {
        match self {
            Self::MyDevice => DeviceGrants::my_device(),
            Self::Guest => DeviceGrants::guest(),
        }
    }
}
impl ParamsExtractor for GetDeviceGrantsRequest {
    type Params = Arc<NodeContext>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}
impl Signal for GetDeviceGrantsRequest {
    type Params = Arc<NodeContext>;
    type Response = GetDeviceGrantsResponse;
    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let grants = node.access_policy.grants(&request.fingerprint)?;
        let libraries = LibraryRegistry::load(Path::new(&*node.config_path))?
            .libraries
            .into_iter()
            .map(|library| DeviceLibraryAccess {
                library_id: library.library_id.to_string(),
                alias: library.alias,
                level: grants.level(library.library_id).into(),
            })
            .collect();
        Ok(Some(GetDeviceGrantsResponse {
            libraries,
            grants: DeviceGrantsInfo {
                control: grants.control,
                all_libraries: grants.all_libraries.map(Into::into),
                administer: grants.administer,
                libraries: grants
                    .libraries
                    .into_iter()
                    .map(|(id, level)| DeviceLibraryGrant {
                        library_id: id.to_string(),
                        level: level.into(),
                    })
                    .collect(),
            },
        }))
    }
}
impl ParamsExtractor for SetDeviceGrantsRequest {
    type Params = Arc<NodeContext>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.node.clone()
    }
}
impl Signal for SetDeviceGrantsRequest {
    type Params = Arc<NodeContext>;
    type Response = SetDeviceGrantsResponse;
    async fn handle(
        &self,
        node: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let result = async {
            // Serialize with trust edits so a blocked device cannot regain stale grants.
            let permissions = node.permission_manager.write().await;
            ensure!(
                permissions
                    .verify_by_fingerprint(&request.fingerprint)
                    .await
                    .is_some_and(|user| user.status == UserStatus::Approved),
                ExpectedRequestError::invalid_input("Device is not approved")
            );
            let registry = LibraryRegistry::load(Path::new(&*node.config_path))?;
            let mut grants = match request.preset {
                Some(preset) => preset.grants(),
                None => node.access_policy.grants(&request.fingerprint)?,
            };
            if let Some(control) = request.control {
                grants.control = control;
            }
            for entry in &request.library_levels {
                let id = Uuid::parse_str(&entry.library_id)
                    .map_err(|error| ExpectedRequestError::invalid_input(error.to_string()))?;
                registry
                    .libraries
                    .iter()
                    .find(|library| library.library_id == id)
                    .context(ExpectedRequestError::invalid_input("Unknown library"))?;
                grants.libraries.insert(id, entry.level.into());
            }
            node.access_policy.set(&request.fingerprint, grants)
        }
        .await;
        Ok(Some(match result {
            Ok(()) => SetDeviceGrantsResponse {
                error_kind: None,
                error: None,
            },
            Err(error) => SetDeviceGrantsResponse {
                error_kind: Some(classify(&error)),
                error: Some(format!("{error:#}")),
            },
        }))
    }
}
