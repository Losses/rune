use std::{path::Path, sync::Arc};

use anyhow::{Context, Result, ensure};
use database::library_registry::{LibraryAccess, LibraryConnectionAccess, LibraryRegistry};
use discovery::server::UserStatus;
use uuid::Uuid;

use crate::{
    Session, Signal,
    access_policy::{ConnectionLevel, DeviceGrants, SyncLevel},
    messages::{
        DeviceConnectionLevel, DeviceGrantsInfo, DeviceLibraryAccess, DeviceLibraryGrant,
        DevicePreset, DeviceSyncLevel, GetDeviceGrantsRequest, GetDeviceGrantsResponse,
        SetDeviceGrantsRequest, SetDeviceGrantsResponse,
    },
    node::NodeContext,
    request_error::{ExpectedRequestError, classify},
    utils::{GlobalParams, ParamsExtractor},
};

impl From<DeviceConnectionLevel> for ConnectionLevel {
    fn from(level: DeviceConnectionLevel) -> Self {
        match level {
            DeviceConnectionLevel::None => Self::None,
            DeviceConnectionLevel::View => Self::View,
            DeviceConnectionLevel::ViewEdit => Self::ViewEdit,
        }
    }
}
impl From<DeviceConnectionLevel> for LibraryConnectionAccess {
    fn from(level: DeviceConnectionLevel) -> Self {
        match level {
            DeviceConnectionLevel::None => Self::None,
            DeviceConnectionLevel::View => Self::View,
            DeviceConnectionLevel::ViewEdit => Self::ViewEdit,
        }
    }
}
impl From<ConnectionLevel> for DeviceConnectionLevel {
    fn from(level: ConnectionLevel) -> Self {
        match level {
            ConnectionLevel::None => Self::None,
            ConnectionLevel::View => Self::View,
            ConnectionLevel::ViewEdit => Self::ViewEdit,
        }
    }
}
impl From<DeviceSyncLevel> for SyncLevel {
    fn from(level: DeviceSyncLevel) -> Self {
        match level {
            DeviceSyncLevel::None => Self::None,
            DeviceSyncLevel::Pull => Self::Pull,
            DeviceSyncLevel::PullPush => Self::PullPush,
        }
    }
}
impl From<SyncLevel> for DeviceSyncLevel {
    fn from(level: SyncLevel) -> Self {
        match level {
            SyncLevel::None => Self::None,
            SyncLevel::Pull => Self::Pull,
            SyncLevel::PullPush => Self::PullPush,
        }
    }
}
impl From<DeviceSyncLevel> for LibraryAccess {
    fn from(level: DeviceSyncLevel) -> Self {
        match level {
            DeviceSyncLevel::None => Self::NotShared,
            DeviceSyncLevel::Pull => Self::Read,
            DeviceSyncLevel::PullPush => Self::ReadWrite,
        }
    }
}
impl From<LibraryAccess> for DeviceSyncLevel {
    fn from(level: LibraryAccess) -> Self {
        match level {
            LibraryAccess::NotShared => Self::None,
            LibraryAccess::Read => Self::Pull,
            LibraryAccess::ReadWrite => Self::PullPush,
        }
    }
}
impl From<LibraryConnectionAccess> for DeviceConnectionLevel {
    fn from(level: LibraryConnectionAccess) -> Self {
        match level {
            LibraryConnectionAccess::None => Self::None,
            LibraryConnectionAccess::View => Self::View,
            LibraryConnectionAccess::ViewEdit => Self::ViewEdit,
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
            .catalogs()
            .filter_map(|library| {
                library.catalog_library_id().map(|id| DeviceLibraryAccess {
                    library_id: id.to_string(),
                    alias: library.alias.clone(),
                    connection: grants.connection_level(id).into(),
                    sync: grants.sync_level(id).into(),
                })
            })
            .collect();
        Ok(Some(GetDeviceGrantsResponse {
            libraries,
            grants: DeviceGrantsInfo {
                control: grants.control,
                all_connection: grants.connection.all_libraries.map(Into::into),
                all_sync: grants.sync.all_libraries.map(Into::into),
                administer: grants.administer,
                libraries: grants
                    .connection
                    .libraries
                    .keys()
                    .chain(grants.sync.libraries.keys())
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(|id| DeviceLibraryGrant {
                        library_id: id.to_string(),
                        connection: grants
                            .connection
                            .libraries
                            .get(&id)
                            .copied()
                            .unwrap_or_default()
                            .into(),
                        sync: grants
                            .sync
                            .libraries
                            .get(&id)
                            .copied()
                            .unwrap_or_default()
                            .into(),
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
            if let Some(level) = request.all_connection {
                grants.connection.all_libraries = Some(level.into());
            }
            if let Some(level) = request.all_sync {
                grants.sync.all_libraries = Some(level.into());
            }
            for entry in &request.library_grants {
                let id = Uuid::parse_str(&entry.library_id)
                    .map_err(|error| ExpectedRequestError::invalid_input(error.to_string()))?;
                registry
                    .catalog(id)
                    .context(ExpectedRequestError::invalid_input("Unknown library"))?;
                grants
                    .connection
                    .libraries
                    .insert(id, entry.connection.into());
                grants.sync.libraries.insert(id, entry.sync.into());
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
