use anyhow::Result;
use fsio::FsIo;
use log::info;
#[cfg(target_os = "android")]
use std::path::Path;

use database::connection::{LibraryState, check_library_state};

use crate::{
    Session, Signal,
    messages::*,
    request_error::ExpectedRequestError,
    utils::{GlobalParams, ParamsExtractor},
};

impl ParamsExtractor for TestLibraryInitializedRequest {
    type Params = ();

    fn extract_params(&self, _: &GlobalParams) -> Self::Params {}
}

impl Signal for TestLibraryInitializedRequest {
    type Params = ();
    type Response = TestLibraryInitializedResponse;

    async fn handle(
        &self,
        _params: Self::Params,
        _session: Option<Session>,
        dart_signal: &Self,
    ) -> Result<Option<Self::Response>> {
        let media_library_path = dart_signal.path.clone();
        // The path is one the user is about to open, not the current library,
        // so it gets its own file access, as opening a library does.
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), &media_library_path)?;
        info!("Testing the library path: {media_library_path}");

        let state = check_library_state(&fsio, &media_library_path).map_err(|error| {
            anyhow::Error::from(ExpectedRequestError::Unavailable {
                what: "library folder",
            })
            .context(error)
        })?;

        Ok(Some(TestLibraryInitializedResponse {
            call_id: None,
            path: media_library_path,
            not_ready: matches!(state, LibraryState::Uninitialized),
        }))
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};

    use crate::Signal;
    use crate::messages::{RequestErrorKind, TestLibraryInitializedRequest};
    use crate::request_error::classify;

    fn request(path: &std::path::Path) -> Result<TestLibraryInitializedRequest> {
        Ok(TestLibraryInitializedRequest {
            path: path.to_str().context("library path")?.to_owned(),
            call_id: None,
        })
    }

    #[tokio::test]
    async fn an_empty_folder_answers_that_it_is_not_ready() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let request = request(dir.path())?;
        let response = request
            .handle((), None, &request)
            .await?
            .context("test response")?;
        assert!(response.not_ready);
        Ok(())
    }

    #[tokio::test]
    async fn an_unreadable_library_folder_is_refused_as_unavailable() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join(".rune"))?;
        std::fs::write(dir.path().join(".rune/.redirect"), "not-a-uuid")?;
        let request = request(dir.path())?;
        let error = match request.handle((), None, &request).await {
            Ok(_) => anyhow::bail!("a broken redirect must not answer a library state"),
            Err(error) => error,
        };
        assert_eq!(classify(&error), RequestErrorKind::Unavailable, "{error:#}");
        Ok(())
    }
}
