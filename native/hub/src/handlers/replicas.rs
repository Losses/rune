use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::backends::remote::RemoteConnectError;
use crate::messages::ProtocolUpdateSide;
use crate::messages::{
    AddRemoteLibraryRequest, AddRemoteLibraryResponse, ListPeerLibrariesRequest,
    ListPeerLibrariesResponse,
};
use crate::request_error::ExpectedRequestError;
use crate::server::{ServerManager, libraries::add_remote_library};
use crate::utils::{GlobalParams, ParamsExtractor};
use crate::{Session, Signal};

/// A peer that could not list its libraries: an incompatible version names the
/// side to update, any other failure keeps its remote-open kind.
fn peer_libraries_error(error: RemoteConnectError) -> anyhow::Error {
    let detail = error.to_string();
    let typed = match error {
        RemoteConnectError::PeerNeedsUpdate {
            local,
            peer,
            update_peer,
            peer_alias,
        } => ExpectedRequestError::PeerNeedsUpdate {
            update_side: if update_peer {
                ProtocolUpdateSide::Peer
            } else {
                ProtocolUpdateSide::ThisDevice
            },
            peer_alias,
            local_version: local,
            peer_version: peer,
        },
        error => ExpectedRequestError::RemoteOpenFailed { kind: error.kind() },
    };
    anyhow::Error::new(typed).context(detail)
}

impl ParamsExtractor for ListPeerLibrariesRequest {
    type Params = Option<Arc<ServerManager>>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.server_manager.get().cloned()
    }
}

impl Signal for ListPeerLibrariesRequest {
    type Params = Option<Arc<ServerManager>>;
    type Response = ListPeerLibrariesResponse;
    async fn handle(
        &self,
        manager: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        // Incomplete library initialization can leave the manager unavailable.
        let manager = manager.ok_or(ExpectedRequestError::Unavailable {
            what: "server manager",
        })?;
        let libraries = manager
            .list_peer_libraries(&request.fingerprint)
            .await
            .map_err(peer_libraries_error)?;
        Ok(Some(ListPeerLibrariesResponse {
            call_id: None,
            libraries,
        }))
    }
}

impl ParamsExtractor for AddRemoteLibraryRequest {
    type Params = Arc<String>;
    fn extract_params(&self, params: &GlobalParams) -> Self::Params {
        params.config_path.clone()
    }
}

impl Signal for AddRemoteLibraryRequest {
    type Params = Arc<String>;
    type Response = AddRemoteLibraryResponse;
    async fn handle(
        &self,
        config: Self::Params,
        _: Option<Session>,
        request: &Self,
    ) -> Result<Option<Self::Response>> {
        let record_id = add_remote_library(
            Path::new(&*config),
            &request.fingerprint,
            &request.library_id,
            &request.alias,
            request.connection.into(),
            request.sync,
        )
        .await?;
        Ok(Some(AddRemoteLibraryResponse {
            call_id: None,
            record_id,
        }))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use crate::Signal;
    use crate::messages::{
        AddRemoteLibraryRequest, DeviceConnectionLevel, DeviceSyncLevel, ListPeerLibrariesRequest,
        RequestErrorKind,
    };
    use crate::request_error::classify;

    #[tokio::test]
    async fn listing_without_a_server_manager_is_unavailable() -> Result<()> {
        let request = ListPeerLibrariesRequest {
            fingerprint: "peer".into(),
            call_id: None,
        };
        let error = match request.handle(None, None, &request).await {
            Ok(_) => anyhow::bail!("listing without a server manager must fail"),
            Err(error) => error,
        };
        assert_eq!(classify(&error), RequestErrorKind::Unavailable);
        Ok(())
    }

    #[tokio::test]
    async fn adding_a_library_with_an_invalid_id_is_invalid_input() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let request = AddRemoteLibraryRequest {
            fingerprint: "peer".into(),
            library_id: "not-a-library-id".into(),
            alias: "Shared".into(),
            connection: DeviceConnectionLevel::View,
            sync: DeviceSyncLevel::Pull,
            call_id: None,
        };
        let config = std::sync::Arc::new(dir.path().to_string_lossy().into_owned());
        let error = match request.handle(config, None, &request).await {
            Ok(_) => anyhow::bail!("an invalid library id must be refused"),
            Err(error) => error,
        };
        assert_eq!(
            classify(&error),
            RequestErrorKind::InvalidInput,
            "{error:#}"
        );
        Ok(())
    }
}
