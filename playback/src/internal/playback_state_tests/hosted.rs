use super::*;
use crate::hosted::HostFailure;
use crate::player::{
    AudioRequest, LibraryPlaybackSource, Playable, PlaybackState, Player, RemoteAudioSource,
};
use futures::future::BoxFuture;
use simple_channel::SimpleReceiver;
use std::sync::atomic::{AtomicBool, AtomicU64};

const HOST: &str = "host";
// Keys that start with "other-" belong to this host.
const OTHER: &str = "otherHost";

// A host's queue, as the host contract describes it.
#[derive(Default)]
struct Model {
    queue: Vec<String>,
    current: Option<usize>,
    playing: bool,
    position_ms: u64,
    revision: u64,
    // What the player sent that the contract does not allow.
    errors: Vec<String>,
}

impl Model {
    fn apply(&mut self, command: &HostCommand) {
        if command.revision <= self.revision {
            self.errors.push(format!(
                "revision {} after {}",
                command.revision, self.revision
            ));
        }
        self.revision = command.revision;
        for action in &command.actions {
            if let Err(error) = self.act(action) {
                self.errors.push(format!(
                    "{action:?} on {:?} at {:?}: {error}",
                    self.queue, self.current
                ));
            }
        }
    }

    fn act(&mut self, action: &HostAction) -> std::result::Result<(), &'static str> {
        match action {
            HostAction::Set { ids } => {
                if ids.is_empty() || ids.len() > RUN_LIMIT + 1 {
                    return Err("a set holds the current track and at most a run more");
                }
                self.queue = ids.clone();
                self.current = Some(0);
                self.position_ms = 0;
            }
            HostAction::Add { ids } => {
                if ids.len() != 1 {
                    return Err("an add carries one id");
                }
                if self.current.is_none() {
                    return Err("an add to a host that holds nothing");
                }
                self.queue.extend(ids.iter().cloned());
            }
            HostAction::Remove { index } => {
                let index = *index as usize;
                // A host never removes the entry it plays.
                if Some(index) == self.current {
                    return Ok(());
                }
                if !self.current.is_some_and(|current| index > current)
                    || index + 1 != self.queue.len()
                {
                    return Err("only the last entry, after the current one, is removed");
                }
                self.queue.pop();
            }
            HostAction::Skip { index, offset_ms } => {
                let index = *index as usize;
                if index >= self.queue.len() {
                    return Err("a skip past the end of the queue");
                }
                self.current = Some(index);
                self.position_ms = *offset_ms;
            }
            HostAction::Start => self.playing = true,
            HostAction::Stop => self.playing = false,
            HostAction::Clear => {
                self.queue.clear();
                self.current = None;
                self.playing = false;
            }
            HostAction::SetGain { .. } => {}
        }
        Ok(())
    }

    fn status(&self, host: &str) -> HostEvent {
        HostEvent::JukeboxStatus {
            host: host.to_owned(),
            revision: self.revision,
            current_index: self.current.map(|current| u32::try_from(current).unwrap()),
            current_id: self.current.map(|current| self.queue[current].clone()),
            playing: self.playing,
            position_ms: self.position_ms,
        }
    }
}

// A host that records the batches it is sent, and applies them to a model of its queue.
struct FakeHost {
    sent: mpsc::UnboundedSender<HostCommand>,
    received: tokio::sync::Mutex<mpsc::UnboundedReceiver<HostCommand>>,
    models: Mutex<HashMap<String, Model>>,
}

impl HostBridge for FakeHost {
    fn send(&self, command: HostCommand) {
        self.models
            .lock()
            .unwrap()
            .entry(command.host.clone())
            .or_default()
            .apply(&command);
        let _ = self.sent.send(command);
    }
}

impl FakeHost {
    fn new() -> Arc<Self> {
        let (sent, received) = mpsc::unbounded_channel();
        Arc::new(Self {
            sent,
            received: tokio::sync::Mutex::new(received),
            models: Mutex::default(),
        })
    }

    // Waits until the host was sent at least `count` batches since the last take, and
    // returns all of them.
    async fn take(&self, count: usize) -> Result<Vec<HostCommand>> {
        let mut received = self.received.lock().await;
        let mut batches = Vec::new();
        let waited = tokio::time::timeout(TEST_DEADLINE, async {
            while batches.len() < count {
                match received.recv().await {
                    Some(batch) => batches.push(batch),
                    None => break,
                }
            }
        })
        .await;
        anyhow::ensure!(
            waited.is_ok() && batches.len() >= count,
            "the host was sent only {batches:?}"
        );
        while let Ok(batch) = received.try_recv() {
            batches.push(batch);
        }
        Ok(batches)
    }

    fn sent_nothing(&self) -> bool {
        self.received
            .try_lock()
            .is_ok_and(|received| received.is_empty())
    }

    fn model<T>(&self, f: impl FnOnce(&mut Model) -> T) -> T {
        f(self
            .models
            .lock()
            .unwrap()
            .entry(HOST.to_owned())
            .or_default())
    }

    // What the main host reports as it stands.
    fn status(&self) -> HostEvent {
        self.model(|model| model.status(HOST))
    }

    // The main host moves on to its next entry by itself, and reports so.
    fn move_on(&self) -> HostEvent {
        self.model(|model| {
            let next = model.current.map_or(0, |current| current + 1);
            assert!(
                next < model.queue.len(),
                "the host holds nothing after {:?} in {:?}",
                model.current,
                model.queue
            );
            model.current = Some(next);
            model.position_ms = 0;
            model.status(HOST)
        })
    }

    // The main host moves on by `steps` entries by itself, as it does while the player
    // cannot hear from it, and reports where it stands once it can.
    fn move_on_by(&self, steps: usize) -> HostEvent {
        self.model(|model| {
            let next = model.current.map_or(0, |current| current + steps);
            assert!(
                next < model.queue.len(),
                "the host holds nothing {steps} after {:?} in {:?}",
                model.current,
                model.queue
            );
            model.current = Some(next);
            model.position_ms = 0;
            model.status(HOST)
        })
    }

    // The main host's last entry finishes, and it reports the end of its queue.
    fn end(&self) -> HostEvent {
        self.model(|model| {
            assert!(
                model
                    .current
                    .is_some_and(|current| current + 1 == model.queue.len()),
                "the host holds more after {:?} in {:?}",
                model.current,
                model.queue
            );
            model.playing = false;
            HostEvent::JukeboxEnded {
                host: HOST.to_owned(),
                revision: model.revision,
                index: model.current.map(|current| u32::try_from(current).unwrap()),
            }
        })
    }

    // The main host plays on through every entry it holds, while the player cannot hear
    // from it, and reports the end of its queue once it can.
    fn run_out(&self) -> HostEvent {
        self.model(|model| {
            model.current = model.queue.len().checked_sub(1);
            model.position_ms = 0;
        });
        self.end()
    }

    // The main host restarts, holding nothing, and says that it is ready.
    fn restart(&self) -> HostEvent {
        self.model(|model| {
            model.queue.clear();
            model.current = None;
            model.playing = false;
        });
        ready(HOST, true, true)
    }

    // The ids in the main host's queue, and the id of its current entry.
    fn queue(&self) -> (Vec<String>, Option<String>) {
        self.model(|model| {
            (
                model.queue.clone(),
                model.current.map(|current| model.queue[current].clone()),
            )
        })
    }

    // Fails if the player sent anything that the host contract does not allow.
    fn check(&self) -> Result<()> {
        let models = self.models.lock().unwrap();
        let errors = models
            .iter()
            .flat_map(|(host, model)| {
                model
                    .errors
                    .iter()
                    .map(move |error| format!("{host}: {error}"))
            })
            .collect::<Vec<_>>();
        anyhow::ensure!(errors.is_empty(), "{errors:#?}");
        Ok(())
    }
}

// A request that is never made: the streams in these tests are only looked at.
struct NoRequest;

impl AudioRequest for NoRequest {
    fn get(&self, _: Option<(u64, Option<u64>)>) -> BoxFuture<'static, Result<reqwest::Response>> {
        Box::pin(async { anyhow::bail!("no request in these tests") })
    }
}

// How the resolver answers about keys that start with "slow-": after `delay_ms`, and
// whether it answered, or the question was dropped first, which `on_drop` is told.
#[derive(Default)]
struct Slow {
    delay_ms: AtomicU64,
    answered: AtomicBool,
    dropped: AtomicBool,
    on_drop: Notify,
}

// Marks a slow question as dropped, answered or not.
struct Dropped(Arc<Slow>);

impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.dropped.store(true, Ordering::SeqCst);
        self.0.on_drop.notify_one();
    }
}

// Tracks of `hosted_library` are hosted, and the others are local tracks of `local`. Of
// the hosted tracks, keys that start with "other-" belong to another host, "stream-" are
// streamed, "error-" cannot be resolved, and "slow-" take a while. While `down` is set, the
// service cannot be reached, and none of its tracks can be resolved.
struct MixedResolver {
    local: Arc<DelayedResolver>,
    hosted_library: Uuid,
    slow: Arc<Slow>,
    down: Arc<AtomicBool>,
    // The cancellation of the streams the resolver handed out, which it sends on.
    streams: mpsc::UnboundedSender<CancellationToken>,
    // Told when the resolver fails on a key.
    failed: Arc<Notify>,
}

impl PlaybackSourceResolver for MixedResolver {
    fn resolve_track(
        &self,
        library_id: String,
        track: String,
    ) -> BoxFuture<'static, Result<LibraryTrackKey>> {
        if library_id == self.hosted_library.to_string() {
            return Box::pin(async move { Ok(LibraryTrackKey::Service(track)) });
        }
        self.local.resolve_track(library_id, track)
    }

    fn for_library(
        &self,
        library_id: String,
    ) -> BoxFuture<'static, Result<Option<LibraryPlaybackSource>>> {
        self.local.for_library(library_id)
    }

    fn resolve(&self, id: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        self.local.resolve(id)
    }

    fn resolve_service(&self, item: ServiceItem) -> BoxFuture<'static, Result<ServiceTarget>> {
        let key = item.track;
        if self.down.load(Ordering::SeqCst) {
            return Box::pin(async { Err(RemoteAudioError::LibraryUnavailable.into()) });
        }
        if key.starts_with("error-") {
            let failed = self.failed.clone();
            return Box::pin(async move {
                failed.notify_one();
                anyhow::bail!("no track {key}")
            });
        }
        if key.starts_with("stream-") {
            let cancellation = CancellationToken::new();
            let _ = self.streams.send(cancellation.clone());
            return Box::pin(async move {
                Ok(ServiceTarget::Stream(RemoteAudioSource {
                    request: Arc::new(NoRequest),
                    url: "https://stream.invalid/".to_owned(),
                    extension: "mp3".to_owned(),
                    size: 0,
                    cancellation,
                    failure_reason: Arc::new(Mutex::new(None)),
                }))
            });
        }
        let host = if key.starts_with("other-") {
            OTHER
        } else {
            HOST
        };
        let slow = key.starts_with("slow-").then(|| self.slow.clone());
        Box::pin(async move {
            if let Some(slow) = slow {
                let _dropped = Dropped(slow.clone());
                let delay = Duration::from_millis(slow.delay_ms.load(Ordering::SeqCst));
                tokio::time::sleep(delay).await;
                slow.answered.store(true, Ordering::SeqCst);
            }
            Ok(ServiceTarget::Hosted(HostedTrack {
                host: host.to_owned(),
                key,
            }))
        })
    }
}

struct Fixture {
    host: Arc<FakeHost>,
    commands: mpsc::UnboundedSender<PlayerCommand>,
    events: mpsc::UnboundedReceiver<PlayerEvent>,
    local_library: Uuid,
    local_uuids: Vec<Uuid>,
    hosted_library: Uuid,
    started: Arc<Notify>,
    release: Arc<Notify>,
    slow: Arc<Slow>,
    down: Arc<AtomicBool>,
    streams: mpsc::UnboundedReceiver<CancellationToken>,
    failed: Arc<Notify>,
    wav_path: PathBuf,
}

impl Fixture {
    fn local(&self, index: usize) -> PlayingItem {
        library_item(self.local_library, self.local_uuids[index])
    }

    fn hosted(&self, key: &str) -> PlayingItem {
        PlayingItem::LibraryTrack {
            library_id: self.hosted_library,
            track: key.to_owned(),
        }
    }

    fn queue(&self, items: &[PlayingItem]) -> Result<()> {
        self.send(PlayerCommand::AddToPlaylist {
            tracks: items
                .iter()
                .map(|item| (item.clone(), PlaybackSource::LibraryStream))
                .collect(),
            mode: AddMode::AppendToEnd,
            revision: None,
        })
    }

    fn send(&self, command: PlayerCommand) -> Result<()> {
        self.commands.send(command)?;
        Ok(())
    }

    fn report(&self, event: HostEvent) -> Result<()> {
        self.send(PlayerCommand::Hosted(event))
    }

    // Waits until the player has handled every command sent so far, and returns what it
    // reported meanwhile, which callers check rather than drop. It does not wait for the
    // lookups of the next track that those commands started.
    async fn settle(&mut self) -> Result<Vec<PlayerEvent>> {
        let (response, _) = oneshot::channel();
        self.send(PlayerCommand::StatusAfterPending(response))?;
        until(&mut self.events, |event| {
            matches!(event, PlayerEvent::StatusAfterPending(_))
        })
        .await
    }

    // Drops the events so far, and fails on any that marks a track unavailable.
    fn assert_all_available(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            assert!(!matches!(event, PlayerEvent::Unavailable(..)), "{event:?}");
        }
    }
}

// An engine whose three local tracks have `samples` samples each, and whose local track
// ids in `blocked` load only once released. A fake host plays the hosted ones.
//
// The host holds one track ahead, as these tests are about the track right after the
// current one; `run_engine` gives it runs as long as the player makes them.
fn hosted_engine(samples: u32, blocked: &[i32]) -> (PlayerInternal, Fixture) {
    let (internal, fixture) = engine_with_host(samples, blocked);
    (internal.with_host_run_limit(1), fixture)
}

// An engine as `hosted_engine` makes, whose host holds whole runs ahead.
fn run_engine() -> (PlayerInternal, Fixture) {
    engine_with_host(2, &[])
}

fn engine_with_host(samples: u32, blocked: &[i32]) -> (PlayerInternal, Fixture) {
    let local_uuids = (0..3).map(|_| Uuid::new_v4()).collect::<Vec<_>>();
    let started = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (local, wav_path) = delayed_resolver_fixture_with_samples(
        &local_uuids,
        Arc::new(Mutex::new(Vec::new())),
        started.clone(),
        release.clone(),
        blocked,
        samples,
    );
    let hosted_library = Uuid::new_v4();
    let slow = Arc::new(Slow::default());
    let down = Arc::new(AtomicBool::new(false));
    let (stream_sender, streams) = mpsc::unbounded_channel();
    let failed = Arc::new(Notify::new());
    let host = FakeHost::new();
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let (event_sender, events) = mpsc::unbounded_channel();
    let internal = PlayerInternal::new(
        commands,
        event_sender,
        CancellationToken::new(),
        commands_sender.clone(),
        Arc::new(FsIo::new_noop()),
        PcmTap::new(),
        Arc::new(RwLock::new(Some(Arc::new(MixedResolver {
            local,
            hosted_library,
            slow: slow.clone(),
            down: down.clone(),
            streams: stream_sender,
            failed: failed.clone(),
        })))),
        Arc::new(TestOutputs::default()),
    )
    .with_host_bridge(Arc::new(RwLock::new(Some(host.clone()))));
    let fixture = Fixture {
        host,
        commands: commands_sender,
        events,
        local_library: Uuid::new_v4(),
        local_uuids,
        hosted_library,
        started,
        release,
        slow,
        down,
        streams,
        failed,
        wav_path,
    };
    (internal, fixture)
}

// Runs `test` against a running engine, and checks what the host was sent.
async fn with_engine(
    internal: PlayerInternal,
    mut f: Fixture,
    test: impl AsyncFnOnce(&mut Fixture) -> Result<()>,
) -> Result<()> {
    let mut internal = internal;
    LocalSet::new()
        .run_until(async move {
            let runner = tokio::task::spawn_local(async move { internal.run().await });
            let result = test(&mut f).await;
            runner.abort();
            let _ = std::fs::remove_file(&f.wav_path);
            result.and_then(|()| f.host.check())
        })
        .await
}

fn batch(host: &str, revision: u64, actions: Vec<HostAction>) -> HostCommand {
    HostCommand {
        host: host.to_owned(),
        revision,
        actions,
    }
}

// A batch of one action for the main host.
fn on(revision: u64, action: HostAction) -> HostCommand {
    batch(HOST, revision, vec![action])
}

fn hand_over(
    host: &str,
    keys: &[&str],
    revision: u64,
    offset_ms: u64,
    gain: f32,
    play: bool,
) -> HostCommand {
    batch(
        host,
        revision,
        vec![
            HostAction::Set {
                ids: keys.iter().map(|key| (*key).to_owned()).collect(),
            },
            HostAction::Skip {
                index: 0,
                offset_ms,
            },
            HostAction::SetGain { gain },
            if play {
                HostAction::Start
            } else {
                HostAction::Stop
            },
        ],
    )
}

// A track handed to the main host to play from its start, at full volume.
fn handed_over(key: &str, revision: u64) -> HostCommand {
    hand_over(HOST, &[key], revision, 0, 1.0, true)
}

// What the main host is sent when its tracks stop being current.
fn released(revision: u64) -> HostCommand {
    batch(HOST, revision, vec![HostAction::Stop, HostAction::Clear])
}

// The main host gets `key` as the track after its current one.
fn added(revision: u64, key: &str) -> HostCommand {
    on(
        revision,
        HostAction::Add {
            ids: vec![key.to_owned()],
        },
    )
}

// The main host loses the entry at `index`.
fn removed(revision: u64, index: u32) -> HostCommand {
    on(revision, HostAction::Remove { index })
}

// The main host skips to the entry at `index`, and plays it from its start.
fn skipped_to(revision: u64, index: u32) -> HostCommand {
    batch(
        HOST,
        revision,
        vec![
            HostAction::Skip {
                index,
                offset_ms: 0,
            },
            HostAction::Start,
        ],
    )
}

// A status of the main host about track 1440 at index 0.
fn status(revision: u64, playing: bool, position_ms: u64) -> HostEvent {
    status_of(revision, "1440", playing, position_ms)
}

// A status of the main host about track `id` at index 0.
fn status_of(revision: u64, id: &str, playing: bool, position_ms: u64) -> HostEvent {
    HostEvent::JukeboxStatus {
        host: HOST.to_owned(),
        revision,
        current_index: Some(0),
        current_id: Some(id.to_owned()),
        playing,
        position_ms,
    }
}

fn lost(revision: u64) -> HostEvent {
    HostEvent::JukeboxStatus {
        host: HOST.to_owned(),
        revision,
        current_index: None,
        current_id: None,
        playing: false,
        position_ms: 0,
    }
}

// An end as a host that reports every move as it happens may send it, without an index.
fn ended(revision: u64) -> HostEvent {
    HostEvent::JukeboxEnded {
        host: HOST.to_owned(),
        revision,
        index: None,
    }
}

fn failed_at(revision: u64, index: u32, reason: HostFailure) -> HostEvent {
    HostEvent::JukeboxItemFailed {
        host: HOST.to_owned(),
        revision,
        index,
        reason,
    }
}

fn failed(revision: u64, reason: HostFailure) -> HostEvent {
    failed_at(revision, 0, reason)
}

fn ready(host: &str, authorized: bool, can_play: bool) -> HostEvent {
    HostEvent::HostReady {
        host: host.to_owned(),
        authorized,
        can_play,
        shows_in_media_controls: true,
    }
}

// Waits for the next event that `wanted` picks, and returns it with the events before it.
async fn until(
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    wanted: impl Fn(&PlayerEvent) -> bool,
) -> Result<Vec<PlayerEvent>> {
    let mut seen = Vec::new();
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            match events.recv().await {
                Some(event) if wanted(&event) => return Ok(()),
                Some(event) => seen.push(event),
                None => bail!("player events closed"),
            }
        }
    })
    .await??;
    Ok(seen)
}

// Waits for the next event that `wanted` picks.
async fn wait_for(
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    wanted: impl Fn(&PlayerEvent) -> bool,
) -> Result<PlayerEvent> {
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            match events.recv().await {
                Some(event) if wanted(&event) => return Ok(event),
                Some(_) => {}
                None => bail!("player events closed"),
            }
        }
    })
    .await?
}

async fn expect_unavailable(
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    track: &PlayingItem,
) -> Result<UnavailableReason> {
    match wait_for(
        events,
        |event| matches!(event, PlayerEvent::Unavailable(item, _) if item == track),
    )
    .await?
    {
        PlayerEvent::Unavailable(_, reason) => Ok(reason),
        _ => unreachable!(),
    }
}

// Waits until `track` counts as played through.
async fn expect_ended(
    events: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    track: &PlayingItem,
) -> Result<()> {
    wait_for(
        events,
        |event| matches!(event, PlayerEvent::EndOfTrack { item, .. } if item == track),
    )
    .await?;
    Ok(())
}

// How many times `seen` counts `track` as played through.
fn ends(seen: &[PlayerEvent], track: &PlayingItem) -> usize {
    seen.iter()
        .filter(|event| matches!(event, PlayerEvent::EndOfTrack { item, .. } if item == track))
        .count()
}

fn any(seen: &[PlayerEvent], wanted: impl Fn(&PlayerEvent) -> bool) -> bool {
    seen.iter().any(wanted)
}

// Starts the first queued track and checks that the host got it in one batch, and then
// `next` as the track after it, when one is given.
async fn start_first(f: &mut Fixture, items: &[PlayingItem], next: Option<&str>) -> Result<()> {
    let PlayingItem::LibraryTrack { track: key, .. } = &items[0] else {
        bail!("{:?} is not hosted", items[0]);
    };
    f.queue(items)?;
    f.send(PlayerCommand::Switch(0))?;
    expect_playing(&mut f.events, &items[0]).await?;
    let mut expected = vec![handed_over(key, 1)];
    expected.extend(next.map(|next| added(2, next)));
    assert_eq!(f.host.take(expected.len()).await?, expected);
    Ok(())
}

#[tokio::test]
async fn a_hosted_track_between_local_ones_plays_on_its_host() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    let opens = internal.test_outputs().opens.clone();
    with_engine(internal, f, async |f| {
        let (first, hosted, last) = (f.local(0), f.hosted("1440"), f.local(1));
        f.queue(&[first.clone(), hosted.clone(), last.clone()])?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &first).await?;

        // The local track plays to its end, and the host takes over.
        let event = wait_for(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == hosted),
        )
        .await?;
        assert!(
            matches!(&event, PlayerEvent::Playing { path, position, .. }
                if path.as_os_str() == "hosted://host/1440" && position.is_zero()),
            "{event:?}"
        );
        // The local track after it is not the host's to play.
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);

        f.report(status(1, true, 1_000))?;
        f.report(ended(1))?;
        expect_ended(&mut f.events, &hosted).await?;
        expect_playing(&mut f.events, &last).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);
        // Only the local tracks opened an output.
        assert_eq!(opens.load(Ordering::SeqCst), 2);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn pause_play_seek_volume_and_stop_of_a_hosted_track_go_to_its_host() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    let opens = internal.test_outputs().opens.clone();
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;

        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        f.send(PlayerCommand::Play)?;
        expect_playing(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(3, HostAction::Start)]);

        f.send(PlayerCommand::Seek(42.5))?;
        assert_eq!(
            expect_playing(&mut f.events, &hosted).await?,
            Duration::from_millis(42_500)
        );
        let skip = HostAction::Skip {
            index: 0,
            offset_ms: 42_500,
        };
        assert_eq!(f.host.take(1).await?, vec![on(4, skip)]);

        f.send(PlayerCommand::SetVolume(0.5))?;
        let gain = HostAction::SetGain { gain: 0.5 };
        assert_eq!(f.host.take(1).await?, vec![on(5, gain)]);

        f.send(PlayerCommand::Stop)?;
        wait_for(&mut f.events, |event| matches!(event, PlayerEvent::Stopped)).await?;
        assert_eq!(f.host.take(1).await?, vec![released(6)]);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_hand_over_carries_the_volume() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        f.send(PlayerCommand::SetVolume(0.6))?;
        f.queue(std::slice::from_ref(&hosted))?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &hosted).await?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(HOST, &["1440"], 1, 0, 0.6, true)]
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn next_and_previous_move_between_hosted_and_local_tracks() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    let suspends = internal.test_outputs().suspends.clone();
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;

        f.send(PlayerCommand::Next)?;
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);

        // The local output stops before the host takes over again.
        f.send(PlayerCommand::Previous)?;
        expect_playing(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 3)]);
        assert_eq!(suspends.load(Ordering::SeqCst), 1);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_hosted_track_of_another_host_takes_over_from_the_first() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("other-7"));
        f.queue(&[first.clone(), second.clone()])?;
        f.send(PlayerCommand::Switch(0))?;
        wait_for(
            &mut f.events,
            |event| matches!(event, PlayerEvent::HostedChanged(Some(host)) if host == HOST),
        )
        .await?;
        expect_playing(&mut f.events, &first).await?;
        // The track after it is not this host's to play.
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);

        f.send(PlayerCommand::Next)?;
        wait_for(
            &mut f.events,
            |event| matches!(event, PlayerEvent::HostedChanged(Some(host)) if host == OTHER),
        )
        .await?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(
            f.host.take(2).await?,
            vec![released(2), hand_over(OTHER, &["other-7"], 3, 0, 1.0, true)]
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_restored_hosted_track_waits_as_a_local_one_does_until_play() -> Result<()> {
    let (mut internal, mut f) = hosted_engine(2, &[]);
    let hosted = f.hosted("1440");
    f.send(PlayerCommand::Restore {
        tracks: vec![(hosted.clone(), PlaybackSource::LibraryStream)],
        index: Some(0),
        position: Duration::from_millis(1_500),
        mode: PlaybackMode::Sequential,
    })?;
    let event = run_until_track_state(&mut internal, &mut f.events).await?;
    assert!(
        matches!(&event, PlayerEvent::Loaded { item, position, .. }
            if *item == hosted && *position == Duration::from_millis(1_500)),
        "{event:?}"
    );
    // Nothing goes to the host, and the player stands as after a local paused load.
    assert!(f.host.sent_nothing());
    assert!(internal.hosted.is_none());
    assert_eq!(internal.state, InternalPlaybackState::Stopped);
    assert!(internal.output_stale);
    assert_eq!(
        internal.restore_position,
        Some(Duration::from_millis(1_500))
    );
    assert_eq!(
        internal.interrupted_item(),
        InterruptedItem::Item {
            index_hint: 0,
            item: hosted.clone(),
            position: Duration::from_millis(1_500),
            playback_mode: Some(PlaybackMode::Sequential),
            playing: false,
        }
    );

    // A seek only moves the place it plays from.
    f.send(PlayerCommand::Seek(3.0))?;
    let event = run_until_track_state(&mut internal, &mut f.events).await?;
    assert!(
        matches!(&event, PlayerEvent::Paused { position, .. }
            if *position == Duration::from_secs(3)),
        "{event:?}"
    );
    assert!(f.host.sent_nothing());

    f.send(PlayerCommand::Play)?;
    let event = run_until_track_state(&mut internal, &mut f.events).await?;
    assert!(
        matches!(&event, PlayerEvent::Playing { item, position, .. }
            if *item == hosted && *position == Duration::from_secs(3)),
        "{event:?}"
    );
    assert_eq!(
        f.host.take(1).await?,
        vec![hand_over(HOST, &["1440"], 1, 3_000, 1.0, true)]
    );
    assert_eq!(internal.test_outputs().opens.load(Ordering::SeqCst), 0);
    let _ = std::fs::remove_file(&f.wav_path);
    f.host.check()
}

#[tokio::test]
async fn an_end_while_paused_brings_the_next_track_up_paused() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("other-7"));
        start_first(f, &[first.clone(), second.clone()], None).await?;
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        // The host ended the track before it saw the pause.
        f.report(ended(1))?;
        expect_ended(&mut f.events, &first).await?;
        expect_loaded(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![released(3)]);

        f.send(PlayerCommand::Play)?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(OTHER, &["other-7"], 4, 0, 1.0, true)]
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_end_of_a_hosted_last_track_ends_the_playlist() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.report(ended(1))?;
        wait_for(&mut f.events, |event| {
            matches!(event, PlayerEvent::EndOfPlaylist)
        })
        .await?;
        wait_for(&mut f.events, |event| {
            matches!(event, PlayerEvent::HostedChanged(None))
        })
        .await?;
        wait_for(&mut f.events, |event| matches!(event, PlayerEvent::Stopped)).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_end_of_a_hosted_track_counts_the_time_it_played_and_not_the_pause() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, next) = (f.hosted("1440"), f.local(0));
        let began = Instant::now();
        start_first(f, &[hosted.clone(), next], None).await?;
        f.report(status(1, true, 0))?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        f.report(status(1, true, 300))?;
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);
        let paused_at = Instant::now();
        tokio::time::sleep(Duration::from_millis(300)).await;

        // The host ended the track before it saw the pause.
        f.report(ended(1))?;
        let event = wait_for(
            &mut f.events,
            |event| matches!(event, PlayerEvent::EndOfTrack { item, .. } if *item == hosted),
        )
        .await?;
        let PlayerEvent::EndOfTrack {
            listened,
            started_at,
            ..
        } = event
        else {
            unreachable!()
        };
        let played = paused_at.saturating_duration_since(began);
        assert!(
            listened >= Duration::from_millis(250) && listened <= played,
            "{listened:?} of {played:?} played"
        );
        assert!(started_at.is_some());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn removing_the_last_track_takes_it_from_its_host() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 0,
            revision: None,
        })?;
        wait_for(&mut f.events, |event| {
            matches!(event, PlayerEvent::HostedChanged(None))
        })
        .await?;
        wait_for(&mut f.events, |event| matches!(event, PlayerEvent::Stopped)).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);

        // A report the host sent before it let go changes nothing.
        f.report(ended(1))?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::EndOfTrack { .. } | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn pause_during_a_load_stops_the_host_and_the_new_track_comes_up_paused() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[1]);
    let opens = internal.test_outputs().opens.clone();
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;
        f.send(PlayerCommand::Switch(1))?;
        tokio::time::timeout(TEST_DEADLINE, f.started.notified()).await?;

        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        f.release.notify_one();
        expect_loaded(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(3)]);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        Ok(())
    })
    .await
}

// Runs `event` while the second track loads behind a hosted one, and returns what the
// player reported until the second track plays.
async fn during_a_load(f: &mut Fixture, event: HostEvent) -> Result<Vec<PlayerEvent>> {
    let (hosted, track) = (f.hosted("1440"), f.local(0));
    start_first(f, &[hosted, track.clone(), f.local(1)], None).await?;
    f.report(status(1, true, 0))?;
    f.send(PlayerCommand::Switch(1))?;
    tokio::time::timeout(TEST_DEADLINE, f.started.notified()).await?;
    // The event is queued before the load can complete.
    f.report(event)?;
    f.release.notify_one();
    let seen = until(
        &mut f.events,
        |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == track),
    )
    .await?;
    assert_eq!(f.host.take(1).await?, vec![released(2)]);
    Ok(seen)
}

#[tokio::test]
async fn an_end_during_a_load_leaves_the_queue_to_the_load() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[1]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        let seen = during_a_load(f, ended(1)).await?;
        assert_eq!(ends(&seen, &hosted), 0, "{seen:?}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_failure_during_a_load_marks_the_track_and_leaves_the_queue_to_the_load() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[1]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        let seen = during_a_load(f, failed(1, HostFailure::Network)).await?;
        assert!(
            any(&seen, |event| matches!(
                event,
                PlayerEvent::Unavailable(item, UnavailableReason::SourceUnreachable)
                    if *item == hosted
            )),
            "{seen:?}"
        );
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Stopped)),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_host_that_cannot_play_during_a_load_follows_no_play() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[1]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone(), f.local(1)], None).await?;
        f.report(status(1, true, 0))?;
        f.send(PlayerCommand::Switch(1))?;
        tokio::time::timeout(TEST_DEADLINE, f.started.notified()).await?;
        f.report(ready(HOST, true, false))?;
        expect_unavailable(&mut f.events, &hosted).await?;

        // The host lets go of the track it failed, so Play waits for the track that loads.
        f.send(PlayerCommand::Play)?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Playing { item, .. } if *item == hosted
            )),
            "{seen:?}"
        );
        assert_eq!(f.host.take(1).await?, vec![released(2)]);
        f.release.notify_one();
        expect_playing(&mut f.events, &track).await?;
        f.settle().await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_track_the_host_fails_is_unavailable_and_the_next_one_plays() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;
        f.report(failed(1, HostFailure::NotAuthorized))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::AccessRevoked
        );
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_unplayable_track_is_gone() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;
        f.report(failed(1, HostFailure::Unplayable))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::TrackGone
        );
        expect_playing(&mut f.events, &track).await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_host_that_cannot_play_fails_its_tracks_until_it_can() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (first, second, track) = (f.hosted("1440"), f.hosted("1441"), f.local(0));
        let queue = [first.clone(), second.clone(), track.clone()];
        start_first(f, &queue, Some("1441")).await?;

        // The track it holds next fails at once, without going to the host.
        f.report(ready(HOST, true, false))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &first).await?,
            UnavailableReason::LibraryUnavailable
        );
        assert_eq!(
            expect_unavailable(&mut f.events, &second).await?,
            UnavailableReason::LibraryUnavailable
        );
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(3)]);

        f.report(ready(HOST, true, true))?;
        f.send(PlayerCommand::Switch(1))?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1441", 4)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_tracks_a_host_refused_are_available_again_once_it_can_play() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (first, second, track) = (f.hosted("1440"), f.hosted("1441"), f.local(0));
        let queue = [first.clone(), second.clone(), track.clone()];
        start_first(f, &queue, Some("1441")).await?;
        f.report(ready(HOST, true, false))?;
        expect_unavailable(&mut f.events, &first).await?;
        expect_unavailable(&mut f.events, &second).await?;
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(3)]);

        f.report(ready(HOST, true, true))?;
        let seen = f.settle().await?;
        for refused in [&first, &second] {
            assert!(
                any(&seen, |event| matches!(
                    event,
                    PlayerEvent::UnavailableCleared(Some(item)) if item == refused
                )),
                "{seen:?}"
            );
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_track_its_host_could_not_play_stays_unavailable_when_the_host_comes_up() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;
        f.report(failed(1, HostFailure::Unplayable))?;
        expect_unavailable(&mut f.events, &hosted).await?;
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);

        f.report(ready(HOST, true, true))?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::UnavailableCleared(_)
            )),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

// A track its host refused, which fails for another cause with the same mark when it is tried
// again while the host still cannot play, keeps that mark once the host can.
#[tokio::test]
async fn a_refused_track_that_fails_for_another_cause_stays_unavailable_when_its_host_can_play()
-> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        start_first(f, &[hosted.clone(), track.clone()], None).await?;
        f.report(ready(HOST, true, false))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::LibraryUnavailable
        );
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);

        // The listener tries it again while its service cannot be reached either.
        f.down.store(true, Ordering::SeqCst);
        f.send(PlayerCommand::LoadAtPosition {
            index: 0,
            position: Duration::ZERO,
            play: true,
        })?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::LibraryUnavailable
        );

        f.report(ready(HOST, true, true))?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::UnavailableCleared(_)
            )),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_host_without_authorization_fails_its_tracks_as_access_revoked() -> Result<()> {
    let (internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    with_engine(internal, f, async |f| {
        let (first, second, track) = (f.hosted("1440"), f.hosted("1441"), f.local(0));
        let queue = [first.clone(), second.clone(), track.clone()];
        start_first(f, &queue, Some("1441")).await?;
        f.report(ready(HOST, false, true))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &first).await?,
            UnavailableReason::AccessRevoked
        );
        assert_eq!(
            expect_unavailable(&mut f.events, &second).await?,
            UnavailableReason::AccessRevoked
        );
        expect_playing(&mut f.events, &track).await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn without_a_host_bridge_a_hosted_track_is_unavailable() -> Result<()> {
    let (mut internal, f) = hosted_engine(LONG_TRACK_SAMPLES, &[]);
    internal.host_bridge = Arc::new(RwLock::new(None));
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        f.queue(&[hosted.clone(), track.clone()])?;
        f.send(PlayerCommand::Switch(0))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::LibraryUnavailable
        );
        expect_playing(&mut f.events, &track).await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_track_whose_host_never_reports_is_unreachable() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(f, &[first.clone(), second.clone()], Some("1441")).await?;
        tokio::time::sleep(Duration::from_secs(9)).await;
        f.assert_all_available();

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            expect_unavailable(&mut f.events, &first).await?,
            UnavailableReason::SourceUnreachable
        );
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(
            f.host.take(2).await?,
            vec![released(3), handed_over("1441", 4)]
        );
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_paused_track_waits_for_its_host_without_a_deadline() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &hosted).await?;
        tokio::time::sleep(Duration::from_secs(60)).await;
        f.assert_all_available();

        // The host has as long again once it is told to play.
        f.send(PlayerCommand::Play)?;
        expect_playing(&mut f.events, &hosted).await?;
        tokio::time::sleep(Duration::from_secs(9)).await;
        f.assert_all_available();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::SourceUnreachable
        );
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_host_that_goes_quiet_while_playing_fails_its_track() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(f, &[first.clone(), second.clone()], Some("1441")).await?;
        f.report(status(1, true, 0))?;
        f.settle().await?;
        tokio::time::sleep(Duration::from_secs(14)).await;
        f.assert_all_available();

        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            expect_unavailable(&mut f.events, &first).await?,
            UnavailableReason::SourceUnreachable
        );
        expect_playing(&mut f.events, &second).await?;
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_host_that_comes_up_again_gets_its_tracks_again_where_they_stood() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        let queue = [first.clone(), second.clone(), f.hosted("1442")];
        start_first(f, &queue, Some("1441")).await?;
        f.report(status(2, true, 5_000))?;
        f.settle().await?;
        tokio::time::advance(Duration::from_secs(2)).await;
        f.report(f.host.restart())?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(HOST, &["1440", "1441"], 3, 7_000, 1.0, true)]
        );

        // The queue starts over, so the host moves on to index 1 again.
        f.report(f.host.move_on())?;
        expect_ended(&mut f.events, &first).await?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![added(4, "1442")]);

        // A paused track goes back paused.
        f.send(PlayerCommand::Pause)?;
        let paused_at = expect_paused(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![on(5, HostAction::Stop)]);
        f.report(f.host.restart())?;
        let offset = u64::try_from(paused_at.as_millis())?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(HOST, &["1441", "1442"], 6, offset, 1.0, false)]
        );
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Unavailable(..))),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_host_that_loses_its_track_gets_it_again_once() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Pause)?;
        // The track may have played for a moment before the pause.
        let paused_at = expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        f.report(lost(2))?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(
                HOST,
                &["1440"],
                3,
                offset_ms(paused_at),
                1.0,
                false
            )]
        );
        f.report(lost(3))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::SourceUnreachable
        );
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_host_that_plays_another_track_in_its_place_gets_it_again_once() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Pause)?;
        let paused_at = expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        f.report(status_of(2, "9999", false, 0))?;
        let offset = u64::try_from(paused_at.as_millis())?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(HOST, &["1440"], 3, offset, 1.0, false)]
        );
        f.report(status_of(3, "9999", false, 0))?;
        assert_eq!(
            expect_unavailable(&mut f.events, &hosted).await?,
            UnavailableReason::SourceUnreachable
        );
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn events_about_an_older_track_or_from_another_host_change_nothing() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 2)]);

        let other = OTHER.to_owned();
        for event in [
            ended(1),
            failed(1, HostFailure::Unplayable),
            status(1, false, 5_000),
            lost(1),
            HostEvent::JukeboxEnded {
                host: other.clone(),
                revision: 2,
                index: None,
            },
            HostEvent::JukeboxItemFailed {
                host: other.clone(),
                revision: 2,
                index: 0,
                reason: HostFailure::Network,
            },
            HostEvent::JukeboxStatus {
                host: other.clone(),
                revision: 2,
                current_index: None,
                current_id: None,
                playing: false,
                position_ms: 5_000,
            },
            ready(&other, false, false),
            HostEvent::Unknown,
        ] {
            f.report(event)?;
        }
        for event in f.settle().await? {
            match event {
                PlayerEvent::Progress { position, .. } => {
                    assert!(position < Duration::from_secs(1), "{position:?}");
                }
                PlayerEvent::PlaylistUpdated(..) | PlayerEvent::RealtimeFFT(_) => {}
                event => panic!("{event:?}"),
            }
        }
        assert!(f.host.sent_nothing());

        // An event about the latest batch still counts.
        f.report(status(2, false, 1_000))?;
        assert_eq!(
            expect_paused(&mut f.events, &hosted).await?,
            Duration::from_secs(1)
        );
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_status_from_before_a_pause_or_a_seek_is_dropped() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Pause)?;
        let paused_at = expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![on(2, HostAction::Stop)]);

        // The host still played when it reported.
        f.report(status(1, true, 3_000))?;
        stays_paused_at(f, paused_at).await?;

        f.send(PlayerCommand::Seek(30.0))?;
        expect_paused(&mut f.events, &hosted).await?;
        f.report(status(2, false, 600))?;
        stays_paused_at(f, Duration::from_secs(30)).await?;
        f.assert_all_available();
        Ok(())
    })
    .await
}

// Checks that the current track stays paused at `position` while the player handles what was
// sent so far, and for a few ticks after. A paused track reports its place once, and again only
// when it changes, so the reports may hold none: a report of playing or of another place fails.
async fn stays_paused_at(f: &mut Fixture, position: Duration) -> Result<()> {
    let mut seen = f.settle().await?;
    tokio::time::sleep(PROGRESS_INTERVAL * 3).await;
    while let Ok(event) = f.events.try_recv() {
        seen.push(event);
    }
    for event in &seen {
        match event {
            PlayerEvent::Progress {
                position: reported, ..
            } => anyhow::ensure!(*reported == position, "{seen:?}"),
            PlayerEvent::Playing { .. } | PlayerEvent::Paused { .. } => bail!("{seen:?}"),
            _ => {}
        }
    }
    Ok(())
}

#[tokio::test]
async fn repeat_one_hands_the_same_track_to_its_host_again() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (hosted, track) = (f.hosted("1440"), f.local(0));
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        start_first(f, &[hosted.clone(), track], None).await?;
        f.report(status(1, true, 0))?;
        f.report(ended(1))?;
        expect_ended(&mut f.events, &hosted).await?;
        expect_playing(&mut f.events, &hosted).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 2)]);
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_hosted_track_moves_on_from_its_report_while_it_plays_only() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.report(status(1, true, 10_000))?;
        f.settle().await?;
        tokio::time::advance(Duration::from_millis(400)).await;

        f.send(PlayerCommand::Pause)?;
        let paused_at = expect_paused(&mut f.events, &hosted).await?;
        assert_eq!(paused_at, Duration::from_millis(10_400));
        tokio::time::sleep(Duration::from_secs(1)).await;

        // A report moves a paused track too. Until then the paused track told its place once at
        // most: not at all when the last report while it played stood at the same place.
        f.report(status(2, false, 20_000))?;
        let mut told = Vec::new();
        loop {
            let position = next_progress(&mut f.events).await?;
            if position == Duration::from_secs(20) {
                break;
            }
            told.push(position);
        }
        assert!(
            told.len() <= 1 && told.iter().all(|position| *position == paused_at),
            "{told:?}"
        );
        Ok(())
    })
    .await
}

// The host holds the next track, so it moves on to it without a gap.

#[tokio::test]
async fn consecutive_hosted_tracks_are_handed_over_once() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    let opens = internal.test_outputs().opens.clone();
    with_engine(internal, f, async |f| {
        let (first, second, third) = (f.hosted("1440"), f.hosted("1441"), f.hosted("1442"));
        let queue = [first.clone(), second.clone(), third.clone()];
        start_first(f, &queue, Some("1441")).await?;
        f.report(f.host.status())?;

        // The host moves on by itself. The first track played through, and the second
        // is current without being handed over again.
        f.report(f.host.move_on())?;
        expect_ended(&mut f.events, &first).await?;
        let event = next_track_state(&mut f.events).await?;
        assert!(
            matches!(&event, PlayerEvent::Playing { item, index: 1, path, position, .. }
                if *item == second
                    && path.as_os_str() == "hosted://host/1441"
                    && position.is_zero()),
            "{event:?}"
        );
        // The host gets the track after it, and nothing else.
        assert_eq!(f.host.take(1).await?, vec![added(3, "1442")]);

        // A report about the entry the second track stands at only says where it is.
        f.report(f.host.status())?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::EndOfTrack { .. } | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        f.report(f.host.move_on())?;
        expect_ended(&mut f.events, &second).await?;
        expect_playing(&mut f.events, &third).await?;

        // Nothing follows the last track, so the host's queue ends with it.
        f.report(f.host.end())?;
        expect_ended(&mut f.events, &third).await?;
        wait_for(&mut f.events, |event| {
            matches!(event, PlayerEvent::EndOfPlaylist)
        })
        .await?;
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        assert_eq!(opens.load(Ordering::SeqCst), 0);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_queue_edit_changes_the_next_track_in_place() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let first = f.hosted("1440");
        let queue = [first.clone(), f.hosted("1441"), f.hosted("1442")];
        start_first(f, &queue, Some("1441")).await?;

        // Moving the third track before the second makes it the next one.
        f.send(PlayerCommand::MovePlayListItem {
            old_index: 2,
            new_index: 1,
            revision: None,
        })?;
        assert_eq!(f.host.take(2).await?, vec![removed(3, 1), added(4, "1442")]);

        // An edit that leaves the next track as it is changes nothing for the host.
        f.queue(&[f.hosted("1443"), f.local(0)])?;
        let mut seen = f.settle().await?;
        assert!(f.host.sent_nothing());

        // Removing the next track makes the one after it next.
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 1,
            revision: None,
        })?;
        assert_eq!(f.host.take(2).await?, vec![removed(5, 1), added(6, "1441")]);

        // So does a track to play next.
        f.send(PlayerCommand::AddToPlaylist {
            tracks: vec![(f.hosted("1444"), PlaybackSource::LibraryStream)],
            mode: AddMode::PlayNext,
            revision: None,
        })?;
        assert_eq!(f.host.take(2).await?, vec![removed(7, 1), added(8, "1444")]);

        // The first track played on throughout.
        seen.extend(f.settle().await?);
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Playing { .. }
                    | PlayerEvent::Paused { .. }
                    | PlayerEvent::Loaded { .. }
                    | PlayerEvent::Stopped
                    | PlayerEvent::EndOfTrack { .. }
            )),
            "{seen:?}"
        );
        assert_eq!(
            f.host.queue(),
            (
                vec!["1440".to_owned(), "1444".to_owned()],
                Some("1440".to_owned())
            )
        );
        f.report(f.host.move_on())?;
        expect_ended(&mut f.events, &first).await?;
        let next = f.hosted("1444");
        expect_playing(&mut f.events, &next).await?;
        Ok(())
    })
    .await
}

#[tokio::test]
async fn no_track_is_handed_over_ahead_under_repeat_one_or_before_a_local_one() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second, track) = (f.hosted("1440"), f.hosted("1441"), f.local(0));
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        start_first(f, &[first, second.clone(), track.clone()], None).await?;

        // Leaving repeat-one gives the host the next track, and going back takes it
        // away again.
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Sequential))?;
        assert_eq!(f.host.take(1).await?, vec![added(2, "1441")]);
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        assert_eq!(f.host.take(1).await?, vec![removed(3, 1)]);

        // Before a local track the host holds nothing next.
        f.send(PlayerCommand::Switch(1))?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1441", 4)]);
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Sequential))?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        // Its end is the end of the host's queue, and the local track plays.
        f.report(f.host.end())?;
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(5)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn next_skips_to_the_track_the_host_holds_and_previous_hands_over_anew() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        let queue = [first.clone(), second.clone(), f.hosted("1442")];
        start_first(f, &queue, Some("1441")).await?;
        f.report(status(2, true, 4_000))?;

        // Next goes to the track the host holds next with a skip. A track the user skips
        // does not count as played through.
        f.send(PlayerCommand::Next)?;
        let seen = f.settle().await?;
        assert!(
            any(
                &seen,
                |event| matches!(event, PlayerEvent::Playing { item, position, .. }
                if *item == second && position.is_zero())
            ),
            "{seen:?}"
        );
        assert_eq!(ends(&seen, &first), 0, "{seen:?}");
        assert_eq!(
            f.host.take(2).await?,
            vec![skipped_to(3, 1), added(4, "1442")]
        );

        // A seek moves the current track only, at its own index.
        f.send(PlayerCommand::Seek(30.0))?;
        assert_eq!(
            expect_playing(&mut f.events, &second).await?,
            Duration::from_secs(30)
        );
        let skip = HostAction::Skip {
            index: 1,
            offset_ms: 30_000,
        };
        assert_eq!(f.host.take(1).await?, vec![on(5, skip)]);

        // Previous takes the track the host holds next away while the earlier track
        // loads, and hands that one over anew. The track after it follows.
        f.send(PlayerCommand::Previous)?;
        expect_playing(&mut f.events, &first).await?;
        assert_eq!(
            f.host.take(3).await?,
            vec![removed(6, 2), handed_over("1440", 7), added(8, "1441")]
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_move_on_to_a_track_taken_out_meanwhile_loads_what_comes_next() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, track) = (f.hosted("1440"), f.local(0));
        start_first(
            f,
            &[first.clone(), f.hosted("1441"), track.clone()],
            Some("1441"),
        )
        .await?;
        f.report(f.host.status())?;

        // The host moves on to the second track, and its report is still on its way when
        // the second track leaves the queue. The host plays on, as it never removes the
        // entry it plays.
        let moved = f.host.move_on();
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 1,
            revision: None,
        })?;
        assert_eq!(f.host.take(1).await?, vec![removed(3, 1)]);
        assert_eq!(f.host.queue().1.as_deref(), Some("1441"));

        // The first track counts as played through once, and what comes next plays.
        f.report(moved)?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == track),
        )
        .await?;
        assert_eq!(ends(&seen, &first), 1, "{seen:?}");
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Unavailable(..) | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        let seen = f.settle().await?;
        assert_eq!(ends(&seen, &first), 0, "{seen:?}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_move_on_that_crosses_a_new_next_track_hands_that_one_over() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second, third) = (f.hosted("1440"), f.hosted("1441"), f.hosted("1442"));
        start_first(
            f,
            &[first.clone(), second.clone(), third.clone()],
            Some("1441"),
        )
        .await?;
        f.report(f.host.status())?;

        // The host moves on to the second track as the third becomes the next one. It
        // keeps the second, and reports it at index 1, where the player keeps the third.
        let moved = f.host.move_on();
        f.send(PlayerCommand::MovePlayListItem {
            old_index: 2,
            new_index: 1,
            revision: None,
        })?;
        assert_eq!(f.host.take(2).await?, vec![removed(3, 1), added(4, "1442")]);
        f.report(f.host.status())?;

        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == third),
        )
        .await?;
        assert_eq!(ends(&seen, &first), 1, "{seen:?}");
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Playing { .. })),
            "{seen:?}"
        );
        assert_eq!(
            f.host.take(2).await?,
            vec![handed_over("1442", 5), added(6, "1441")]
        );

        // The report that the host sent as it moved on comes late, and changes nothing.
        f.report(moved)?;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::EndOfTrack { .. } | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        assert_eq!(f.host.queue().1.as_deref(), Some("1442"));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn next_as_the_host_moves_on_starts_the_next_track_again() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(
            f,
            &[first.clone(), second.clone(), f.hosted("1442")],
            Some("1441"),
        )
        .await?;
        f.report(f.host.status())?;

        let moved = f.host.move_on();
        f.send(PlayerCommand::Next)?;
        let mut seen = f.settle().await?;
        assert_eq!(
            f.host.take(2).await?,
            vec![skipped_to(3, 1), added(4, "1442")]
        );
        // The host plays the second track from its start again, and the report from
        // before the skip is stale.
        f.report(moved)?;
        f.report(f.host.status())?;
        seen.extend(f.settle().await?);
        let playing = seen
            .iter()
            .filter(|event| matches!(event, PlayerEvent::Playing { .. }))
            .collect::<Vec<_>>();
        assert!(
            matches!(playing[..], [PlayerEvent::Playing { item, position, .. }]
                if *item == second && position.is_zero()),
            "{seen:?}"
        );
        assert_eq!(ends(&seen, &first), 0, "{seen:?}");
        assert_eq!(f.host.queue().1.as_deref(), Some("1441"));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_move_on_that_crosses_a_pause_brings_the_next_track_up_paused() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(
            f,
            &[first.clone(), second.clone(), f.hosted("1442")],
            Some("1441"),
        )
        .await?;

        // The host moved on before it saw the pause, and its report from before the
        // pause is stale. It reports again once it applied the pause.
        let moved = f.host.move_on();
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![on(3, HostAction::Stop)]);
        f.report(moved)?;
        f.report(f.host.status())?;
        expect_ended(&mut f.events, &first).await?;
        assert_eq!(expect_paused(&mut f.events, &second).await?, Duration::ZERO);
        assert_eq!(f.host.take(1).await?, vec![added(4, "1442")]);

        f.send(PlayerCommand::Play)?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![on(5, HostAction::Start)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn volume_changes_do_not_make_the_reports_of_the_host_stale() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(
            f,
            &[first.clone(), second.clone(), f.hosted("1442")],
            Some("1441"),
        )
        .await?;
        let moved = f.host.move_on();
        for volume in [0.5, 0.4, 0.3] {
            f.send(PlayerCommand::SetVolume(volume))?;
        }
        assert_eq!(
            f.host.take(3).await?,
            vec![
                on(3, HostAction::SetGain { gain: 0.5 }),
                on(4, HostAction::SetGain { gain: 0.4 }),
                on(5, HostAction::SetGain { gain: 0.3 }),
            ]
        );
        f.report(moved)?;
        expect_ended(&mut f.events, &first).await?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(f.host.take(1).await?, vec![added(6, "1442")]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_next_track_the_host_fails_leaves_its_queue_and_is_handed_over_as_usual() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(
            f,
            &[first.clone(), second.clone(), f.hosted("1442")],
            Some("1441"),
        )
        .await?;

        // A failure from before the host held the second track is about nothing it holds.
        f.report(failed_at(1, 1, HostFailure::Network))?;
        let mut seen = f.settle().await?;
        assert!(f.host.sent_nothing());

        f.report(failed_at(2, 1, HostFailure::Network))?;
        assert_eq!(f.host.take(1).await?, vec![removed(3, 1)]);
        seen.extend(f.settle().await?);
        assert!(f.host.sent_nothing());
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Unavailable(..))),
            "{seen:?}"
        );

        // At the end of the current track the next one is handed over as usual.
        f.report(f.host.end())?;
        expect_ended(&mut f.events, &first).await?;
        expect_playing(&mut f.events, &second).await?;
        assert_eq!(
            f.host.take(2).await?,
            vec![handed_over("1441", 4), added(5, "1442")]
        );
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn the_next_track_is_looked_up_for_five_seconds_without_holding_up_the_current_one()
-> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let first = f.hosted("1440");
        f.slow.delay_ms.store(6_000, Ordering::SeqCst);
        let begun = Instant::now();
        f.queue(&[first.clone(), f.hosted("slow-1")])?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);
        assert!(begun.elapsed() < Duration::from_secs(1));
        f.report(status(1, true, 0))?;

        // An answer that would take six seconds is not waited for.
        tokio::time::sleep(Duration::from_secs(7)).await;
        assert!(f.host.sent_nothing());
        assert!(f.slow.dropped.load(Ordering::SeqCst));
        assert!(!f.slow.answered.load(Ordering::SeqCst));

        // One in four seconds is.
        f.slow.delay_ms.store(4_000, Ordering::SeqCst);
        f.report(status(1, true, 7_000))?;
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Sequential))?;
        assert_eq!(f.host.take(1).await?, vec![added(2, "slow-1")]);
        assert!(f.slow.answered.load(Ordering::SeqCst));
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn an_answer_to_an_older_question_about_the_next_track_is_dropped() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        f.slow.delay_ms.store(4_000, Ordering::SeqCst);
        let queue = [f.hosted("1440"), f.hosted("1441"), f.hosted("slow-1")];
        start_first(f, &queue, Some("1441")).await?;
        f.report(status(2, true, 0))?;

        // The slow track becomes the next one, and an answer to the question about the
        // track before it comes late.
        f.send(PlayerCommand::MovePlayListItem {
            old_index: 2,
            new_index: 1,
            revision: None,
        })?;
        assert_eq!(f.host.take(1).await?, vec![removed(3, 1)]);
        f.send(PlayerCommand::HostRunResolved {
            request: 1,
            track: Some(HostedTrack {
                host: HOST.to_owned(),
                key: "1441".to_owned(),
            }),
        })?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        assert_eq!(f.host.take(1).await?, vec![added(4, "slow-1")]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_next_track_the_resolver_fails_on_is_not_handed_over_ahead() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, broken) = (f.hosted("1440"), f.hosted("error-1"));
        f.queue(&[first.clone(), broken.clone()])?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);
        tokio::time::timeout(TEST_DEADLINE, f.failed.notified()).await?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        // Once current, it fails as a load does.
        f.report(ended(1))?;
        expect_ended(&mut f.events, &first).await?;
        expect_unavailable(&mut f.events, &broken).await?;
        wait_for(&mut f.events, |event| {
            matches!(event, PlayerEvent::EndOfPlaylist)
        })
        .await?;
        assert_eq!(f.host.take(1).await?, vec![released(2)]);
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_change_of_mode_stops_the_lookup_of_a_track_that_no_longer_comes_next() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let first = f.hosted("1440");
        f.slow.delay_ms.store(4_000, Ordering::SeqCst);
        f.queue(&[first.clone(), f.hosted("slow-1")])?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);
        f.report(status(1, true, 0))?;
        tokio::time::sleep(Duration::from_secs(1)).await;

        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        f.settle().await?;
        tokio::time::timeout(TEST_DEADLINE, f.slow.on_drop.notified()).await?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(!f.slow.answered.load(Ordering::SeqCst));
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn shuffle_keeps_the_track_the_host_holds_next_where_next_goes() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let keys = ["1440", "1441", "1442", "1443", "1444"];
        let queue = keys.map(|key| f.hosted(key));
        start_first(f, &queue, Some("1441")).await?;
        f.report(f.host.status())?;

        // The shuffled order may bring another track next, which the host gets in place.
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Shuffle))?;
        f.settle().await?;
        // A next track that changed comes with an `add`, once its lookup answered, which may be
        // before or after the drain.
        let mut batches = f.host.take(0).await?;
        let has = |batches: &[HostCommand], wanted: fn(&HostAction) -> bool| {
            batches.iter().any(|batch| batch.actions.iter().any(wanted))
        };
        if has(&batches, |action| {
            matches!(action, HostAction::Remove { .. })
        }) && !has(&batches, |action| matches!(action, HostAction::Add { .. }))
        {
            batches.extend(f.host.take(1).await?);
        }
        assert!(
            batches
                .iter()
                .flat_map(|batch| &batch.actions)
                .all(|action| matches!(
                    action,
                    HostAction::Remove { index: 1 } | HostAction::Add { .. }
                )),
            "{batches:?}"
        );
        let (held, current) = f.host.queue();
        assert_eq!(current.as_deref(), Some("1440"));
        assert_eq!(held.len(), 2, "{held:?}");

        // Next goes where the host holds it.
        f.send(PlayerCommand::Next)?;
        let seen = f.settle().await?;
        let next = f.hosted(&held[1]);
        assert!(
            any(
                &seen,
                |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == next)
            ),
            "{seen:?}"
        );
        let batches = f.host.take(1).await?;
        assert_eq!(batches[0].actions, skipped_to(0, 1).actions);
        assert_eq!(f.host.queue().1, Some(held[1].clone()));
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_stream_found_for_the_next_track_is_let_go() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let first = f.hosted("1440");
        f.queue(&[first.clone(), f.hosted("stream-1")])?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &first).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1440", 1)]);
        let stream = tokio::time::timeout(TEST_DEADLINE, f.streams.recv())
            .await?
            .context("the resolver handed out no stream")?;
        tokio::time::timeout(TEST_DEADLINE, stream.cancelled()).await?;
        f.settle().await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

// The host holds the run of tracks after the current one, and plays on through it while
// the player cannot run, as in an app that the system suspended.

// A batch for the main host that removes the entries at `indexes`, in that order.
fn trimmed(revision: u64, indexes: &[u32]) -> HostCommand {
    batch(
        HOST,
        revision,
        indexes
            .iter()
            .map(|index| HostAction::Remove { index: *index })
            .collect(),
    )
}

fn keys(keys: &[&str]) -> Vec<String> {
    keys.iter().map(|key| (*key).to_owned()).collect()
}

#[tokio::test]
async fn consecutive_hosted_tracks_go_to_the_host_as_one_run() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        assert_eq!(
            f.host.take(4).await?,
            vec![
                handed_over("1440", 1),
                added(2, "1441"),
                added(3, "1442"),
                added(4, "1443"),
            ]
        );
        assert_eq!(
            f.host.queue(),
            (
                keys(&["1440", "1441", "1442", "1443"]),
                Some("1440".to_owned())
            )
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn each_track_of_a_run_counts_only_its_own_time() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(2).await?;
        f.report(f.host.status())?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        f.report(f.host.move_on())?;
        f.report(f.host.end())?;
        let mut listened = Vec::new();
        for track in &queue {
            match wait_for(
                &mut f.events,
                |event| matches!(event, PlayerEvent::EndOfTrack { item, .. } if item == track),
            )
            .await?
            {
                PlayerEvent::EndOfTrack {
                    listened: heard, ..
                } => listened.push(heard),
                _ => unreachable!(),
            }
        }
        assert!(
            listened[0] >= Duration::from_millis(250) && listened[1] < Duration::from_millis(250),
            "{listened:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_run_stops_before_a_track_the_player_plays_itself() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let (second, track, after) = (f.hosted("1441"), f.local(0), f.hosted("1442"));
        let queue = [
            f.hosted("1440"),
            second.clone(),
            track.clone(),
            after.clone(),
        ];
        start_first(f, &queue, Some("1441")).await?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        // The host moves on, and nothing that it plays follows the local track's place.
        f.report(f.host.move_on())?;
        expect_playing(&mut f.events, &second).await?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        f.report(f.host.end())?;
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(3)]);
        expect_playing(&mut f.events, &after).await?;
        assert_eq!(f.host.take(1).await?, vec![handed_over("1442", 4)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_run_is_capped_and_topped_up_as_the_host_moves_on() -> Result<()> {
    let (internal, f) = run_engine();
    let internal = internal.with_host_run_limit(2);
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443", "1444"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        assert_eq!(
            f.host.take(3).await?,
            vec![handed_over("1440", 1), added(2, "1441"), added(3, "1442")]
        );
        f.settle().await?;
        assert!(f.host.sent_nothing());

        f.report(f.host.move_on())?;
        expect_playing(&mut f.events, &queue[1]).await?;
        assert_eq!(f.host.take(1).await?, vec![added(4, "1443")]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_queue_edit_inside_the_run_trims_it_from_the_tail() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(4).await?;

        // Without the third track the fourth follows the second. The host loses both
        // from the tail, and gets the fourth again.
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 2,
            revision: None,
        })?;
        assert_eq!(
            f.host.take(2).await?,
            vec![trimmed(5, &[3, 2]), added(6, "1443")]
        );
        let mut seen = f.settle().await?;

        // A local track to play next leaves the host nothing to hold ahead.
        f.send(PlayerCommand::AddToPlaylist {
            tracks: vec![(f.local(0), PlaybackSource::LibraryStream)],
            mode: AddMode::PlayNext,
            revision: None,
        })?;
        assert_eq!(f.host.take(1).await?, vec![trimmed(7, &[2, 1])]);
        seen.extend(f.settle().await?);
        assert!(f.host.sent_nothing());
        assert_eq!(f.host.queue(), (keys(&["1440"]), Some("1440".to_owned())));

        // The first track played on throughout.
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Playing { .. }
                    | PlayerEvent::Paused { .. }
                    | PlayerEvent::Loaded { .. }
                    | PlayerEvent::Stopped
                    | PlayerEvent::EndOfTrack { .. }
            )),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_change_of_mode_takes_the_run_away_and_gives_it_back() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(3).await?;

        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatOne))?;
        assert_eq!(f.host.take(1).await?, vec![trimmed(4, &[2, 1])]);
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Sequential))?;
        assert_eq!(
            f.host.take(2).await?,
            vec![added(5, "1441"), added(6, "1442")]
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_status_several_entries_ahead_counts_the_tracks_between_as_played() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(4).await?;
        f.report(f.host.status())?;

        // The host played on by three tracks while the player did not hear from it.
        f.report(f.host.move_on_by(3))?;
        let seen = until(&mut f.events, |event| {
            matches!(event, PlayerEvent::Playing { item, index: 3, .. } if *item == queue[3])
        })
        .await?;
        for track in &queue[..3] {
            assert_eq!(ends(&seen, track), 1, "{seen:?}");
        }
        // The player heard none of the tracks between, so none of their time counts.
        for event in &seen {
            if let PlayerEvent::EndOfTrack {
                item,
                listened,
                started_at,
                ..
            } = event
                && queue[1..3].contains(item)
            {
                assert_eq!(
                    (*listened, *started_at),
                    (Duration::ZERO, None),
                    "{event:?}"
                );
            }
        }
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Unavailable(..) | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        // The last track is current without being handed over again.
        f.settle().await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

// A status of the main host about track `id` at `index`.
fn status_at(revision: u64, index: u32, id: &str, playing: bool) -> HostEvent {
    HostEvent::JukeboxStatus {
        host: HOST.to_owned(),
        revision,
        current_index: Some(index),
        current_id: Some(id.to_owned()),
        playing,
        position_ms: 0,
    }
}

// Starts a run of the hosted tracks `keys` from the first, and waits until the host holds
// them all.
async fn start_run(f: &mut Fixture, keys: &[&str]) -> Result<Vec<PlayingItem>> {
    let queue = keys.iter().map(|key| f.hosted(key)).collect::<Vec<_>>();
    f.queue(&queue)?;
    f.send(PlayerCommand::Switch(0))?;
    expect_playing(&mut f.events, &queue[0]).await?;
    f.host.take(keys.len()).await?;
    Ok(queue)
}

#[tokio::test]
async fn a_status_ahead_about_a_track_not_held_there_counts_what_the_host_played() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = start_run(f, &["1440", "1441", "1442", "1443"]).await?;

        // The host stands where the player holds the fourth track, but plays another.
        // It played the second and the third on its way, so those count, and the fourth
        // is handed over anew rather than the second.
        f.report(status_at(4, 3, "9999", true))?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == queue[3]),
        )
        .await?;
        for played in &queue[..3] {
            assert_eq!(ends(&seen, played), 1, "{seen:?}");
        }
        assert_eq!(f.host.take(1).await?, vec![handed_over("1443", 5)]);
        f.assert_all_available();
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_end_at_an_entry_the_player_does_not_hold_counts_what_it_holds() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let track = f.local(0);
        let hosted = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        let mut queue = hosted.to_vec();
        queue.push(track.clone());
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &hosted[0]).await?;
        f.host.take(3).await?;

        f.report(HostEvent::JukeboxEnded {
            host: HOST.to_owned(),
            revision: 3,
            index: Some(5),
        })?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == track),
        )
        .await?;
        for played in &hosted {
            assert_eq!(ends(&seen, played), 1, "{seen:?}");
        }
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Unavailable(..))),
            "{seen:?}"
        );
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_end_of_an_entry_replaced_after_the_report_leaves_its_replacement_to_play() -> Result<()>
{
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(3).await?;

        // The second track leaves the queue, and the host gets the third in its place.
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 1,
            revision: None,
        })?;
        assert_eq!(
            f.host.take(2).await?,
            vec![trimmed(4, &[2, 1]), added(5, "1442")]
        );

        // The host had played on through the second track before it saw that, and ended it.
        f.report(HostEvent::JukeboxEnded {
            host: HOST.to_owned(),
            revision: 3,
            index: Some(1),
        })?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == queue[2]),
        )
        .await?;
        assert_eq!(ends(&seen, &queue[0]), 1, "{seen:?}");
        assert_eq!(ends(&seen, &queue[2]), 0, "{seen:?}");
        assert_eq!(f.host.take(1).await?, vec![handed_over("1442", 6)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_move_on_past_an_entry_replaced_after_the_report_plays_the_replacement_anew() -> Result<()>
{
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(4).await?;

        // The second track leaves the queue, and the host gets the rest of the run again.
        f.send(PlayerCommand::RemoveFromPlaylist {
            index: 1,
            revision: None,
        })?;
        assert_eq!(
            f.host.take(3).await?,
            vec![trimmed(5, &[3, 2, 1]), added(6, "1442"), added(7, "1443")]
        );

        // The host had played on through the second track into the third before it saw that.
        f.report(status_at(4, 2, "1442", true))?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == queue[2]),
        )
        .await?;
        assert_eq!(ends(&seen, &queue[0]), 1, "{seen:?}");
        assert_eq!(f.host.take(1).await?, vec![handed_over("1442", 8)]);
        let seen = f.settle().await?;
        assert_eq!(ends(&seen, &queue[2]), 0, "{seen:?}");
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn the_place_of_a_hosted_track_stays_while_the_player_did_not_run() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.report(status(1, true, 10_000))?;
        f.settle().await?;

        tokio::time::advance(Duration::from_secs(60)).await;
        f.settle().await?;
        while f.events.try_recv().is_ok() {}
        let position = next_progress(&mut f.events).await?;
        assert!(
            position < Duration::from_secs(11),
            "the place ran on to {position:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_long_hosted_queue_goes_to_the_host_a_whole_run_ahead() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let keys = (0..30)
            .map(|key| format!("{}", 1400 + key))
            .collect::<Vec<_>>();
        let queue = keys.iter().map(|key| f.hosted(key)).collect::<Vec<_>>();
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        let mut expected = vec![handed_over(&keys[0], 1)];
        for (revision, key) in (2..).zip(&keys[1..=RUN_LIMIT]) {
            expected.push(added(revision, key));
        }
        assert_eq!(f.host.take(RUN_LIMIT + 1).await?, expected);
        f.settle().await?;
        assert!(f.host.sent_nothing());

        // Each move on tops the run up by one.
        f.report(f.host.move_on())?;
        expect_playing(&mut f.events, &queue[1]).await?;
        let revision = u64::try_from(RUN_LIMIT).unwrap() + 2;
        assert_eq!(
            f.host.take(1).await?,
            vec![added(revision, &keys[RUN_LIMIT + 1])]
        );

        // A host that comes up again gets the current track and the whole run in one set.
        f.report(f.host.restart())?;
        let batches = f.host.take(1).await?;
        let HostAction::Set { ids } = &batches[0].actions[0] else {
            bail!("{batches:?}");
        };
        assert_eq!(ids[..], keys[1..=RUN_LIMIT + 1]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn the_run_follows_the_shuffled_order() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let keys = ["1440", "1441", "1442", "1443", "1444", "1445"];
        start_run(f, &keys).await?;
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::Shuffle))?;
        f.settle().await?;
        // The host loses at once what no longer follows, and gets the rest of the shuffled run
        // as each lookup answers, until it holds every other track after the current one.
        while f.host.queue().0.len() < keys.len() {
            f.host.take(1).await?;
        }
        let _ = f.host.take(0).await?;
        let (held, current) = f.host.queue();
        assert_eq!(current.as_deref(), Some("1440"));
        let mut others = held[1..].to_vec();
        others.sort();
        assert_eq!(others, keys[1..]);

        // Next goes through what the host holds, in its order, by skips alone.
        for (index, key) in (1..).zip(&held[1..]) {
            f.send(PlayerCommand::Next)?;
            let next = f.hosted(key);
            expect_playing(&mut f.events, &next).await?;
            let batches = f.host.take(1).await?;
            assert_eq!(batches[0].actions, skipped_to(0, index).actions);
        }
        Ok(())
    })
    .await
}

#[tokio::test]
async fn under_repeat_all_the_run_wraps_round_to_the_current_track() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        f.send(PlayerCommand::SetPlaybackMode(PlaybackMode::RepeatAll))?;
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(1))?;
        expect_playing(&mut f.events, &queue[1]).await?;
        assert_eq!(
            f.host.take(3).await?,
            vec![handed_over("1441", 1), added(2, "1442"), added(3, "1440")]
        );
        f.settle().await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_local_track_moved_into_the_run_ends_it_there() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        start_run(f, &["1440", "1441", "1442", "1443"]).await?;
        f.queue(&[f.local(0)])?;
        f.settle().await?;
        assert!(f.host.sent_nothing());

        f.send(PlayerCommand::MovePlayListItem {
            old_index: 4,
            new_index: 2,
            revision: None,
        })?;
        assert_eq!(f.host.take(1).await?, vec![trimmed(5, &[3, 2])]);
        f.settle().await?;
        assert!(f.host.sent_nothing());
        assert_eq!(
            f.host.queue(),
            (keys(&["1440", "1441"]), Some("1440".to_owned()))
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_status_ahead_from_before_a_seek_is_stale() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = start_run(f, &["1440", "1441", "1442", "1443"]).await?;

        // The host moved on by three, and its report is on its way when the user seeks.
        let moved = f.host.move_on_by(3);
        f.send(PlayerCommand::Seek(30.0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        let skip = HostAction::Skip {
            index: 0,
            offset_ms: 30_000,
        };
        assert_eq!(f.host.take(1).await?, vec![on(5, skip)]);
        f.report(moved)?;
        f.report(f.host.status())?;
        let seen = f.settle().await?;
        assert_eq!(ends(&seen, &queue[0]), 0, "{seen:?}");
        assert!(
            !any(&seen, |event| matches!(
                event,
                PlayerEvent::Unavailable(..) | PlayerEvent::Playing { .. }
            )),
            "{seen:?}"
        );
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_status_several_entries_ahead_while_paused_leaves_the_track_paused() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = start_run(f, &["1440", "1441", "1442", "1443"]).await?;
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &queue[0]).await?;
        assert_eq!(f.host.take(1).await?, vec![on(5, HostAction::Stop)]);

        f.report(f.host.move_on_by(2))?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Paused { item, .. } if *item == queue[2]),
        )
        .await?;
        for played in &queue[..2] {
            assert_eq!(ends(&seen, played), 1, "{seen:?}");
        }
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Playing { .. })),
            "{seen:?}"
        );
        f.settle().await?;
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_end_after_the_host_played_through_its_run_counts_every_track() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let track = f.local(0);
        let hosted = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        let mut queue = hosted.to_vec();
        queue.push(track.clone());
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &hosted[0]).await?;
        f.host.take(3).await?;

        // The host played its whole run while the player did not hear from it, and the
        // local track after it plays.
        f.report(f.host.run_out())?;
        let seen = until(
            &mut f.events,
            |event| matches!(event, PlayerEvent::Playing { item, .. } if *item == track),
        )
        .await?;
        for played in &hosted {
            assert_eq!(ends(&seen, played), 1, "{seen:?}");
        }
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Unavailable(..))),
            "{seen:?}"
        );
        assert_eq!(f.host.take(1).await?, vec![released(4)]);
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_paused_hosted_track_reports_its_place_once() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.send(PlayerCommand::Pause)?;
        expect_paused(&mut f.events, &hosted).await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        f.settle().await?;

        // The place does not move while the track is paused, so nothing more is said.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let seen = f.settle().await?;
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Progress { .. })),
            "{seen:?}"
        );

        // A seek moves it, which is reported once.
        f.send(PlayerCommand::Seek(30.0))?;
        tokio::time::sleep(Duration::from_secs(5)).await;
        let seen = f.settle().await?;
        let reports = seen
            .iter()
            .filter_map(|event| match event {
                PlayerEvent::Progress { position, .. } => Some(*position),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(reports, vec![Duration::from_secs(30)], "{seen:?}");
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_load_takes_the_run_from_the_host_before_the_new_track_is_ready() -> Result<()> {
    let (internal, f) = engine_with_host(LONG_TRACK_SAMPLES, &[1]);
    with_engine(internal, f, async |f| {
        let track = f.local(0);
        let hosted = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        let mut queue = hosted.to_vec();
        queue.push(track.clone());
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &hosted[0]).await?;
        f.host.take(3).await?;

        // While the local track loads, the host holds the current track only, so it
        // cannot move on to the next one of its run.
        f.send(PlayerCommand::Switch(3))?;
        tokio::time::timeout(TEST_DEADLINE, f.started.notified()).await?;
        assert_eq!(f.host.take(1).await?, vec![trimmed(4, &[2, 1])]);
        assert_eq!(f.host.queue(), (keys(&["1440"]), Some("1440".to_owned())));

        f.release.notify_one();
        expect_playing(&mut f.events, &track).await?;
        assert_eq!(f.host.take(1).await?, vec![released(5)]);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn an_end_of_an_entry_before_the_current_one_changes_nothing() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = start_run(f, &["1440", "1441", "1442"]).await?;
        f.report(f.host.move_on())?;
        expect_playing(&mut f.events, &queue[1]).await?;

        // The end of the first entry arrives after the status that moved on from it.
        f.report(HostEvent::JukeboxEnded {
            host: HOST.to_owned(),
            revision: 3,
            index: Some(0),
        })?;
        let seen = f.settle().await?;
        assert_eq!(ends(&seen, &queue[1]), 0, "{seen:?}");
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Playing { .. })),
            "{seen:?}"
        );
        assert!(f.host.sent_nothing());
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn time_the_player_did_not_run_does_not_count_as_silence() -> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let (first, second) = (f.hosted("1440"), f.hosted("1441"));
        start_first(f, &[first.clone(), second.clone()], Some("1441")).await?;
        f.report(status(1, true, 0))?;
        f.settle().await?;

        // A minute passes in one step, as for an app that the system suspended.
        tokio::time::advance(Duration::from_secs(60)).await;
        f.settle().await?;
        f.assert_all_available();

        // The host still has as long to report as it had before.
        tokio::time::sleep(Duration::from_secs(14)).await;
        f.assert_all_available();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert_eq!(
            expect_unavailable(&mut f.events, &first).await?,
            UnavailableReason::SourceUnreachable
        );
        Ok(())
    })
    .await
}

// A command the player takes after a time it did not run, before its next progress tick, does
// not count that time against the host: only the tick judges the deadline, once it left it out.
#[tokio::test(start_paused = true)]
async fn a_command_after_a_time_the_player_did_not_run_leaves_the_host_its_time() -> Result<()> {
    let (mut internal, mut f) = hosted_engine(2, &[]);
    let hosted = f.hosted("1440");
    f.queue(std::slice::from_ref(&hosted))?;
    f.send(PlayerCommand::Switch(0))?;
    let event = run_until_track_state(&mut internal, &mut f.events).await?;
    assert!(
        matches!(&event, PlayerEvent::Playing { item, .. } if *item == hosted),
        "{event:?}"
    );
    f.report(status(1, true, 0))?;
    tokio::select! {
        result = internal.run() => bail!("the player stopped: {result:?}"),
        seen = f.settle() => drop(seen?),
    }

    // A minute passes in one step, and the player takes a command before its next tick.
    tokio::time::advance(Duration::from_secs(60)).await;
    internal.set_playback_mode(PlaybackMode::RepeatAll)?;
    while let Ok(event) = f.events.try_recv() {
        assert!(!matches!(event, PlayerEvent::Unavailable(..)), "{event:?}");
    }
    assert!(internal.hosted.is_some());
    let _ = std::fs::remove_file(&f.wav_path);
    f.host.check()
}

// A host that comes up again after a time the player did not run, and whose report the player
// takes before its next tick, gets its track back where the track stood when the player stopped.
#[tokio::test(start_paused = true)]
async fn a_host_that_comes_up_after_a_time_the_player_did_not_run_gets_the_track_where_it_stood()
-> Result<()> {
    let (internal, f) = hosted_engine(2, &[]);
    with_engine(internal, f, async |f| {
        let hosted = f.hosted("1440");
        start_first(f, std::slice::from_ref(&hosted), None).await?;
        f.report(status(1, true, 10_000))?;
        f.settle().await?;

        // The host's report waits as a minute passes in one step, as for an app that the system
        // suspended, and the player takes it before the tick that fell due meanwhile.
        f.report(f.host.restart())?;
        tokio::time::advance(Duration::from_secs(60)).await;
        let batches = f.host.take(1).await?;
        let Some(HostAction::Skip { offset_ms, .. }) = batches[0].actions.get(1) else {
            bail!("{batches:?}");
        };
        assert!(*offset_ms < 11_000, "the place ran on to {offset_ms} ms");
        assert_eq!(
            batches,
            vec![hand_over(HOST, &["1440"], 2, *offset_ms, 1.0, true)]
        );
        Ok(())
    })
    .await
}

#[tokio::test(start_paused = true)]
async fn a_host_that_comes_up_again_gets_its_whole_run_again() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(3).await?;
        f.report(status(3, true, 5_000))?;
        f.settle().await?;
        tokio::time::advance(Duration::from_secs(2)).await;

        f.report(f.host.restart())?;
        assert_eq!(
            f.host.take(1).await?,
            vec![hand_over(
                HOST,
                &["1440", "1441", "1442"],
                4,
                7_000,
                1.0,
                true
            )]
        );
        f.report(f.host.move_on_by(2))?;
        expect_playing(&mut f.events, &queue[2]).await?;
        f.assert_all_available();
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_track_the_host_fails_inside_the_run_ends_the_run_there() -> Result<()> {
    let (internal, f) = run_engine();
    with_engine(internal, f, async |f| {
        let queue = ["1440", "1441", "1442", "1443"].map(|key| f.hosted(key));
        f.queue(&queue)?;
        f.send(PlayerCommand::Switch(0))?;
        expect_playing(&mut f.events, &queue[0]).await?;
        f.host.take(4).await?;

        // The host cannot play the third track, so it and the fourth leave its queue.
        f.report(failed_at(4, 2, HostFailure::Network))?;
        assert_eq!(f.host.take(1).await?, vec![trimmed(5, &[3, 2])]);
        let mut seen = f.settle().await?;
        assert!(f.host.sent_nothing());

        f.report(f.host.move_on())?;
        expect_playing(&mut f.events, &queue[1]).await?;
        seen.extend(f.settle().await?);
        assert!(f.host.sent_nothing());
        assert!(
            !any(&seen, |event| matches!(event, PlayerEvent::Unavailable(..))),
            "{seen:?}"
        );

        // Once current, the third track is handed over anew, and the run goes on.
        f.report(f.host.end())?;
        expect_playing(&mut f.events, &queue[2]).await?;
        assert_eq!(
            f.host.take(2).await?,
            vec![handed_over("1442", 6), added(7, "1443")]
        );
        Ok(())
    })
    .await
}

// A player whose resolver hosts the tracks of the returned library on `HOST`, and whose
// local track is `local_uuid`. Its status is followed from the start.
fn hosted_player(
    local_uuid: Uuid,
) -> (
    Player,
    SimpleReceiver<PlayerStatus>,
    Arc<FakeHost>,
    Uuid,
    PathBuf,
) {
    let (local, wav_path) = delayed_resolver_fixture_with_samples(
        &[local_uuid],
        Arc::new(Mutex::new(Vec::new())),
        Arc::new(Notify::new()),
        Arc::new(Notify::new()),
        &[],
        LONG_TRACK_SAMPLES,
    );
    let hosted_library = Uuid::new_v4();
    let host = FakeHost::new();
    let player = Player::new(Arc::new(FsIo::new_noop()), None);
    let statuses = player.subscribe_status();
    player.set_source_resolver(Arc::new(MixedResolver {
        local,
        hosted_library,
        slow: Arc::default(),
        down: Arc::default(),
        streams: mpsc::unbounded_channel().0,
        failed: Arc::default(),
    }));
    player.set_host_bridge(host.clone());
    (player, statuses, host, hosted_library, wav_path)
}

async fn status_until(
    statuses: &SimpleReceiver<PlayerStatus>,
    wanted: impl Fn(&PlayerStatus) -> bool,
) -> Result<PlayerStatus> {
    tokio::time::timeout(TEST_DEADLINE, async {
        loop {
            match statuses.recv().await {
                Ok(status) if wanted(&status) => return Ok(status),
                Ok(_) => {}
                Err(error) => bail!("the player status closed: {error:?}"),
            }
        }
    })
    .await?
}

#[tokio::test]
async fn the_status_names_the_host_while_a_hosted_track_is_current() -> Result<()> {
    let local_uuid = Uuid::new_v4();
    let (player, statuses, host, hosted_library, wav_path) = hosted_player(local_uuid);
    let hosted = PlayingItem::LibraryTrack {
        library_id: hosted_library,
        track: "1440".to_owned(),
    };
    let track = library_item(Uuid::new_v4(), local_uuid);
    player.add_to_playlist_with_sources(
        vec![
            (hosted.clone(), PlaybackSource::LibraryStream),
            (track.clone(), PlaybackSource::LibraryStream),
        ],
        AddMode::AppendToEnd,
    );
    player.switch(0);
    let status = status_until(&statuses, |status| {
        status.item.as_ref() == Some(&hosted) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(status.hosted_by.as_deref(), Some(HOST));
    assert_eq!(host.take(1).await?, vec![handed_over("1440", 1)]);

    player.deliver_host_event(ended(1));
    let status = status_until(&statuses, |status| {
        status.item.as_ref() == Some(&track) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(status.hosted_by, None);

    player.terminate();
    let _ = std::fs::remove_file(wav_path);
    host.check()
}

#[tokio::test]
async fn the_status_says_whether_the_host_shows_itself_in_the_media_controls() -> Result<()> {
    let local_uuid = Uuid::new_v4();
    let (player, statuses, host, hosted_library, wav_path) = hosted_player(local_uuid);
    let hosted = PlayingItem::LibraryTrack {
        library_id: hosted_library,
        track: "1440".to_owned(),
    };
    let track = library_item(Uuid::new_v4(), local_uuid);
    player.deliver_host_event(HostEvent::HostReady {
        host: HOST.to_owned(),
        authorized: true,
        can_play: true,
        shows_in_media_controls: false,
    });
    player.add_to_playlist_with_sources(
        vec![
            (hosted.clone(), PlaybackSource::LibraryStream),
            (track.clone(), PlaybackSource::LibraryStream),
        ],
        AddMode::AppendToEnd,
    );
    player.switch(0);
    let status = status_until(&statuses, |status| {
        status.item.as_ref() == Some(&hosted) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(status.hosted_by.as_deref(), Some(HOST));
    assert!(!status.host_shows_in_media_controls);
    assert_eq!(host.take(1).await?, vec![handed_over("1440", 1)]);

    // Started again without a word of it, the host shows itself there, and gets its track again.
    let started =
        HostEvent::parse(r#"{"type":"hostReady","host":"host","authorized":true,"canPlay":true}"#)
            .expect("a hostReady");
    player.deliver_host_event(started);
    let status = status_until(&statuses, |status| status.host_shows_in_media_controls).await?;
    assert_eq!(status.hosted_by.as_deref(), Some(HOST));
    host.take(1).await?;

    // No host plays the track that follows, and none shows itself for it.
    player.deliver_host_event(ended(2));
    let status = status_until(&statuses, |status| {
        status.item.as_ref() == Some(&track) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(status.hosted_by, None);
    assert!(!status.host_shows_in_media_controls);

    player.terminate();
    let _ = std::fs::remove_file(wav_path);
    host.check()
}

#[tokio::test]
async fn a_track_the_host_moved_on_from_counts_as_played_through() -> Result<()> {
    let (player, statuses, host, hosted_library, wav_path) = hosted_player(Uuid::new_v4());
    let played_through = player.subscribe_played_through();
    let hosted = |key: &str| PlayingItem::LibraryTrack {
        library_id: hosted_library,
        track: key.to_owned(),
    };
    player.add_to_playlist_with_sources(
        vec![
            (hosted("1440"), PlaybackSource::LibraryStream),
            (hosted("1441"), PlaybackSource::LibraryStream),
        ],
        AddMode::AppendToEnd,
    );
    player.switch(0);
    status_until(&statuses, |status| {
        status.item.as_ref() == Some(&hosted("1440")) && status.state == PlaybackState::Playing
    })
    .await?;
    assert_eq!(
        host.take(2).await?,
        vec![handed_over("1440", 1), added(2, "1441")]
    );

    player.deliver_host_event(host.move_on());
    let played = tokio::time::timeout(TEST_DEADLINE, played_through.recv()).await?;
    assert_eq!(played.ok().map(|played| played.item), Some(hosted("1440")));
    let status = status_until(&statuses, |status| {
        status.item.as_ref() == Some(&hosted("1441"))
    })
    .await?;
    assert_eq!(status.index, Some(1));
    assert_eq!(status.state, PlaybackState::Playing);
    assert_eq!(status.hosted_by.as_deref(), Some(HOST));
    assert!(host.sent_nothing());

    player.terminate();
    let _ = std::fs::remove_file(wav_path);
    host.check()
}

#[tokio::test]
async fn every_track_the_host_played_while_the_player_did_not_hear_counts() -> Result<()> {
    let (player, statuses, host, hosted_library, wav_path) = hosted_player(Uuid::new_v4());
    let played_through = player.subscribe_played_through();
    let keys = (0..RUN_LIMIT)
        .map(|key| format!("{}", 1400 + key))
        .collect::<Vec<_>>();
    let hosted = |key: &str| PlayingItem::LibraryTrack {
        library_id: hosted_library,
        track: key.to_owned(),
    };
    player.add_to_playlist_with_sources(
        keys.iter()
            .map(|key| (hosted(key), PlaybackSource::LibraryStream))
            .collect(),
        AddMode::AppendToEnd,
    );
    player.switch(0);
    host.take(RUN_LIMIT).await?;

    // The host played on through all but the last track of its run before the player
    // heard from it again.
    let last = &keys[RUN_LIMIT - 1];
    player.deliver_host_event(host.move_on_by(RUN_LIMIT - 1));
    status_until(&statuses, |status| {
        status.item.as_ref() == Some(&hosted(last))
    })
    .await?;
    let mut played = Vec::new();
    while let Ok(Ok(through)) =
        tokio::time::timeout(Duration::from_millis(200), played_through.recv()).await
    {
        played.push(through.item);
    }
    let expected = keys[..RUN_LIMIT - 1]
        .iter()
        .map(|key| hosted(key))
        .collect::<Vec<_>>();
    assert_eq!(played, expected);

    player.terminate();
    let _ = std::fs::remove_file(wav_path);
    host.check()
}
