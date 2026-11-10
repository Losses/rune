use std::sync::Arc;

use anyhow::Result;
use rinf::DartSignal;
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    Signal, TypedRequest,
    backends::remote::RendererLink,
    macros::RequestSlots,
    request_error::ExpectedRequestError,
    rpc::{Slot, TargetRequest},
    subsonic_binding::SubsonicBinding,
    utils::{GlobalParams, ParamsExtractor},
};

/// The implementation currently connected to the UI as its library.
#[derive(Clone)]
pub(crate) enum OpenLibrary {
    Closed,
    Local,
    RunePeer {
        reconnecting: Option<String>,
        link: Option<Arc<RendererLink>>,
    },
    Subsonic(Arc<SubsonicBinding>),
}

/// The implementation currently connected to the UI as its player.
#[derive(Clone)]
pub(crate) enum CurrentPlayer {
    Local,
    RunePeer {
        link: Option<Arc<RendererLink>>,
        attached: bool,
        reconnecting: Option<String>,
    },
    SubsonicJukebox(Arc<SubsonicBinding>),
}

/// Executes requests for one concrete library or player implementation.
#[derive(Clone)]
pub(crate) enum Implementation {
    Local,
    RunePeer {
        link: Option<Arc<RendererLink>>,
        reconnecting: Option<String>,
    },
    Subsonic(Arc<SubsonicBinding>),
    #[cfg(test)]
    TestOverride {
        implementation: Box<Implementation>,
        slots: RequestSlots,
    },
}

impl Implementation {
    pub(crate) fn slots(&self) -> RequestSlots {
        match self {
            Self::Local => RequestSlots::local(),
            Self::RunePeer {
                link: Some(link), ..
            } => {
                let mut slots = RequestSlots::rune();
                slots.narrow_to_minor(link.peer_version().minor);
                slots
            }
            Self::RunePeer { .. } => RequestSlots::rune(),
            Self::Subsonic(binding) => RequestSlots::subsonic(&binding.probe),
            #[cfg(test)]
            Self::TestOverride { slots, .. } => *slots,
        }
    }

    pub(crate) async fn run<R>(
        &self,
        params: &GlobalParams,
        request: &R,
    ) -> Result<Option<<R as TypedRequest>::Response>>
    where
        R: TargetRequest
            + Signal<
                Response = <R as TypedRequest>::Response,
                Params = <R as ParamsExtractor>::Params,
            > + ParamsExtractor
            + DartSignal
            + Serialize,
        <R as TypedRequest>::Response: DeserializeOwned,
    {
        match self.execution_implementation() {
            Self::RunePeer {
                reconnecting: Some(alias),
                ..
            } => {
                return Err(ExpectedRequestError::Reconnecting {
                    alias: alias.clone(),
                }
                .into());
            }
            Self::RunePeer { link: None, .. } => {
                return Err(ExpectedRequestError::Unavailable {
                    what: "remote UI library",
                }
                .into());
            }
            _ => {}
        }

        match R::slot(&self.slots()) {
            Slot::Native => {
                request
                    .handle(request.extract_params(params), None, request)
                    .await
            }
            Slot::Forward => {
                let Self::RunePeer {
                    link: Some(link), ..
                } = self.execution_implementation()
                else {
                    anyhow::bail!("Forward slot has no renderer link");
                };
                Ok(Some(link.call(request).await?))
            }
            Slot::Translate => {
                let Self::Subsonic(binding) = self.execution_implementation() else {
                    anyhow::bail!("Translate slot has no Subsonic binding");
                };
                request.translate(binding.as_ref()).await
            }
            Slot::Unsupported { reason } => Err(ExpectedRequestError::Unsupported {
                request: R::wire_name(),
                reason,
            }
            .into()),
        }
    }

    fn execution_implementation(&self) -> &Self {
        match self {
            #[cfg(test)]
            Self::TestOverride { implementation, .. } => implementation.execution_implementation(),
            implementation => implementation,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_slots(self, edit: impl FnOnce(&mut RequestSlots)) -> Self {
        let mut slots = self.slots();
        edit(&mut slots);
        Self::TestOverride {
            implementation: Box::new(self),
            slots,
        }
    }
}

impl OpenLibrary {
    pub(crate) fn implementation(&self) -> Implementation {
        match self {
            Self::Closed | Self::Local => Implementation::Local,
            Self::RunePeer { link, reconnecting } => Implementation::RunePeer {
                link: link.as_ref().map(Arc::clone),
                reconnecting: reconnecting.clone(),
            },
            Self::Subsonic(binding) => Implementation::Subsonic(Arc::clone(binding)),
        }
    }
}

impl CurrentPlayer {
    pub(crate) fn implementation(&self) -> Implementation {
        match self {
            Self::Local => Implementation::Local,
            Self::RunePeer {
                link, reconnecting, ..
            } => Implementation::RunePeer {
                link: link.as_ref().map(Arc::clone),
                reconnecting: reconnecting.clone(),
            },
            Self::SubsonicJukebox(binding) => Implementation::Subsonic(Arc::clone(binding)),
        }
    }

    pub(crate) fn suppresses_local_session_events(&self) -> bool {
        matches!(self, Self::RunePeer { .. })
    }
}
