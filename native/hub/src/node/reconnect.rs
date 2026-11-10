#[cfg(test)]
use std::sync::atomic::Ordering;
use std::{
    iter::repeat,
    sync::{Arc, Weak},
    time::Duration,
};

use tokio::{
    task::JoinHandle,
    time::{Instant, sleep, sleep_until},
};
use tokio_util::sync::CancellationToken;

use super::{CurrentPlayer, LibraryBinding, NodeContext, UiLibrary};
use crate::{
    backends::remote::{RemoteConnectError, RendererLink, RendererLinkUse},
    messages::{
        RefreshSessionRequest, RemoteConnectionLost, RemoteLibraryReconnected, RemoteLossKind,
        RendererAttachmentChanged,
    },
};

#[derive(Clone)]
pub(crate) struct ReconnectConfig {
    pub delays: [Duration; 4],
    pub interval: Duration,
    pub deadline: Duration,
}

impl Default for ReconnectConfig {
    fn default() -> Self {
        Self {
            delays: [1, 2, 4, 8].map(Duration::from_secs),
            interval: Duration::from_secs(10),
            deadline: Duration::from_secs(120),
        }
    }
}

pub(super) struct ReconnectTask {
    pub alias: String,
    cancel: CancellationToken,
    handle: Option<JoinHandle<()>>,
}

impl ReconnectTask {
    pub(super) fn start(
        node: Weak<NodeContext>,
        fingerprint: String,
        alias: String,
        cancel: CancellationToken,
        config: ReconnectConfig,
        loss: (RemoteLossKind, String),
    ) -> Self {
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            let deadline = Instant::now() + config.deadline;
            let (connected, permanent_loss) = tokio::select! {
                biased;
                _ = task_cancel.cancelled() => return,
                _ = sleep_until(deadline) => (None, None),
                connected = async {
                    for delay in config.delays.into_iter().chain(repeat(config.interval)) {
                        sleep(delay).await;
                        let Some(node) = node.upgrade() else { return (None, None); };
                        if !node.begin_reconnect_attempt(&fingerprint, &task_cancel) { return (None, None); }
                        match RendererLink::connect(node.clone(), &fingerprint, RendererLinkUse::Control).await {
                            Ok((link, _, alias)) => {
                                if !link.is_closed() { return (Some((link, alias)), None); }
                            }
                            Err(error @ RemoteConnectError::PeerNeedsUpdate { .. }) => {
                                return (None, Some(error.to_string()));
                            }
                            Err(error) => log::debug!("Remote library reconnect failed: {error}"),
                        }
                    }
                    (None, None)
                } => connected,
            };
            let loss = permanent_loss.map_or(loss, |detail| {
                (RemoteLossKind::ProtocolIncompatible, detail)
            });
            let restored = node.upgrade().and_then(|node| {
                node.finish_reconnect(&fingerprint, connected, loss, &task_cancel)
            });
            if let Some(link) = restored
                && let Err(error) = link.call(&RefreshSessionRequest {}).await
            {
                log::warn!("Could not refresh reconnected session: {error:#}");
            }
        });
        Self {
            alias,
            cancel,
            handle: Some(handle),
        }
    }
}

impl Drop for ReconnectTask {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl NodeContext {
    fn begin_reconnect_attempt(&self, fingerprint: &str, cancel: &CancellationToken) -> bool {
        self.ui_library.lock().is_ok_and(|ui| {
            let active = !cancel.is_cancelled() && matches!(&*ui, UiLibrary::Open(open) if matches!(&open.catalog, LibraryBinding::RemotePeer { fingerprint: current, reconnect: Some(_) } if current == fingerprint));
            #[cfg(test)]
            if active {
                self.reconnect_attempts.fetch_add(1, Ordering::SeqCst);
            }
            active
        })
    }

    fn finish_reconnect(
        &self,
        fingerprint: &str,
        connected: Option<(RendererLink, String)>,
        loss: (RemoteLossKind, String),
        cancel: &CancellationToken,
    ) -> Option<Arc<RendererLink>> {
        let Ok(mut ui) = self.ui_library.lock() else {
            return None;
        };
        let UiLibrary::Open(open) = &mut *ui else {
            return None;
        };
        let LibraryBinding::RemotePeer {
            fingerprint: current,
            reconnect,
        } = &mut open.catalog
        else {
            return None;
        };
        if current != fingerprint || cancel.is_cancelled() {
            return None;
        }
        let mut task = reconnect.take()?;
        // This task is completing itself; dropping its handle must not abort it.
        task.handle.take();
        if let Some((link, alias)) = connected {
            if let Err(error) = self.install_renderer_link(link, alias.clone()) {
                log::warn!("Could not reinstall remote library: {error:#}");
            } else {
                self.gui_broadcaster.broadcast(&RemoteLibraryReconnected {
                    fingerprint: fingerprint.to_owned(),
                    alias,
                });
                return self.renderer_link().ok().flatten();
            }
        }
        if let Ok(mut player) = self.current_player.lock() {
            *player = CurrentPlayer::RunePeer {
                link: self.renderer_link().ok().flatten(),
                attached: false,
                reconnecting: None,
            };
        }
        self.gui_broadcaster.broadcast(&RemoteConnectionLost {
            kind: loss.0,
            detail: loss.1,
        });
        self.gui_broadcaster
            .broadcast(&RendererAttachmentChanged { renderer: None });
        None
    }

    #[cfg(test)]
    pub(crate) fn reconnect_attempt_count(&self) -> usize {
        self.reconnect_attempts.load(Ordering::SeqCst)
    }
}
