//! Node-owned render state. Catalog resources are leased only while resolving an item.
pub(crate) mod hosted;
mod item_capabilities;
pub(crate) mod store;

use std::{
    collections::HashMap,
    future::{pending, ready},
    path::{Path, PathBuf},
    slice::from_ref,
    sync::{
        Arc, Mutex as StdMutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Error, Result};
use database::{
    actions::{
        file::file_ids_by_uuid,
        logging::{LogLevel, insert_log},
    },
    connection::MainDbConnection,
    error::ExpectedError,
    library_registry::LibraryRecordId,
    playing_item::{PlayingItemMetadataSummary, dispatcher::PlayingItemActionDispatcher},
};
use discovery::access_policy::{AccessPolicy, PeerIdentity};
use fsio::FsIo;
use futures::future::BoxFuture;
use playback::{
    InternalLog, MediaPlayback, MediaPosition, PlaybackMode,
    controller::{MediaControlManager, handle_media_control_event},
    hosted::is_hosted_path,
    player::{
        HandoffBlockId, InterruptedItem, Playable, PlaybackSource, PlaybackState, PlayedThrough,
        Player, PlayerCommandError, PlayerStatus, PlayingItem, PlaylistStatus, QueueEntry,
        TakeBackBlockResult, UnavailableReason,
    },
};
use scrobbling::{
    ScrobblingTrack,
    manager::{LoginStatus, ScrobblingError, ScrobblingManager, ScrobblingServiceManager},
};
use sea_orm::TransactionTrait;
use tokio::{
    sync::{Mutex, Notify, mpsc, oneshot},
    task::JoinHandle,
    time::{Instant, sleep_until, timeout},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    handlers::playback::playing_item_requests,
    messages::{
        AttachedControllerInfo, ControllersUpdated, CrashResponse, PlaybackStatus, PlayingItemRef,
        PlaylistItem, PlaylistUnavailableReason, PlaylistUpdate, RealtimeFFT, RequestErrorKind,
        RequestFailedResponse, ScrobbleServiceStatus, ScrobbleServiceStatusUpdated,
        VisualizerFeedChart, VisualizerFeedCovers,
    },
    node::{HiddenLocalSession, NodeContext},
    request_error::{ExpectedRequestError, player_command_error},
    request_owner::Implementation,
    server::PairedAudioResolver,
    service::ServiceBinding,
    utils::{
        Broadcaster, library_item_metadata, playback_implementation_for_item,
        player::{
            metadata_summary_to_scrobbling_track, update_media_controls_metadata,
            update_media_controls_progress,
        },
        record_library_item_played, report_item_played_through,
    },
    visualizer::{feed_cover_bytes, playing_cover_changed},
};
use store::{SessionState, StoredPlaybackOwner};
use tokio::task;

/// One renderer and its durable queue, with node-scoped event listeners.
pub(crate) struct HandoffLoadParams {
    pub expected_revision: Option<u64>,
    pub tracks: Vec<(PlayingItem, PlaybackSource)>,
    pub block: HandoffBlockId,
    pub start_index: Option<usize>,
    pub position: Duration,
    pub play: bool,
    pub playback_mode: Option<PlaybackMode>,
    /// Who started this load when it starts playback: the controller device's identity for a
    /// remote load, `None` for one this device itself made.
    pub initiator: Option<PeerIdentity>,
}

/// Builds the player a session plays through from the real one.
pub(crate) type PlayerWrap = Box<dyn FnOnce(Player) -> Arc<Mutex<dyn Playable>> + Send>;

pub struct PlayerSession {
    pub player: Arc<Mutex<dyn Playable>>,
    pub scrobbler: Arc<Mutex<dyn ScrobblingServiceManager>>,
    controls: Mutex<Option<Arc<Mutex<MediaControlManager>>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    cancel: CancellationToken,
    state: Arc<Mutex<SessionState>>,
    path: PathBuf,
    node: Weak<NodeContext>,
    persistence_writes: Arc<AtomicUsize>,
    attached_controllers: StdMutex<Vec<ControllerEntry>>,
    /// Asks the event loop to show this session in the UI again.
    show_requests: mpsc::UnboundedSender<ShowRequest>,
    /// Wakes the event loop to name the current track again.
    rename_items: Arc<Notify>,
    /// Wakes the event loop to name the current track and the queue again.
    rename_queue: Arc<Notify>,
    /// Wakes the event loop to tell the UI again what the current item supports.
    item_capabilities_changed: Arc<Notify>,
    /// Wakes the persistence task when the playback owner changes, so the owner reaches the
    /// session store whether or not any player event follows.
    playback_owner_changed: Arc<Notify>,
}

/// Asks the event loop to show this session in the UI again once it has handled everything the
/// player reported before. The loop drops `hidden` then and answers `shown`.
struct ShowRequest {
    hidden: HiddenLocalSession,
    shown: oneshot::Sender<()>,
}

struct ControllerEntry {
    controller: AttachedController,
    cancel: CancellationToken,
    /// Cancelled once the entry has left the list.
    removed: CancellationToken,
}

#[derive(Clone)]
struct AttachedController {
    id: Uuid,
    fingerprint: String,
    alias: String,
    connected_at: u64,
}

/// How long detaching a controller waits for its connection to finish closing.
const DETACH_CONTROLLER_DEADLINE: Duration = Duration::from_secs(5);

/// How long a switch back to this device's player waits for its session to be shown again. Tests
/// wait as long as for any other step, so that a slow run never stops waiting early.
#[cfg(not(test))]
const SHOW_AGAIN_WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const SHOW_AGAIN_WAIT: Duration = crate::test_wait::TEST_DEADLINE;

/// Owns one attached controller entry for the lifetime of its WebSocket.
pub(crate) struct AttachedControllerGuard {
    session: Arc<PlayerSession>,
    id: Uuid,
    cancel: CancellationToken,
}

impl AttachedControllerGuard {
    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

impl Drop for AttachedControllerGuard {
    fn drop(&mut self) {
        self.session.remove_controller(self.id);
    }
}

impl PlayerSession {
    pub(crate) fn attach_controller(
        self: &Arc<Self>,
        fingerprint: String,
        alias: String,
    ) -> AttachedControllerGuard {
        let id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        let connected_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let controller = AttachedController {
            id,
            fingerprint,
            alias,
            connected_at,
        };
        self.controllers().push(ControllerEntry {
            controller,
            cancel: cancel.clone(),
            removed: CancellationToken::new(),
        });
        self.broadcast_controllers_updated();
        AttachedControllerGuard {
            session: self.clone(),
            id,
            cancel,
        }
    }

    /// The controller list stays consistent even if a holder panicked: every
    /// critical section only pushes, retains or reads.
    fn controllers(&self) -> MutexGuard<'_, Vec<ControllerEntry>> {
        self.attached_controllers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn attached_controllers(&self) -> Vec<AttachedControllerInfo> {
        self.controllers()
            .iter()
            .map(|entry| AttachedControllerInfo {
                id: entry.controller.id.to_string(),
                fingerprint: entry.controller.fingerprint.clone(),
                alias: entry.controller.alias.clone(),
                connected_at: entry.controller.connected_at,
            })
            .collect()
    }

    /// Closes the controller's connection and returns once it is no longer listed. When the
    /// connection does not finish closing within `DETACH_CONTROLLER_DEADLINE`, the entry is
    /// removed from the list here.
    pub(crate) async fn detach_controller(&self, id: Uuid) -> Result<()> {
        let removed = {
            let controllers = self.controllers();
            let Some(entry) = controllers.iter().find(|entry| entry.controller.id == id) else {
                return Err(ExpectedError::NotFound { what: "controller" }.into());
            };
            entry.cancel.cancel();
            entry.removed.clone()
        };
        if timeout(DETACH_CONTROLLER_DEADLINE, removed.cancelled())
            .await
            .is_err()
        {
            self.remove_controller(id);
        }
        Ok(())
    }

    fn remove_controller(&self, id: Uuid) {
        let removed = {
            let mut controllers = self.controllers();
            let before = controllers.len();
            controllers.retain(|entry| {
                let keep = entry.controller.id != id;
                if !keep {
                    entry.removed.cancel();
                }
                keep
            });
            controllers.len() != before
        };
        if removed {
            self.broadcast_controllers_updated();
        }
    }

    fn broadcast_controllers_updated(&self) {
        if let Some(node) = self.node.upgrade() {
            node.broadcaster.broadcast(&ControllersUpdated {
                controllers: self.attached_controllers(),
            });
        }
    }

    pub async fn edit_queue<F>(&self, expected_revision: Option<u64>, edit: F) -> Result<()>
    where
        F: FnOnce(&mut dyn Playable) -> bool + Send,
    {
        self.edit_queue_inner(expected_revision, None, |player, _started_report| {
            (edit(player), false, false, None)
        })
        .await
    }

    /// An edit that loads playback records who started it: `initiator` is the controller
    /// device's identity for a remote load, or `Some(None)` for one this device itself made.
    /// The edit closure answers `(changed, starts_playback, displaces, completion)` and wires
    /// the report channel it receives into its playlist edit, as in [`Self::edit_queue_inner`].
    pub(crate) async fn edit_queue_with_load<F>(
        &self,
        expected_revision: Option<u64>,
        initiator: Option<PeerIdentity>,
        edit: F,
    ) -> Result<()>
    where
        F: FnOnce(
                &mut dyn Playable,
                oneshot::Sender<bool>,
            ) -> (
                bool,
                bool,
                bool,
                Option<BoxFuture<'static, Result<(), PlayerCommandError>>>,
            ) + Send,
    {
        self.edit_queue_inner(expected_revision, Some(initiator), edit)
            .await
    }

    /// Applies the edit under both locks, so ownership moves in the same serialized step that
    /// starts playback: the initiator is written before the edit's commands go out, in the
    /// order the edits themselves take effect. A load whose completion never returns — the
    /// connection revoked mid-load drops its handler — still owns the playback it started,
    /// and an interrupted load hands the ownership back only while no newer load has taken it,
    /// which the generation tells apart.
    ///
    /// An initiator the policy no longer grants control is refused before anything is sent:
    /// the revocation pass ran before this write, and nothing after it could catch the load.
    /// A revocation inside the write's own serialization window — after the admission check,
    /// before the ownership write — is re-checked where the ownership is written and paused
    /// inline, once, behind the load's commands; everything after the locks release is the
    /// revocation watcher's. Since ownership bookkeeping for a revocation is settled inside
    /// the write itself, nothing of it is lost when the caller's task is cancelled while the
    /// load completes.
    ///
    /// The edit closure answers `(changed, starts_playback, displaces, completion)` and
    /// receives the report channel the player answers with whether the edit started playback,
    /// which the closure wires into its playlist edit; the completion is the explicit one the
    /// edit's load command comes with, so the rollback decision is made from whether that
    /// command itself succeeded, not only from whether the player thread is still answering.
    /// The caller does not await the completion again: its error is this call's error.
    async fn edit_queue_inner<F>(
        &self,
        expected_revision: Option<u64>,
        initiator: Option<Option<PeerIdentity>>,
        edit: F,
    ) -> Result<()>
    where
        F: FnOnce(
                &mut dyn Playable,
                oneshot::Sender<bool>,
            ) -> (
                bool,
                bool,
                bool,
                Option<BoxFuture<'static, Result<(), PlayerCommandError>>>,
            ) + Send,
    {
        let mut state = self.state.lock().await;
        if let Some(expected) = expected_revision
            && expected != state.queue_revision
        {
            return Err(ExpectedRequestError::StaleQueueRevision {
                expected,
                current: state.queue_revision,
            }
            .into());
        }
        // A remote initiator the policy no longer grants control never writes at all: the
        // revocation's pass over the owner already ran before the edit could be authorized
        // here, so nothing the edit would send can be caught after it, and a request that
        // slips through the admission check in that window is refused rather than paused
        // after the fact. The initiator stays in the admission window only while its access
        // stands.
        if let Some(Some(initiator)) = &initiator
            && let Some(node) = self.node.upgrade()
        {
            node.access_policy.check(
                &PeerIdentity {
                    fingerprint: initiator.fingerprint.clone(),
                    public_key: initiator.public_key.clone(),
                },
                &crate::access_policy::Action::Control,
            )?;
        }
        let mut player = self.player.lock().await;
        // The revision is committed before the edit's commands go out, because they must carry
        // it: a playlist snapshot the player reports pairs its items with the revision it was
        // stamped with where the edit was applied. An edit that ends up changing nothing rolls
        // the bump back.
        let candidate = state
            .queue_revision
            .checked_add(1)
            .context("Queue revision overflow")?;
        state.queue_revision = candidate;
        player.tag_queue_revision(candidate);
        let (started_report, started_rx) = oneshot::channel();
        let (edit_made_changes, starts_playback, displaces, explicit_completion) =
            edit(&mut *player, started_report);
        player.clear_queue_revision_tag();
        if !edit_made_changes {
            state.queue_revision -= 1;
        }
        // Whether the edit started playback is the player's own answer where the edit applies
        // when it carries the report channel; the player's decision cannot be mirrored from
        // outside, since a track loaded paused without output reads externally exactly like a
        // user-paused one while the player counts it as having no current item. An edit that
        // wires no report falls back to what its closure declared.
        let started = match started_rx.await {
            Ok(started) => started,
            Err(_) => starts_playback,
        };
        let mut ownership = None;
        if (started || displaces)
            && let Some(initiator) = initiator
        {
            let generation = state.playback_owner_generation + 1;
            ownership = Some((
                generation,
                state.playback_owner.clone(),
                state.playback_owner_generation,
            ));
            if started {
                state.playback_owner = initiator.map(|initiator| StoredPlaybackOwner {
                    fingerprint: initiator.fingerprint,
                    public_key: initiator.public_key,
                });
            } else {
                // A load that displaces the current track without starting playback ends the
                // playback the previous owner started: whatever plays the new track later —
                // a resume, which never moves the ownership — is no longer it. A plain pause
                // and play on the untouched queue displaces nothing and keeps the owner.
                state.playback_owner = None;
            }
            state.playback_owner_generation = generation;
            // A revocation landing between the admission check above and this write cannot be
            // seen by the watcher — it passes while the owner here is not yet written, and the
            // load's commands are already out. The re-check happens under the same lock hold
            // the watcher serializes against, so a revoked owner is caught with certainty and
            // paused inline, once, behind the load's own commands. The pause is fire-and-forget:
            // the release is written in the same step, so a re-approved controller's newer load
            // can never be caught by it, and the revocation work does not depend on this
            // request's task surviving the wait for the load's completion.
            if let Some(owner) = state.playback_owner.clone()
                && self.node.upgrade().is_some_and(|node| {
                    node.access_policy
                        .check(
                            &PeerIdentity {
                                fingerprint: owner.fingerprint.clone(),
                                public_key: owner.public_key.clone(),
                            },
                            &crate::access_policy::Action::Control,
                        )
                        .is_err()
                })
            {
                log::info!(
                    "Pausing playback: controller {} lost control access as its load was written",
                    owner.fingerprint
                );
                player.pause();
                state.playback_owner = None;
                ownership = None;
            }
        }
        let completion = player.command_completion();
        drop(player);
        drop(state);
        if ownership.is_some() {
            self.playback_owner_changed.notify_one();
        }
        let failure = match completion.await {
            Err(error) => Some(error),
            Ok(_) => match explicit_completion {
                Some(explicit) => explicit.await.err(),
                None => None,
            },
        };
        if let Some(error) = failure {
            // `Superseded` propagates to the caller, as it always did, but it never rolls the
            // ownership back: a newer load's ownership is already safe behind its generation,
            // and a pause — the revocation watcher's, or one the device made mid-load — leaves
            // the edit's queue applied and playing nothing, so the ownership of the playback
            // the load started must survive its interruption exactly where the edit put it.
            if error != PlayerCommandError::Superseded {
                self.rollback_playback_owner(ownership).await;
            }
            return Err(Error::new(player_command_error(error)));
        }
        Ok(())
    }

    /// Hands the ownership back to what it was before the load whose completion failed, but
    /// only while no newer load has taken it: the generation the load stamped is still the
    /// current one. A newer load's ownership is never rolled back by an older load's failure.
    async fn rollback_playback_owner(
        &self,
        ownership: Option<(u64, Option<StoredPlaybackOwner>, u64)>,
    ) {
        let Some((generation, previous_owner, previous_generation)) = ownership else {
            return;
        };
        let mut state = self.state.lock().await;
        if state.playback_owner_generation == generation {
            state.playback_owner = previous_owner;
            state.playback_owner_generation = previous_generation;
            drop(state);
            self.playback_owner_changed.notify_one();
        }
    }

    pub(crate) async fn handoff_load_queue(&self, params: HandoffLoadParams) -> Result<()> {
        let mut state = self.state.lock().await;
        if let Some(expected) = params.expected_revision
            && expected != state.queue_revision
        {
            return Err(ExpectedRequestError::StaleQueueRevision {
                expected,
                current: state.queue_revision,
            }
            .into());
        }
        let mut player = self.player.lock().await;
        player.tag_queue_revision(
            state
                .queue_revision
                .checked_add(1)
                .context("Queue revision overflow")?,
        );
        // The same admission rule as in `edit_queue_inner`: an initiator already revoked when
        // the handoff is written is refused rather than dispatched and paused after the fact.
        if let Some(initiator) = &params.initiator
            && let Some(node) = self.node.upgrade()
        {
            node.access_policy.check(
                &PeerIdentity {
                    fingerprint: initiator.fingerprint.clone(),
                    public_key: initiator.public_key.clone(),
                },
                &crate::access_policy::Action::Control,
            )?;
        }
        let response = player.handoff_load(
            params.tracks,
            params.block,
            params.start_index,
            params.position,
            params.play,
            params.playback_mode,
        );
        player.clear_queue_revision_tag();
        // Ownership moves with the command itself, in the same serialized step as in
        // `edit_queue_inner`: a handoff load interrupted after this point still owns the
        // playback it started. The player only loads when a start index comes with the
        // command, so playback starts exactly when both are set, and any start displaces the
        // current track, ending the previous owner's playback the same way an edit's load does.
        let starts = params.play && params.start_index.is_some();
        let displaces = params.start_index.is_some();
        let mut ownership = None;
        if starts || displaces {
            let generation = state.playback_owner_generation + 1;
            ownership = Some((
                generation,
                state.playback_owner.clone(),
                state.playback_owner_generation,
            ));
            if starts {
                state.playback_owner = params.initiator.map(|initiator| StoredPlaybackOwner {
                    fingerprint: initiator.fingerprint,
                    public_key: initiator.public_key,
                });
            } else {
                state.playback_owner = None;
            }
            state.playback_owner_generation = generation;
            // The same revocation window re-check as in `edit_queue_inner`.
            if let Some(owner) = state.playback_owner.clone()
                && self.node.upgrade().is_some_and(|node| {
                    node.access_policy
                        .check(
                            &PeerIdentity {
                                fingerprint: owner.fingerprint.clone(),
                                public_key: owner.public_key.clone(),
                            },
                            &crate::access_policy::Action::Control,
                        )
                        .is_err()
                })
            {
                log::info!(
                    "Pausing playback: controller {} lost control access as its handoff load was written",
                    owner.fingerprint
                );
                player.pause();
                state.playback_owner = None;
                ownership = None;
            }
        }
        drop(player);
        if ownership.is_some() {
            self.playback_owner_changed.notify_one();
        }
        let result = response.await;
        if result.is_err() {
            drop(state);
            self.rollback_playback_owner(ownership).await;
            return result
                .context("Handoff load player command failed")
                .map(|_| ());
        }
        let result = result.context("Handoff load player command failed")?;
        state.add_handoff_block(params.block, result.interrupted);
        state.update_queue(&result.items);
        state.queue_revision = state
            .queue_revision
            .checked_add(1)
            .context("Queue revision overflow")?;
        drop(state);
        Ok(())
    }

    pub(crate) async fn take_back_handoff_block(
        &self,
        block: HandoffBlockId,
        resume: bool,
    ) -> Result<(TakeBackBlockResult, u64)> {
        let mut state = self.state.lock().await;
        let interrupted: InterruptedItem = state
            .handoff_block(block)
            .map(|record| record.interrupted.into())
            // A renderer can still have the tagged queue block after its session
            // metadata was refreshed. In that case the player command can remove
            // the block while preserving any current item outside it.
            .unwrap_or(InterruptedItem::Nothing {
                playback_mode: None,
            });
        let mut player = self.player.lock().await;
        player.tag_queue_revision(
            state
                .queue_revision
                .checked_add(1)
                .context("Queue revision overflow")?,
        );
        let response = player.take_back_block(block, interrupted, resume);
        player.clear_queue_revision_tag();
        drop(player);
        let result = response
            .await
            .context("Take-back player command failed")?
            .ok_or(ExpectedRequestError::Unavailable {
                what: "handoff block",
            })?;
        state.update_queue(&result.remaining);
        state.remove_handoff_block(block);
        state.queue_revision = state
            .queue_revision
            .checked_add(1)
            .context("Queue revision overflow")?;
        let new_revision = state.queue_revision;
        drop(state);
        Ok((result, new_revision))
    }

    pub(crate) async fn clear_handoff_blocks(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        let mut player = self.player.lock().await;
        player.tag_queue_revision(
            state
                .queue_revision
                .checked_add(1)
                .context("Queue revision overflow")?,
        );
        let response = player.clear_handoff_blocks();
        player.clear_queue_revision_tag();
        drop(player);
        let result = response
            .await
            .context("Clear handoff block tags player command failed")?;
        if !result.changed {
            return Ok(());
        }
        state.update_queue(&result.items);
        state.handoff_blocks.clear();
        state.queue_revision = state
            .queue_revision
            .checked_add(1)
            .context("Queue revision overflow")?;
        drop(state);
        Ok(())
    }

    pub async fn session_id(&self) -> String {
        self.state.lock().await.session_id.to_string()
    }

    pub async fn queue_revision(&self) -> u64 {
        self.state.lock().await.queue_revision
    }

    pub(crate) async fn playback_snapshot(&self) -> (PlayerStatus, u64) {
        // Locks are taken in the same order as queue edits (state then player): taking them
        // together serializes this read against every edit, so the revision pairs with exactly
        // the edits the snapshot waits for, none half applied. The barrier enqueued while the
        // locks are held cannot slip behind an edit the revision already counts.
        let state = self.state.lock().await;
        let player = self.player.lock().await;
        let revision = state.queue_revision;
        let status_after_pending = player.status_after_pending();
        drop(player);
        drop(state);
        (status_after_pending.await, revision)
    }

    /// The tracks of `library_id` in the queue, by the keys they carry.
    pub(crate) async fn queued_tracks(&self, library_id: Uuid) -> Vec<String> {
        self.player
            .lock()
            .await
            .get_playlist()
            .into_iter()
            .filter_map(|item| match item {
                PlayingItem::LibraryTrack {
                    library_id: item_library,
                    track,
                } if item_library == library_id => Some(track),
                _ => None,
            })
            .collect()
    }

    /// Names the current track and the queue again, for a library that signed in since they
    /// were last named. The player bar and the queue are sent anew from the event loop, which
    /// keeps what it found for the track playing.
    pub(crate) fn rename_queue(&self) {
        self.rename_queue.notify_one();
    }

    /// Tells the UI anew what the current item supports, for what it depends on that changed
    /// without a status: a service library that signed in or lost its sign-in, or the library
    /// the UI opened or closed.
    pub(crate) fn item_capabilities_changed(&self) {
        self.item_capabilities_changed.notify_one();
    }

    /// Re-send this session's current player bar and queue after renderer control ends.
    pub(crate) async fn reemit_current(&self) -> Result<()> {
        let Some(node) = self.node.upgrade() else {
            return Ok(());
        };
        self.reemit_to(&node.gui_broadcaster).await
    }

    /// Asks the event loop, before this returns, to show this session in the UI again after this
    /// device's player became current again, and `hidden` keeps it out of the UI until then. The
    /// loop shows it once it has handled everything the player reported before, whether or not
    /// anyone still waits. The returned future waits for that no longer than `SHOW_AGAIN_WAIT`,
    /// and a session still catching up by then stays hidden until it has.
    pub(crate) fn show_again(&self, hidden: HiddenLocalSession) -> impl Future<Output = ()> {
        let (shown, answer) = oneshot::channel();
        // A stopped event loop sends nothing more. The request comes back and is dropped, which
        // shows the session and answers at once.
        let _ = self.show_requests.send(ShowRequest { hidden, shown });
        async move {
            if timeout(SHOW_AGAIN_WAIT, answer).await.is_err() {
                log::warn!(
                    "The session is still catching up with its player after {SHOW_AGAIN_WAIT:?}, \
                     and is shown again once it has"
                );
            }
        }
    }

    pub(crate) async fn reemit_to(&self, broadcaster: &Arc<dyn Broadcaster>) -> Result<()> {
        let Some(node) = self.node.upgrade() else {
            return Ok(());
        };
        // Taken in the same order as queue edits (state then player) and held across both
        // reads, so that a queue edit cannot slip between them: it would show the queue from
        // before the edit under the revision the edit already committed. The status waits for
        // the commands enqueued so far, matching the revision exactly.
        let state = self.state.lock().await;
        let player = self.player.lock().await;
        let (session_id, revision) = (state.session_id.to_string(), state.queue_revision);
        let status = player.status_after_pending().await;
        drop(player);
        drop(state);
        let controls = self.controls.lock().await.clone();
        let mut item_state = ItemEventState {
            re_show: true,
            ..ItemEventState::default()
        };
        handle_status_event(
            &node,
            status.clone(),
            &mut item_state,
            &controls,
            None,
            broadcaster,
            &session_id,
        )
        .await;
        let playlist = PlaylistStatus {
            items: status.playlist,
            unavailable: status.unavailable_reasons,
            revision,
        };
        send_playlist(&node, &playlist, broadcaster.as_ref(), &session_id).await;
        Ok(())
    }

    pub async fn queue_for_library(&self, library_id: Uuid) -> Result<Vec<i32>> {
        let items = self
            .player
            .lock()
            .await
            .get_status()
            .playlist
            .into_iter()
            .filter_map(|entry| match entry.item {
                PlayingItem::LibraryTrack {
                    library_id: item_library,
                    track,
                } if item_library == library_id => Some(track),
                _ => None,
            })
            .collect::<Vec<_>>();
        let items = items
            .into_iter()
            .map(|track| Uuid::parse_str(&track).context("Queue contains an invalid track UUID"))
            .collect::<Result<Vec<_>>>()?;
        let node = self
            .node
            .upgrade()
            .ok_or(ExpectedRequestError::Unavailable {
                what: "player node",
            })?;
        let library = node
            .library_pool
            .acquire(library_id)
            .await
            .map_err(|_| ExpectedRequestError::Unavailable { what: "library" })?;
        let ids = file_ids_by_uuid(&library.main_db, &items).await?;
        Ok(items
            .iter()
            .filter_map(|uuid| ids.get(uuid).copied())
            .collect())
    }

    /// The player a session plays through, made of the real one. Production uses the real
    /// player as it is; tests put a player of their own around it.
    pub(crate) fn real_player() -> PlayerWrap {
        Box::new(|player| Arc::new(Mutex::new(player)))
    }

    pub(crate) async fn new(
        node: &Arc<NodeContext>,
        name: &str,
        os_media_controls: bool,
        wrap_player: PlayerWrap,
    ) -> Result<Arc<Self>> {
        let path = Path::new(&*node.config_path)
            .join("sessions")
            .join(format!("{name}.json"));
        let state = SessionState::read(&path)?;
        let cancel = node.token.child_token();
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), "")?;
        let player = wrap_player(Player::new(Arc::new(fsio), Some(cancel.child_token())));
        player
            .lock()
            .await
            .set_source_resolver(Arc::new(PairedAudioResolver(
                Arc::downgrade(node),
                String::new(),
            )));
        // Only this device's own player: the host that plays a hosted track is the app here.
        player
            .lock()
            .await
            .set_host_bridge(Arc::new(hosted::JsonHostBridge::to_app()));
        let controls = if os_media_controls {
            match MediaControlManager::with_name(&format!(
                "rune_player_{}_{}",
                node.node_id.replace('-', "_"),
                name
            ))
            .and_then(|mut manager| {
                manager.initialize()?;
                Ok(manager)
            }) {
                Ok(manager) => Some(Arc::new(Mutex::new(manager))),
                Err(error) => {
                    log::warn!("OS media controls unavailable: {error:#}");
                    None
                }
            }
        } else {
            None
        };
        let (show_requests, show_receiver) = mpsc::unbounded_channel();
        let session = Arc::new(Self {
            player,
            scrobbler: Arc::new(Mutex::new(ScrobblingManager::new(
                10,
                Duration::from_secs(5),
            ))),
            controls: Mutex::new(controls),
            tasks: Mutex::new(Vec::new()),
            cancel,
            state: Arc::new(Mutex::new(state.clone())),
            path,
            node: Arc::downgrade(node),
            persistence_writes: Arc::new(AtomicUsize::new(0)),
            attached_controllers: StdMutex::new(Vec::new()),
            show_requests,
            rename_items: Arc::new(Notify::new()),
            rename_queue: Arc::new(Notify::new()),
            item_capabilities_changed: Arc::new(Notify::new()),
            playback_owner_changed: Arc::new(Notify::new()),
        });
        session.start_events(node, show_receiver).await;
        // The restored queue is the queue the persisted revision describes: restore is no
        // edit and bumps nothing, but it must carry the revision edits continue from.
        session.player.lock().await.restore_tagged(
            state.queue_entries(),
            state.index,
            state.position,
            state.playback_mode(),
            state.queue_revision,
        );
        Ok(session)
    }

    /// Flush durable state and await the event listeners before terminating playback.
    pub async fn shutdown(&self) -> Result<()> {
        if self.cancel.is_cancelled() {
            return Ok(());
        }
        let status = self.player.lock().await.get_status();
        self.cancel.cancel();
        for task in self.tasks.lock().await.drain(..) {
            task.await.context("Player session listener failed")?;
        }
        if let Some(manager) = self.controls.lock().await.take()
            && let Err(error) = manager.lock().await.controls.detach()
        {
            log::warn!("Could not detach OS media controls: {error:?}");
        }
        let mut state = self.state.lock().await;
        state.update(&status);
        let result = state.write(&self.path);
        self.player.lock().await.terminate();
        result
    }

    async fn start_events(
        &self,
        node: &Arc<NodeContext>,
        mut show_requests: mpsc::UnboundedReceiver<ShowRequest>,
    ) {
        let player = self.player.lock().await;
        // The one stream keeps statuses and completions in the order the player produced them,
        // so a completion handled here really preceded every status handled after it, and a
        // completion that still plays out after a status belongs to its occurrence.
        let playback_events = player.subscribe_playback_events();
        let playlist = player.subscribe_playlist();
        let fft = player.subscribe_realtime_fft();
        let crash = player.subscribe_crash();
        let logs = player.subscribe_log();
        drop(player);
        let scrobbler = self.scrobbler.lock().await;
        let errors = scrobbler.subscribe_error();
        let login = scrobbler.subscribe_login_status();
        drop(scrobbler);
        let controls = self.controls.lock().await.clone();
        let controller = if let Some(manager) = &controls {
            Some(manager.lock().await.subscribe_controller_events())
        } else {
            None
        };
        let weak = self.node.clone();
        let broadcaster = node.broadcaster.clone();
        let player = self.player.clone();
        let scrobbler = self.scrobbler.clone();
        let cancel = self.cancel.clone();
        let session_id = self.state.lock().await.session_id.to_string();
        let rename_items = self.rename_items.clone();
        let rename_queue = self.rename_queue.clone();
        let item_capabilities_changed = self.item_capabilities_changed.clone();
        let (persist_status_tx, mut persist_status_rx) = mpsc::channel(32);
        let (persist_playlist_tx, mut persist_playlist_rx) = mpsc::channel(32);
        // Authentication can hold the scrobbler lock across network retries.
        let (now_playing_tx, mut now_playing_rx) = mpsc::unbounded_channel();
        let report_scrobbler = self.scrobbler.clone();
        let report_cancel = self.cancel.clone();
        let now_playing_task = tokio::spawn(async move {
            let _ = report_cancel
                .run_until_cancelled(async {
                    while let Some(track) = now_playing_rx.recv().await {
                        report_scrobbler.lock().await.update_now_playing_all(track);
                    }
                })
                .await;
        });
        let (late_cover_tx, mut late_cover_rx) = mpsc::unbounded_channel();
        let event_task = tokio::spawn(async move {
            let mut item_state = ItemEventState {
                late_covers: Some(late_cover_tx),
                late_names: Some(rename_items.clone()),
                ..ItemEventState::default()
            };
            // The requests to show the session again that are still waiting, and the status
            // barrier they wait for. The player answers a barrier once it has queued its reports
            // of everything it did before, so one sent after the last request comes after every
            // report from before this device's player became current again.
            let mut shows = Vec::new();
            let mut barrier: Option<BoxFuture<'static, PlayerStatus>> = None;
            // The status the barrier answered with, for the node to note once the session shows.
            let mut answered: Option<PlayerStatus> = None;
            // The status the loop handled last, and whether a track named late waits to go out
            // again on it.
            let mut last_status: Option<PlayerStatus> = None;
            let mut late_name = false;
            // The queue the loop handled last, and whether it waits to go out again, named anew.
            let mut last_playlist: Option<PlaylistStatus> = None;
            let mut late_queue = false;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        if let Some(manager) = &controls { let _ = manager.lock().await.controls.detach(); }
                        return;
                    },
                    Some(show) = show_requests.recv() => {
                        shows.push(show);
                        barrier = Some(player.lock().await.status_after_pending());
                    }
                    answer = async { match barrier.as_mut() { Some(barrier) => barrier.await, None => pending().await } } => {
                        barrier = None;
                        answered = Some(answer);
                    }
                    Ok(event) = playback_events.recv() => {
                        let Some(node) = weak.upgrade() else { return };
                        match event {
                            playback::player::PlaybackEvent::Played(played) => {
                                handle_played_event(&node, played, &mut item_state, &scrobbler).await;
                            }
                            playback::player::PlaybackEvent::Status(value) => {
                                if persist_status_tx.send(value.clone()).await.is_err() { return; }
                                last_status = Some(value.clone());
                                // A report handled after the barrier answered says at least as much as the
                                // answer, so the session shows again without noting the answer over it.
                                answered = None;
                                note_local_track(&node, &value);
                                handle_status_event(&node, value, &mut item_state, &controls, Some(&now_playing_tx), &broadcaster, &session_id).await;
                            }
                        }
                    }
                    Some(late) = late_cover_rx.recv() => {
                        let Some(node) = weak.upgrade() else { return };
                        if show_late_cover(&node, late, &mut item_state, &controls).await {
                            let status = player.lock().await.get_status();
                            // Statuses still queued come before this snapshot. If the player has
                            // moved to another item, they report that change in order.
                            if status.item == item_state.last_item {
                                last_status = Some(status.clone());
                                handle_status_event(&node, status, &mut item_state, &controls, Some(&now_playing_tx), &broadcaster, &session_id).await;
                            }
                        }
                    }
                    Ok(value) = playlist.recv() => {
                        if persist_playlist_tx.send(value.clone()).await.is_err() { return; }
                        last_playlist = Some(value.clone());
                        if let Some(node) = weak.upgrade() {
                            // The player stamped the snapshot with the revision of the edit that
                            // produced it, where the edit was applied, so the queue and the
                            // revision it goes out with describe the same queue state.
                            handle_playlist_event(&node, value, broadcaster.as_ref(), &session_id).await;
                        }
                    }
                    // Taken in a later pass than the one that saw the barrier answered, and only
                    // when no status or queue report is waiting, so the loop has handled every
                    // report the player queued before answering.
                    _ = ready(()), if barrier.is_none() && !shows.is_empty() => {
                        // Dropping the hidden sessions lets the session's events reach the UI.
                        let answers: Vec<_> = shows
                            .drain(..)
                            .map(|ShowRequest { hidden, shown }| {
                                drop(hidden);
                                shown
                            })
                            .collect();
                        if let Some(node) = weak.upgrade() {
                            // Every report from before the barrier is handled, so its status is
                            // what this device's player holds as the session shows again.
                            if let Some(status) = answered.take() {
                                note_local_track(&node, &status);
                            }
                            if let Ok(session) = node.session()
                                && let Err(error) = session.reemit_to(&broadcaster).await
                            {
                                log::warn!("Could not show the session again: {error:#}");
                            }
                        }
                        for shown in answers {
                            let _ = shown.send(());
                        }
                    }
                    // A track named late goes out again only when no status report waits, and on
                    // the status handled last: a snapshot taken from the player now could be newer
                    // than reports still queued, which would then take the UI back.
                    _ = ready(()), if late_name => {
                        late_name = false;
                        let Some(node) = weak.upgrade() else { return };
                        if name_late_track(&mut item_state, &controls, true).await
                            && let Some(status) = last_status.clone()
                        {
                            note_local_track(&node, &status);
                            handle_status_event(&node, status, &mut item_state, &controls, Some(&now_playing_tx), &broadcaster, &session_id).await;
                        }
                    }
                    // The queue goes out again the same way, as handled last, once no report waits.
                    _ = ready(()), if late_queue => {
                        late_queue = false;
                        let Some(node) = weak.upgrade() else { return };
                        if let Some(playlist) = last_playlist.clone() {
                            handle_playlist_event(&node, playlist, broadcaster.as_ref(), &session_id).await;
                        }
                    }
                    Ok(value) = fft.recv() => broadcaster.broadcast(&RealtimeFFT { value }),
                    Ok(detail) = crash.recv() => broadcaster.broadcast(&CrashResponse { detail }),
                    Ok(error) = logs.recv() => {
                        if let Some(node) = weak.upgrade() {
                            handle_log_event(&node, error, broadcaster.as_ref()).await;
                        }
                    }
                    Ok(error) = errors.recv() => {
                        handle_scrobbler_error(weak.upgrade(), error).await;
                    }
                    Ok(value) = login.recv() => handle_login_event(value, broadcaster.as_ref()),
                    // A service named a track late. Only its status goes out again, once no report
                    // waits: the queue names its tracks as it is read, and reading it again would
                    // ask the service about every queued track.
                    _ = rename_items.notified() => late_name = true,
                    // A library signed in, and the tracks of it in the queue were read together
                    // first. The queue goes out again too, its rows of the library named from what
                    // the library's binding holds now, without a request each.
                    _ = rename_queue.notified() => {
                        late_name = true;
                        late_queue = true;
                    }
                    _ = item_capabilities_changed.notified() => {
                        let Some(node) = weak.upgrade() else { return };
                        item_capabilities::changed(&node, &mut item_state, broadcaster.as_ref()).await;
                    }
                    Some(value) = async { match &controller { Some(receiver) => receiver.recv().await.ok(), None => pending().await } } => { let _ = handle_media_control_event(&player, value).await; }
                }
            }
        });
        let state = self.state.clone();
        let path = self.path.clone();
        let persistence_writes = self.persistence_writes.clone();
        let cancel = self.cancel.clone();
        let owner_changed = self.playback_owner_changed.clone();
        let persistence_task = tokio::spawn(async move {
            let mut written = state.lock().await.clone();
            let mut dirty_deadline = None;
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        flush_session_state(&state, &path, &mut written, &persistence_writes).await;
                        return;
                    },
                    // The owner moves without any player event — a revocation that releases a
                    // paused playback's owner emits none — so it must mark the store dirty
                    // itself instead of riding the load's queue and status reports.
                    _ = owner_changed.notified() => {
                        dirty_deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
                    }
                    Some(value) = persist_playlist_rx.recv() => {
                        record_playlist_change(&state, value.items, &mut dirty_deadline).await;
                    }
                    Some(value) = persist_status_rx.recv() => {
                        record_status_change(&state, &value, &mut dirty_deadline).await;
                    }
                    _ = sleep_until(dirty_deadline.unwrap_or_else(Instant::now)), if dirty_deadline.is_some() =>
                        persist_when_due(&state, &path, &mut written, &mut dirty_deadline, &persistence_writes).await,
                }
            }
        });
        // Revoking a controller's control access stops the playback it started, wherever its
        // connection is by then. The owner persists with the queue, so the playback stays the
        // controller's own across pauses, reconnects and restarts; a playback the device
        // itself or another controller loaded survives the revocation. The owner is released
        // once the revocation has acted — playback already paused needs no acting on — so
        // what the device resumes afterwards is its own again. The first pass runs before
        // waiting for a change because the revocation may have finished while the session was
        // down or still starting.
        let state = self.state.clone();
        let player = self.player.clone();
        let revoke_weak = self.node.clone();
        let revoke_cancel = self.cancel.clone();
        let owner_notify = self.playback_owner_changed.clone();
        let revoke_task = tokio::spawn(async move {
            let Some(node) = revoke_weak.upgrade() else {
                return;
            };
            let policy = node.access_policy.clone();
            drop(node);
            let mut changes = policy.changes();
            loop {
                revoke_playback_owner_once(&state, &player, &owner_notify, &revoke_weak).await;
                tokio::select! {
                    biased;
                    _ = revoke_cancel.cancelled() => return,
                    result = changes.changed() => if result.is_err() { return },
                }
            }
        });
        self.tasks.lock().await.extend([
            event_task,
            persistence_task,
            now_playing_task,
            revoke_task,
        ]);
    }
}

/// Pauses the playback the current owner started and releases the ownership when that owner no
/// longer holds control access. The owner is read again under both locks, in the same order the
/// load edits take them, so the pause is serialized with the loads that move ownership: playback
/// another controller or the device itself started is never paused here. The pause is not
/// conditioned on the player state — a load that has not reached Playing yet would otherwise
/// slip past it — and pausing what already stands paused is a no-op. The generation is
/// remembered with the pause: while it is awaited the locks are off, and a load a re-approved
/// controller started meanwhile must survive the release, so the release is conditioned on both
/// the owner and its generation standing unchanged. The revocation watcher runs this pass on
/// every access-policy change; the load edits run it after their completion when the owner they
/// recorded was already revoked while the load was in flight, a revocation no policy change can
/// wake the watcher for again.
async fn revoke_playback_owner_once(
    state: &Mutex<SessionState>,
    player: &Mutex<dyn Playable>,
    owner_notify: &Notify,
    node: &Weak<NodeContext>,
) {
    let Some(node) = node.upgrade() else {
        return;
    };
    let policy = node.access_policy.clone();
    drop(node);
    let revoked = {
        let owner = state.lock().await.playback_owner.clone();
        owner.filter(|owner| {
            policy
                .check(
                    &PeerIdentity {
                        fingerprint: owner.fingerprint.clone(),
                        public_key: owner.public_key.clone(),
                    },
                    &crate::access_policy::Action::Control,
                )
                .is_err()
        })
    };
    let Some(owner) = revoked else {
        return;
    };
    let fingerprint = owner.fingerprint.clone();
    let completion = {
        let state = state.lock().await;
        if state.playback_owner.as_ref() != Some(&owner) {
            drop(state);
            None
        } else {
            let generation = state.playback_owner_generation;
            let mut player = player.lock().await;
            let completion = player.command_with_completion(playback::PlayerCommand::Pause);
            drop(player);
            drop(state);
            Some((completion, generation))
        }
    };
    if let Some((completion, generation)) = completion {
        if let Err(error) = completion.await {
            log::warn!(
                "Could not pause playback of the revoked controller {fingerprint}: {error:?}"
            );
        } else {
            log::info!("Paused playback: controller {fingerprint} lost control access");
        }
        let mut state = state.lock().await;
        if state.playback_owner.as_ref() == Some(&owner)
            && state.playback_owner_generation == generation
        {
            state.playback_owner = None;
            drop(state);
            owner_notify.notify_one();
        } else {
            drop(state);
        }
    }
}

async fn update_persisted_cursor(state: &Mutex<SessionState>, status: &PlayerStatus) -> bool {
    let mut state = state.lock().await;
    let previous = state.clone();
    state.update_cursor(status);
    *state != previous
}

async fn record_playlist_change(
    state: &Mutex<SessionState>,
    items: Vec<QueueEntry>,
    deadline: &mut Option<Instant>,
) {
    let mut state = state.lock().await;
    let previous = state.clone();
    state.update_queue(&items);
    if *state != previous {
        deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
    }
}

async fn record_status_change(
    state: &Mutex<SessionState>,
    status: &PlayerStatus,
    deadline: &mut Option<Instant>,
) {
    if update_persisted_cursor(state, status).await {
        deadline.get_or_insert_with(|| Instant::now() + Duration::from_millis(300));
    }
}

async fn persist_when_due(
    state: &Mutex<SessionState>,
    path: &Path,
    written: &mut SessionState,
    deadline: &mut Option<Instant>,
    writes: &AtomicUsize,
) {
    if deadline.take().is_some()
        && let Err(error) = persist_session_state(state, path, written, writes).await
    {
        log::error!("Could not persist player session: {error:#}");
        *deadline = Some(Instant::now() + Duration::from_millis(300));
    }
}

async fn flush_session_state(
    state: &Mutex<SessionState>,
    path: &Path,
    written: &mut SessionState,
    writes: &AtomicUsize,
) {
    if let Err(error) = persist_session_state(state, path, written, writes).await {
        log::error!("Could not flush player session: {error:#}");
    }
}

async fn persist_session_state(
    state: &Mutex<SessionState>,
    path: &Path,
    written: &mut SessionState,
    writes: &AtomicUsize,
) -> Result<()> {
    let current = state.lock().await;
    if *current != *written {
        current.write(path)?;
        writes.fetch_add(1, Ordering::AcqRel);
        *written = current.clone();
    }
    Ok(())
}

#[derive(Default)]
struct ItemEventState {
    last_item: Option<PlayingItem>,
    library: Option<ResolvedLibrary>,
    metadata: PlayingItemMetadataSummary,
    cover: Option<String>,
    now_playing_pending: bool,
    track_started: bool,
    /// `last_item` as sent to Dart, valid for `request_ui_library`.
    request: Option<PlayingItemRef>,
    request_ui_library: Option<Uuid>,
    /// What the UI was last told `request` supports.
    capabilities: item_capabilities::Declared,
    feed_item: Option<PlayingItemRef>,
    feed_controller_count: usize,
    /// Where a cover that finishes downloading after its track started is sent, when the
    /// caller follows the player and can show it.
    late_covers: Option<mpsc::UnboundedSender<LateCover>>,
    /// Told when a service names a track only after its status went out unnamed, when the
    /// caller follows the player and can name the track again.
    late_names: Option<Arc<Notify>>,
    /// Whether `metadata` was found for `last_item`.
    metadata_found: bool,
    /// Whether the system's Now Playing still lacks what `metadata` says.
    controls_behind: bool,
    /// Whether Rune's own Now Playing gave way to a platform host that plays the track.
    controls_yielded: bool,
    /// Whether this state only shows the current state again, outside the session's event loop.
    /// It does not know whether Now Playing gave way to a host, so it hands nothing over: only
    /// the loop does.
    re_show: bool,
}

/// What one status report is, for the system's Now Playing.
#[derive(Clone, Copy, Debug)]
struct NowPlayingReport {
    item_changed: bool,
    metadata_found: bool,
    hosted: bool,
    playing: bool,
}

/// What one status report sends to the system's Now Playing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NowPlayingUpdate {
    metadata: bool,
    progress: bool,
}

/// What `report` sends to the system's Now Playing, and whether the system still lacks the
/// current item's metadata afterwards; `behind` says whether it lacked it before.
///
/// A platform host that plays the track publishes Now Playing itself, so nothing goes out
/// meanwhile, and the item's metadata goes out once the host lets go. The host lets go while
/// its ending track still reports playing, just before the next track starts or the player
/// stops; that report sends nothing, so the system never shows the ending track again.
/// Metadata that was not found is never sent, and progress waits for the metadata it goes
/// with.
fn now_playing_update(behind: bool, report: NowPlayingReport) -> (bool, NowPlayingUpdate) {
    let behind = report.metadata_found && (behind || report.item_changed || report.hosted);
    if report.hosted {
        return (behind, NowPlayingUpdate::default());
    }
    // A paused hosted track that a paused load replaces is published once, as paused, first.
    let metadata = behind && (report.item_changed || !report.playing);
    let behind = behind && !metadata;
    (
        behind,
        NowPlayingUpdate {
            metadata,
            progress: !behind,
        },
    )
}

/// What a status report changes about who holds the system's Now Playing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NowPlayingHandOff {
    /// A platform host took over the track, and shows Now Playing for it itself: Rune's own
    /// commands and info give way, so that a command does not act twice and Rune's earlier
    /// track does not show over the host's.
    YieldToHost,
    /// The host let go: Rune's own commands come back before its metadata goes out again.
    Reclaim,
}

/// Whether Rune's Now Playing gives way to a host after a report of whether a host plays the
/// track, given whether it did before, and the hand-off that the report makes: one at each
/// change, and none while it stays.
fn now_playing_hand_off(yielded: bool, hosted: bool) -> (bool, Option<NowPlayingHandOff>) {
    match (yielded, hosted) {
        (false, true) => (true, Some(NowPlayingHandOff::YieldToHost)),
        (true, false) => (false, Some(NowPlayingHandOff::Reclaim)),
        _ => (yielded, None),
    }
}

/// Whether a cover or a name that arrives after its track started goes to the system's Now
/// Playing now, given whether Now Playing gave way to a host and whether it still lacks the
/// track's metadata, and whether it lacks the metadata afterwards.
///
/// While a host holds Now Playing, or the metadata waits for the host's ending track to leave,
/// nothing goes out: what arrived goes out with the metadata once that does.
fn late_metadata_update(yielded: bool, behind: bool) -> (bool, bool) {
    let behind = yielded || behind;
    (behind, !behind)
}

/// How long a status report waits for a service to name the track playing.
const STATUS_METADATA_WAIT: Duration = Duration::from_secs(3);

struct ResolvedLibrary {
    library_id: Uuid,
    /// The library's files, which a service library does not have on this node.
    fsio: Option<Arc<FsIo>>,
    media_root: String,
    implementation: Implementation,
}

/// A playing track's cover that was not cached when the track started.
struct LateCover {
    item: PlayingItem,
    cover: String,
}

/// The library of a playing library track, and the track's metadata and cover. A service
/// library has no catalog library on this node, so its binding describes its tracks.
async fn playing_track_library(
    node: &Arc<NodeContext>,
    item: &PlayingItem,
    library_id: Uuid,
    track: &str,
    late_covers: Option<&mpsc::UnboundedSender<LateCover>>,
    late_names: Option<&Arc<Notify>>,
) -> Result<(
    ResolvedLibrary,
    Result<(PlayingItemMetadataSummary, Option<String>)>,
)> {
    let implementation = playback_implementation_for_item(node, library_id).await?;
    if let Implementation::Service(binding) = &implementation {
        let metadata = service_track_metadata(binding, item, track, late_covers, late_names).await;
        let library = ResolvedLibrary {
            library_id,
            fsio: None,
            media_root: String::new(),
            implementation,
        };
        return Ok((library, metadata));
    }
    let library = node.library_pool.acquire(library_id).await?;
    let metadata = implementation.item_metadata(item, true, &library).await;
    let library = ResolvedLibrary {
        library_id: library.library_id,
        fsio: Some(library.fsio.clone()),
        media_root: library.media_root.clone(),
        implementation,
    };
    Ok((library, metadata))
}

/// A service track's metadata, with its cover if the cover is already cached. Otherwise the
/// cover is fetched and sent to `late_covers` once it is cached.
///
/// A service that does not answer in time gives what its binding already holds, so a slow
/// service does not hold the status up. When the binding holds nothing, the track goes unnamed
/// for now, and `late_names` is told once the service has answered, so that the track is named
/// again from what the binding holds then.
async fn service_track_metadata(
    binding: &Arc<dyn ServiceBinding>,
    item: &PlayingItem,
    track: &str,
    late_covers: Option<&mpsc::UnboundedSender<LateCover>>,
    late_names: Option<&Arc<Notify>>,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    let mut lookup = {
        let binding = Arc::clone(binding);
        let track = track.to_owned();
        task::spawn(async move { binding.item_metadata(&track, true).await })
    };
    let (metadata, cover) = match timeout(STATUS_METADATA_WAIT, &mut lookup).await {
        Ok(answer) => answer.context("The lookup of the service track stopped")??,
        Err(_) => {
            if let Some(summary) = binding.cached_summary(track) {
                lookup.abort();
                (summary, None)
            } else {
                match late_names {
                    Some(late_names) => {
                        let binding = Arc::clone(binding);
                        let track = track.to_owned();
                        let late_names = Arc::clone(late_names);
                        task::spawn(async move {
                            // The track is named again only from what the binding holds by
                            // then, so a service that stays slow is not asked over and over.
                            if matches!(lookup.await, Ok(Ok(_)))
                                && binding.cached_summary(&track).is_some()
                            {
                                late_names.notify_one();
                            }
                        });
                    }
                    None => lookup.abort(),
                }
                anyhow::bail!("The service did not answer in time");
            }
        }
    };
    if cover.is_none()
        && let Some(late_covers) = late_covers
    {
        fetch_late_cover(binding, item, track, late_covers);
    }
    Ok((metadata, cover))
}

/// Fetches the cover of a service track whose status went out without it, and sends it to
/// `late_covers` once it is cached.
fn fetch_late_cover(
    binding: &Arc<dyn ServiceBinding>,
    item: &PlayingItem,
    track: &str,
    late_covers: &mpsc::UnboundedSender<LateCover>,
) {
    let binding = Arc::clone(binding);
    let item = item.clone();
    let track = track.to_owned();
    let late_covers = late_covers.clone();
    tokio::spawn(async move {
        match binding.cover_when_cached(&track).await {
            Ok(Some(cover)) => {
                let _ = late_covers.send(LateCover { item, cover });
            }
            Ok(None) => {}
            Err(error) => log::warn!("Could not fetch the playing service cover: {error:#}"),
        }
    });
}

/// Shows a cover that arrived after its track started, if that track is still the one playing
/// and has no cover yet. Returns whether it was shown, so that the status is reported again.
async fn show_late_cover(
    node: &Arc<NodeContext>,
    late: LateCover,
    item_state: &mut ItemEventState,
    controls: &Option<Arc<Mutex<MediaControlManager>>>,
) -> bool {
    if item_state.last_item.as_ref() != Some(&late.item) || item_state.cover.is_some() {
        return false;
    }
    item_state.cover = Some(late.cover);
    let (controls_behind, publish) =
        late_metadata_update(item_state.controls_yielded, item_state.controls_behind);
    item_state.controls_behind = controls_behind;
    if let Some(manager) = controls
        && publish
    {
        let _ = update_media_controls_metadata(
            manager.clone(),
            &item_state.metadata,
            item_state.cover.as_ref(),
        )
        .await;
    }
    playing_cover_changed(node, Some(&late.item), item_state.cover.as_deref());
    true
}

/// Names a library track that went out unnamed, now that its service has answered: from what the
/// binding that looked it up holds by now. The track keeps its place in playback, so it is not
/// started again, and its service is not asked again. A track whose library could not be had is
/// looked up again in full. `now_playing` says whether the caller reports what plays now.
/// Returns whether the track's status is to go out again, named or to be looked up.
async fn name_late_track(
    item_state: &mut ItemEventState,
    controls: &Option<Arc<Mutex<MediaControlManager>>>,
    now_playing: bool,
) -> bool {
    let Some(item) = item_state.last_item.clone() else {
        return false;
    };
    let PlayingItem::LibraryTrack { track, .. } = &item else {
        return false;
    };
    if item_state.metadata_found {
        return false;
    }
    let binding = match &item_state.library {
        Some(ResolvedLibrary {
            implementation: Implementation::Service(binding),
            ..
        }) => Arc::clone(binding),
        Some(_) => return false,
        None => {
            item_state.last_item = None;
            return true;
        }
    };
    let Some(metadata) = binding.cached_summary(track) else {
        return false;
    };
    item_state.metadata = metadata;
    item_state.metadata_found = true;
    item_state.now_playing_pending = now_playing && scrobbled_here(item_state.library.as_ref());
    if let Some(late_covers) = &item_state.late_covers {
        fetch_late_cover(&binding, &item, track, late_covers);
    }
    // While a host holds the system's Now Playing, the name goes out once it lets go.
    let (controls_behind, publish) =
        late_metadata_update(item_state.controls_yielded, item_state.controls_behind);
    item_state.controls_behind = controls_behind;
    if let Some(manager) = controls
        && publish
    {
        let _ = update_media_controls_metadata(
            manager.clone(),
            &item_state.metadata,
            item_state.cover.as_ref(),
        )
        .await;
    }
    true
}

/// Tells the node what this device's player plays now, before the report is made, so that the
/// visualizer and the volume control here do not wait for the metadata lookup when a platform
/// host takes over the track or lets go of it. Only the event loop does this: a re-show reports
/// an older snapshot, which must not land after a newer one.
fn note_local_track(node: &Arc<NodeContext>, status: &PlayerStatus) {
    if let Err(error) = node.note_local_track(
        status.path.as_deref(),
        status.path.is_some() || status.item.is_some(),
    ) {
        log::warn!("Could not publish what plays the track: {error:#}");
    }
}

/// Turns a player status into the GUI report. `now_playing` is `None` when only
/// re-showing the current state (e.g. after renderer control ends).
async fn handle_status_event(
    node: &Arc<NodeContext>,
    value: PlayerStatus,
    item_state: &mut ItemEventState,
    controls: &Option<Arc<Mutex<MediaControlManager>>>,
    now_playing: Option<&mpsc::UnboundedSender<ScrobblingTrack>>,
    broadcaster: &Arc<dyn Broadcaster>,
    session_id: &str,
) {
    let item_changed = value.item != item_state.last_item;
    if item_changed {
        item_state.metadata = PlayingItemMetadataSummary::default();
        item_state.cover = None;
        item_state.library = None;
        item_state.now_playing_pending = false;
        item_state.track_started = false;
        item_state.metadata_found = false;
        if let Some(item) = &value.item {
            let result = match item {
                PlayingItem::LibraryTrack { library_id, track } => {
                    let late_covers = item_state.late_covers.as_ref();
                    let late_names = item_state.late_names.as_ref();
                    match playing_track_library(
                        node,
                        item,
                        *library_id,
                        track,
                        late_covers,
                        late_names,
                    )
                    .await
                    {
                        Ok((library, result)) => {
                            item_state.library = Some(library);
                            result
                        }
                        Err(error) => Err(error),
                    }
                }
                _ => item_metadata(node, item, true).await,
            };
            if let Ok((metadata, cover)) = result {
                item_state.metadata = metadata;
                item_state.cover = cover;
                item_state.metadata_found = true;
                item_state.now_playing_pending =
                    now_playing.is_some() && scrobbled_here(item_state.library.as_ref());
            }
        }
        playing_cover_changed(node, value.item.as_ref(), item_state.cover.as_deref());
        item_state.last_item = value.item.clone();
    }
    if value.state == PlaybackState::Playing
        && item_state.now_playing_pending
        && let Some(now_playing) = now_playing
    {
        item_state.now_playing_pending = false;
        let _ = now_playing.send(metadata_summary_to_scrobbling_track(
            &item_state.metadata,
            Some(SystemTime::now()),
        ));
    }
    let (controls_behind, system_now_playing) = now_playing_update(
        item_state.controls_behind,
        NowPlayingReport {
            item_changed,
            metadata_found: item_state.metadata_found,
            hosted: value.hosted_by.is_some(),
            playing: value.state == PlaybackState::Playing,
        },
    );
    item_state.controls_behind = controls_behind;
    let (controls_yielded, hand_off) =
        now_playing_hand_off(item_state.controls_yielded, value.hosted_by.is_some());
    item_state.controls_yielded = controls_yielded;
    if let (Some(manager), Some(hand_off)) = (controls, hand_off)
        && !item_state.re_show
    {
        let mut manager = manager.lock().await;
        let handed = match hand_off {
            NowPlayingHandOff::YieldToHost => manager.yield_commands(),
            NowPlayingHandOff::Reclaim => manager.reclaim_commands(),
        };
        if let Err(error) = handed {
            log::warn!("Could not hand Now Playing over ({hand_off:?}): {error:#}");
        }
    }
    if let Some(manager) = controls
        && system_now_playing.metadata
    {
        let _ = update_media_controls_metadata(
            manager.clone(),
            &item_state.metadata,
            item_state.cover.as_ref(),
        )
        .await;
    }
    if value.state == PlaybackState::Playing && !item_state.track_started {
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|time| u64::try_from(time.as_millis()).ok());
        if let (Some(start_time), Some(item), Some(library)) =
            (start_time, value.item.as_ref(), item_state.library.as_ref())
        {
            item_state.track_started = true;
            library.implementation.track_started(item, start_time).await;
        }
    }
    let root = item_state
        .library
        .as_ref()
        .map(|library| library.media_root.clone())
        .unwrap_or_default();
    let playing = value.state == PlaybackState::Playing;
    // A track a platform host plays has no file here, and its audio never reaches the player.
    let file_path = value.path.as_deref().filter(|path| !is_hosted_path(path));
    // This device is the source: its player gives the spectrum and the chart is analysed here,
    // whether or not an item is loaded.
    node.visualizer().set_playing(playing);
    node.visualizer().set_playback(
        file_path,
        value.position,
        playing,
        item_state
            .library
            .as_ref()
            .and_then(|library| library.fsio.clone()),
    );
    let ui_library_id = current_ui_library_id(node).await;
    if item_changed || ui_library_id != item_state.request_ui_library {
        item_state.request_ui_library = ui_library_id;
        item_state.request = match &value.item {
            Some(item) => playing_item_requests(from_ref(item), &node.library_pool, ui_library_id)
                .await
                .pop(),
            None => None,
        };
        // The UI hears what the item supports before the status that names it, so that it
        // never offers for a moment what the item cannot do.
        item_capabilities::send(node, item_state, broadcaster.as_ref());
    } else {
        item_capabilities::send_if_untold(node, item_state, broadcaster.as_ref());
    }
    let item_request = item_state.request.clone();
    let report = format_status(
        &value,
        &item_state.metadata,
        item_state.cover.clone(),
        root,
        item_request,
        session_id,
    );
    if let Some(manager) = controls
        && system_now_playing.progress
    {
        let playback = now_playing_playback(&value, item_state.metadata_found);
        let _ = update_media_controls_progress(manager.clone(), playback).await;
    }
    broadcaster.broadcast(&report);

    let attached_controllers = node
        .session()
        .map_or(0, |session| session.attached_controllers().len());
    let controller_set_changed = attached_controllers != item_state.feed_controller_count;
    item_state.feed_controller_count = attached_controllers;
    if attached_controllers == 0 {
        item_state.feed_item = None;
    }
    let feed_item = value.item.as_ref().and_then(playing_item_ref);
    let feed_item_changed = feed_item
        .as_ref()
        .is_some_and(|item| item_state.feed_item.as_ref() != Some(item));
    if attached_controllers > 0
        && (item_changed || controller_set_changed || feed_item_changed)
        && let Some(item) = feed_item
        && let Some(path) = file_path
        && let Some(library) = &item_state.library
        && let Some(library_fsio) = &library.fsio
    {
        item_state.feed_item = Some(item.clone());
        let source = visualizer::ChartSource::LocalFile {
            path: path.to_path_buf(),
            fsio: Some(Arc::clone(library_fsio)),
        };
        let chart = node.visualizer().cached_chart(&source).ok().flatten();
        if let Some(chart) = chart {
            broadcaster.broadcast(&VisualizerFeedChart { item, chart });
        } else {
            let fsio = Arc::clone(library_fsio);
            let path = path.to_path_buf();
            let item = item.clone();
            let broadcaster = Arc::clone(broadcaster);
            let weak_node = Arc::downgrade(node);
            tokio::spawn(async move {
                let chart =
                    task::spawn_blocking(move || visualizer::analyze_chart_file(&fsio, &path))
                        .await;
                let Ok(Ok(chart)) = chart else {
                    return;
                };
                if weak_node
                    .upgrade()
                    .and_then(|node| node.session().ok())
                    .is_some_and(|session| !session.attached_controllers().is_empty())
                {
                    broadcaster.broadcast(&VisualizerFeedChart { item, chart });
                }
            });
        }

        let current_path = item_state.cover.as_deref().map(PathBuf::from);
        let fsio = Arc::clone(library_fsio);
        let library_id = library.library_id;
        let node = Arc::clone(node);
        let item = value.item.as_ref().and_then(playing_item_ref);
        let broadcaster = Arc::clone(broadcaster);
        tokio::spawn(async move {
            let Some(item) = item else { return };
            let Ok(library) = node.library_pool.acquire(library_id).await else {
                return;
            };
            match feed_cover_bytes(
                fsio,
                Arc::clone(&library.main_db),
                Arc::clone(&library.recommend_db),
                Arc::clone(&node),
                library_id,
                current_path,
            )
            .await
            {
                Ok((playing, random))
                    if !node
                        .session()
                        .is_ok_and(|session| session.attached_controllers().is_empty()) =>
                {
                    broadcaster.broadcast(&VisualizerFeedCovers {
                        item,
                        playing,
                        random,
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("Could not prepare visualizer covers for the feed: {error:#}")
                }
            }
        });
    }
}

/// The playback a status gives the system's Now Playing, which shows it against the metadata it
/// was last given. The state always goes, so that a pause or a stop shows at once, even when the
/// item's metadata is never found. The position of an item whose metadata was not found waits
/// until it is, as when a slow service names its track late: it would go out against the item
/// before.
fn now_playing_playback(status: &PlayerStatus, metadata_found: bool) -> MediaPlayback {
    let progress = metadata_found.then_some(MediaPosition(status.position));
    match status.state {
        PlaybackState::Playing => MediaPlayback::Playing { progress },
        PlaybackState::Paused => MediaPlayback::Paused { progress },
        PlaybackState::Stopped => MediaPlayback::Stopped,
    }
}

fn playing_item_ref(item: &PlayingItem) -> Option<PlayingItemRef> {
    match item {
        PlayingItem::LibraryTrack { library_id, track } => Some(PlayingItemRef::LibraryTrack {
            library_id: library_id.to_string(),
            track: track.clone(),
        }),
        PlayingItem::IndependentFile(path) => {
            Some(PlayingItemRef::IndependentFile { path: path.clone() })
        }
        PlayingItem::Online(_, _) | PlayingItem::Unknown => None,
    }
}

async fn handle_playlist_event(
    node: &NodeContext,
    value: PlaylistStatus,
    broadcaster: &dyn Broadcaster,
    session_id: &str,
) {
    send_playlist(node, &value, broadcaster, session_id).await;
}

/// Whether Rune's own scrobblers hear of the track of `library`: not when the library's service
/// counts the plays of its tracks itself.
fn scrobbled_here(library: Option<&ResolvedLibrary>) -> bool {
    !matches!(
        library.map(|library| &library.implementation),
        Some(Implementation::Service(binding)) if binding.service().counts_its_own_plays()
    )
}

async fn handle_played_event(
    node: &NodeContext,
    played: PlayedThrough,
    item_state: &mut ItemEventState,
    scrobbler: &Arc<Mutex<dyn ScrobblingServiceManager>>,
) {
    let PlayedThrough {
        item,
        listened,
        started_at,
    } = played;
    if let Err(error) = record_played(node, &item).await {
        log::warn!("Could not record played item: {error:#}");
    }
    if let Ok((metadata, _)) = item_metadata(node, &item, false).await {
        scrobbler.lock().await.scrobble_all(
            metadata_summary_to_scrobbling_track(&metadata, started_at),
            listened,
        );
    }
    let report = if item_state.last_item.as_ref() == Some(&item)
        && let Some(library) = item_state.library.as_ref()
    {
        library.implementation.track_played_through(&item).await;
        Ok(())
    } else {
        report_item_played_through(node, &item).await
    };
    if let Err(error) = report {
        log::warn!("Could not report library track completion: {error:#}");
    }
    // The occurrence of the item ended, so the next one starts and reports anew: Repeat One
    // and adjacent entries of the same track report the item again without a change of item,
    // which the start flag and the pending now-playing report must not hold against it. The
    // completion above already consumed what the service stored for this start, so a status of
    // this occurrence still in flight after it cannot start again.
    item_state.track_started = false;
    item_state.now_playing_pending = true;
}

/// A playback failure reaches the application log, and when no request answer already carried
/// it, also reaches the user as a typed request failure: nothing else reports a playback that
/// stops on its own. The first failure of a chain through broken tracks is reported, and its
/// continuations stay in the log, so a queue of broken tracks does not bury the user in reports.
async fn handle_log_event(node: &NodeContext, error: InternalLog, broadcaster: &dyn Broadcaster) {
    let detail = error.error.clone();
    if let Some(item) = &error.item {
        let _ = item_log(node, item, error.domain.clone(), detail.clone()).await;
    }
    if error.answered || error.chain_continuation {
        return;
    }
    let kind = match error.failure {
        Some(UnavailableReason::AccessRevoked | UnavailableReason::CredentialsRejected) => {
            RequestErrorKind::PermissionDenied
        }
        Some(UnavailableReason::TrackGone) => RequestErrorKind::NotFound,
        _ => RequestErrorKind::Unavailable,
    };
    broadcaster.broadcast(&RequestFailedResponse {
        request: "Playback".into(),
        call_id: None,
        kind,
        detail,
        cause: None,
    });
}

async fn handle_scrobbler_error(node: Option<Arc<NodeContext>>, error: ScrobblingError) {
    let detail = format_scrobbler_error(&error);
    log::warn!("{detail}");
    if let (Some(node), Some(library_id)) = (node, error.library_id.clone()) {
        if let Ok(library_id) = library_id.parse() {
            let domain = format!("scrobbler::{:?}::{:?}", error.action, error.service);
            let _ = library_log(&node, library_id, domain, detail).await;
        } else {
            log::warn!("Scrobbler error with invalid library context");
        }
    } else {
        log::warn!("Scrobbler error without catalog context");
    }
}

fn format_scrobbler_error(error: &ScrobblingError) -> String {
    format!(
        "Scrobbler {:?} for {} failed: {:#}",
        error.action, error.service, error.error
    )
}

fn handle_login_event(value: Vec<LoginStatus>, broadcaster: &dyn Broadcaster) {
    broadcaster.broadcast(&ScrobbleServiceStatusUpdated {
        services: value
            .into_iter()
            .map(|status| ScrobbleServiceStatus {
                service_id: status.service.to_string(),
                is_available: status.is_available,
                error: status.error_message,
            })
            .collect(),
    });
}

async fn item_metadata(
    node: &NodeContext,
    item: &PlayingItem,
    with_cover: bool,
) -> Result<(PlayingItemMetadataSummary, Option<String>)> {
    if matches!(item, PlayingItem::LibraryTrack { .. }) {
        return library_item_metadata(node, item, with_cover).await;
    }
    let dispatcher = PlayingItemActionDispatcher::new();
    let items = from_ref(item);
    {
        #[cfg(not(target_os = "android"))]
        let fsio = FsIo::new();
        #[cfg(target_os = "android")]
        let fsio = FsIo::new(Path::new(".rune/.android-fs.db"), "")?;
        let db = MainDbConnection::default();
        let metadata = dispatcher
            .get_metadata_summary(&fsio, &db, items)
            .await?
            .into_iter()
            .next()
            .context("File metadata unavailable")?;
        let cover = if with_cover {
            dispatcher
                .bake_cover_art(&fsio, &"", &db, items)
                .await?
                .remove(item)
        } else {
            None
        };
        Ok((metadata, cover))
    }
}

async fn record_played(node: &NodeContext, item: &PlayingItem) -> Result<()> {
    record_library_item_played(node, item).await
}

async fn item_log(
    node: &NodeContext,
    item: &PlayingItem,
    domain: String,
    detail: String,
) -> Result<()> {
    if let PlayingItem::LibraryTrack { library_id, .. } = item {
        library_log(node, *library_id, domain, detail).await?;
    }
    Ok(())
}

async fn library_log(
    node: &NodeContext,
    library_id: Uuid,
    domain: String,
    detail: String,
) -> Result<()> {
    let library = node.library_pool.acquire(library_id).await?;
    let tx = library.main_db.begin().await?;
    insert_log(&tx, LogLevel::Error, domain, detail).await?;
    tx.commit().await?;
    Ok(())
}

async fn send_playlist(
    node: &NodeContext,
    playlist: &PlaylistStatus,
    broadcaster: &dyn Broadcaster,
    session_id: &str,
) {
    let revision = playlist.revision;
    let mut independent_items = Vec::new();
    for entry in &playlist.items {
        let item = &entry.item;
        match item {
            PlayingItem::LibraryTrack { .. } => {}
            _ => independent_items.push(item.clone()),
        }
    }

    let dispatcher = PlayingItemActionDispatcher::new();
    let mut metadata = HashMap::<PlayingItem, PlayingItemMetadataSummary>::new();
    for entry in &playlist.items {
        let PlayingItem::LibraryTrack { library_id, track } = &entry.item else {
            continue;
        };
        // A service library whose binding this device keeps names its rows from what the
        // binding holds, without a request each.
        if let Some(binding) = node.services.held(LibraryRecordId(*library_id)) {
            if let Some(summary) = binding.cached_summary(track) {
                metadata.insert(entry.item.clone(), summary);
            }
            continue;
        }
        if let Ok((summary, _)) = library_item_metadata(node, &entry.item, false).await {
            metadata.insert(entry.item.clone(), summary);
        }
    }

    if !independent_items.is_empty() {
        #[cfg(not(target_os = "android"))]
        let fsio_result = Ok::<_, anyhow::Error>(FsIo::new());
        #[cfg(target_os = "android")]
        let fsio_result = FsIo::new(Path::new(".rune/.android-fs.db"), "");
        let db = MainDbConnection::default();
        if let Ok(fsio) = fsio_result
            && let Ok(summaries) = dispatcher
                .get_metadata_summary(&fsio, &db, &independent_items)
                .await
        {
            metadata.extend(
                summaries
                    .into_iter()
                    .map(|summary| (summary.item.clone(), summary)),
            );
        }
    }

    let ui_library_id = current_ui_library_id(node).await;
    let queue_items = playlist
        .items
        .iter()
        .map(|entry| entry.item.clone())
        .collect::<Vec<_>>();
    let item_requests =
        playing_item_requests(&queue_items, &node.library_pool, ui_library_id).await;
    let items = playlist
        .items
        .iter()
        .zip(item_requests)
        .map(|(entry, item_request)| {
            let item = &entry.item;
            let summary = metadata.get(item);
            let unavailable = playlist
                .unavailable
                .get(item)
                .copied()
                .or_else(|| summary.is_none().then_some(UnavailableReason::Unreadable))
                .map(PlaylistUnavailableReason::from);
            let metadata = summary.cloned().unwrap_or_default();
            PlaylistItem {
                item: item_request,
                artist: metadata.artist,
                album: metadata.album,
                title: metadata.title,
                duration: metadata.duration,
                unavailable,
            }
        })
        .collect();
    broadcaster.broadcast(&PlaylistUpdate {
        items,
        session_id: session_id.to_string(),
        revision,
    });
}

fn format_status(
    status: &PlayerStatus,
    metadata: &PlayingItemMetadataSummary,
    cover_art_path: Option<String>,
    lib_path: String,
    item_request: Option<PlayingItemRef>,
    session_id: &str,
) -> PlaybackStatus {
    PlaybackStatus {
        session_id: session_id.to_string(),
        state: status.state.clone().into(),
        progress_seconds: status.position.as_secs_f32(),
        progress_percentage: if metadata.duration > 0.0 {
            status.position.as_secs_f32() / metadata.duration as f32
        } else {
            0.0
        },
        artist: Some(metadata.artist.clone()),
        album: Some(metadata.album.clone()),
        title: Some(metadata.title.clone()),
        duration: metadata.duration,
        index: status.index.map(|i| i as i32),
        item: status.item.clone().map(|item| item.to_string()),
        item_request,
        playback_mode: status.playback_mode.into(),
        ready: status.ready,
        cover_art_path,
        lib_path,
    }
}

async fn current_ui_library_id(node: &NodeContext) -> Option<Uuid> {
    let params = node.current_ui().await.ok()?;
    params
        .ui_library
        .lock()
        .await
        .as_ref()
        .map(|library| library.library_id)
}

impl Drop for PlayerSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        for task in self.tasks.get_mut().drain(..) {
            task.abort();
        }
    }
}

#[cfg(test)]
mod system_now_playing_tests {
    use super::{
        NowPlayingHandOff, NowPlayingReport, NowPlayingUpdate, late_metadata_update,
        now_playing_hand_off, now_playing_update,
    };

    const METADATA_AND_PROGRESS: NowPlayingUpdate = NowPlayingUpdate {
        metadata: true,
        progress: true,
    };
    const PROGRESS: NowPlayingUpdate = NowPlayingUpdate {
        metadata: false,
        progress: true,
    };
    const NOTHING: NowPlayingUpdate = NowPlayingUpdate {
        metadata: false,
        progress: false,
    };

    fn report(
        item_changed: bool,
        metadata_found: bool,
        hosted: bool,
        playing: bool,
    ) -> NowPlayingReport {
        NowPlayingReport {
            item_changed,
            metadata_found,
            hosted,
            playing,
        }
    }

    /// The updates a run of reports sends, starting from a system that shows nothing of ours.
    fn updates(reports: &[NowPlayingReport]) -> Vec<NowPlayingUpdate> {
        let mut behind = false;
        reports
            .iter()
            .map(|report| {
                let (now_behind, update) = now_playing_update(behind, *report);
                behind = now_behind;
                update
            })
            .collect()
    }

    #[test]
    fn a_local_track_publishes_its_metadata_once_and_then_its_progress() {
        assert_eq!(
            updates(&[
                report(true, true, false, true),
                report(false, true, false, true)
            ]),
            [METADATA_AND_PROGRESS, PROGRESS]
        );
    }

    #[test]
    fn nothing_goes_out_while_hosted_and_the_ending_track_is_not_published_again() {
        assert_eq!(
            updates(&[
                // A local track plays, then the host takes over a hosted track.
                report(true, true, false, true),
                report(false, true, true, true),
                report(true, true, true, true),
                report(false, true, true, false),
                // The host lets go while its track still reports playing, then a local track
                // starts.
                report(false, true, false, true),
                report(true, true, false, true),
                report(false, true, false, true),
            ]),
            [
                METADATA_AND_PROGRESS,
                NOTHING,
                NOTHING,
                NOTHING,
                NOTHING,
                METADATA_AND_PROGRESS,
                PROGRESS,
            ]
        );
    }

    #[test]
    fn a_host_that_lets_go_of_a_paused_track_has_it_published_once() {
        assert_eq!(
            updates(&[
                report(true, true, true, true),
                report(false, true, false, false),
                report(false, true, false, false),
            ]),
            [NOTHING, METADATA_AND_PROGRESS, PROGRESS]
        );
    }

    /// The hand-offs that a run of reports of whether a host plays the track makes, starting
    /// from Rune holding Now Playing.
    fn hand_offs(hosted: &[bool]) -> Vec<Option<NowPlayingHandOff>> {
        let mut yielded = false;
        hosted
            .iter()
            .map(|hosted| {
                let (now_yielded, hand_off) = now_playing_hand_off(yielded, *hosted);
                yielded = now_yielded;
                hand_off
            })
            .collect()
    }

    #[test]
    fn rune_gives_now_playing_to_a_host_once_and_takes_it_back_once() {
        use NowPlayingHandOff::{Reclaim, YieldToHost};
        assert_eq!(
            hand_offs(&[false, true, true, true, false, false, true, false]),
            [
                None,
                Some(YieldToHost),
                None,
                None,
                Some(Reclaim),
                None,
                Some(YieldToHost),
                Some(Reclaim),
            ]
        );
    }

    #[test]
    fn a_late_cover_or_name_waits_while_a_host_holds_now_playing() {
        // Rune holds Now Playing and showed the track, so the cover goes out at once.
        assert_eq!(late_metadata_update(false, false), (false, true));
        // A host holds Now Playing: nothing goes out over what the host shows.
        assert_eq!(late_metadata_update(true, true), (true, false));
        assert_eq!(late_metadata_update(true, false), (true, false));
        // The host let go while its track still plays, which is not published again.
        assert_eq!(late_metadata_update(false, true), (true, false));

        // The cover then goes out with the metadata, once the host lets go of a paused track.
        let (behind, _) = late_metadata_update(true, true);
        let (_, update) = now_playing_update(behind, report(false, true, false, false));
        assert_eq!(update, METADATA_AND_PROGRESS);
    }

    #[test]
    fn metadata_that_was_not_found_is_never_published() {
        assert_eq!(
            updates(&[
                report(true, false, true, true),
                report(false, false, false, false),
                report(true, false, false, true),
            ]),
            [NOTHING, PROGRESS, PROGRESS]
        );
        // Nor later, when the next item has none either.
        assert_eq!(
            updates(&[
                report(true, true, true, true),
                report(true, false, false, true)
            ]),
            [NOTHING, PROGRESS]
        );
    }
}

#[cfg(test)]
mod scrobbled_here_tests {
    use std::sync::Arc;

    use database::library_registry::ServiceKind;
    use database::playing_item::PlayingItemMetadataSummary;
    use playback::player::ServiceTarget;
    use uuid::Uuid;

    use super::{ResolvedLibrary, scrobbled_here};
    use crate::macros::{RequestSlots, RequestTranslator};
    use crate::request_owner::Implementation;
    use crate::service::{ServiceBinding, ServiceFuture};

    /// A binding of a library of the service that answers nothing.
    struct Bound(ServiceKind);

    impl RequestTranslator for Bound {}

    impl ServiceBinding for Bound {
        fn library_id(&self) -> Uuid {
            Uuid::nil()
        }

        fn slots(&self) -> RequestSlots {
            RequestSlots::unimplemented()
        }

        fn service(&self) -> ServiceKind {
            self.0
        }

        fn item_metadata<'a>(
            &'a self,
            _track: &'a str,
            _with_cover: bool,
        ) -> ServiceFuture<'a, (PlayingItemMetadataSummary, Option<String>)> {
            Box::pin(async { anyhow::bail!("no metadata") })
        }

        fn visualizer_covers<'a>(
            &'a self,
            _track: &'a str,
        ) -> ServiceFuture<'a, (Option<String>, Vec<String>)> {
            Box::pin(async { anyhow::bail!("no covers") })
        }

        fn playback_source<'a>(&'a self, _track: &'a str) -> ServiceFuture<'a, ServiceTarget> {
            Box::pin(async { anyhow::bail!("no playback") })
        }
    }

    fn resolved(implementation: Implementation) -> ResolvedLibrary {
        ResolvedLibrary {
            library_id: Uuid::nil(),
            fsio: None,
            media_root: String::new(),
            implementation,
        }
    }

    #[test]
    fn rune_scrobblers_leave_the_tracks_of_a_service_that_counts_its_plays_to_it() {
        let service = |kind| resolved(Implementation::Service(Arc::new(Bound(kind))));
        assert!(!scrobbled_here(Some(&service(ServiceKind::AppleMusic))));
        // A Subsonic track goes to Rune's scrobblers as before, and so does a local one.
        assert!(scrobbled_here(Some(&service(ServiceKind::Subsonic))));
        assert!(scrobbled_here(Some(&resolved(Implementation::Local))));
        assert!(scrobbled_here(None));
    }
}

#[cfg(test)]
mod scrobbler_error_tests {
    use anyhow::anyhow;
    use scrobbling::manager::{ActionType, ScrobblingError, ScrobblingService};

    use super::format_scrobbler_error;

    #[test]
    fn scrobbler_error_log_keeps_context_and_sanitized_cause() {
        let error = ScrobblingError {
            library_id: None,
            service: ScrobblingService::ListenBrainz,
            action: ActionType::Authenticate,
            error: anyhow!("token rejected: <redacted>; status=401"),
        };

        let detail = format_scrobbler_error(&error);

        assert!(detail.contains("Authenticate"));
        assert!(detail.contains("ListenBrainz"));
        assert!(detail.contains("token rejected"));
        assert!(detail.contains("status=401"));
        assert!(!detail.contains("secret-token-value"));
    }
}

#[cfg(test)]
mod test_access {
    use super::*;

    impl PlayerSession {
        pub(crate) fn persistence_write_count(&self) -> usize {
            self.persistence_writes.load(Ordering::Acquire)
        }

        pub(crate) async fn playback_owner_for_test(&self) -> Option<StoredPlaybackOwner> {
            self.state.lock().await.playback_owner.clone()
        }
    }
}

#[cfg(test)]
mod now_playing_tests;
