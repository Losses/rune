use super::*;
use crate::player::{
    AudioRequest, LibraryPlaybackSource, RemoteAudioSource, UnavailableReason,
    validate_audio_response,
};
use anyhow::{Context, ensure};
use futures::future::BoxFuture;
use reqwest::header;
use std::collections::HashMap;
use std::net::TcpListener as StdTcpListener;
use tokio::task;
use tokio::time as tokio_time;

/// The responses of a peer's audio endpoint to ranged requests, correct or broken in one of
/// the ways a faulty peer broke them. A range whose start passes the first byte is corrupt,
/// everything else is correct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Corruption {
    /// The peer answers every request correctly.
    Correct,
    /// The peer answers a ranged request with 200 and the whole body.
    WrongStatus,
    /// The peer answers with a content range that does not name the asked range.
    WrongContentRange,
    /// The peer answers with fewer bytes than its content range names.
    ShortBody,
}

/// Serves `data` over bare HTTP, correct except as `corruption` says. Returns its base URL.
fn audio_server(data: &'static [u8], corruption: Corruption) -> Result<String> {
    let listener = StdTcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            thread::spawn(move || {
                let _ = serve_audio(stream, data, corruption);
            });
        }
    });
    Ok(format!("http://127.0.0.1:{port}/audio"))
}

fn serve_audio(mut stream: std::net::TcpStream, data: &[u8], corruption: Corruption) -> Result<()> {
    use std::io::{Read, Write};
    stream.set_nonblocking(false)?;
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1024];
    loop {
        if buffer.windows(4).any(|end| end == b"\r\n\r\n") {
            break;
        }
        let read = stream.read(&mut byte)?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&byte[..read]);
        if buffer.len() > 8192 {
            bail!("audio request too large");
        }
    }
    let head = String::from_utf8_lossy(&buffer).to_string();
    let range = head.lines().find_map(|line| {
        let value = line
            .strip_prefix("Range:")
            .or_else(|| line.strip_prefix("range:"))?
            .trim()
            .strip_prefix("bytes=")?;
        let (start, end) = value.split_once('-')?;
        let start = start.trim().parse::<u64>().ok()?;
        let end = end.trim();
        let end = if end.is_empty() {
            None
        } else {
            end.parse::<u64>().ok()
        };
        Some((start, end))
    });
    let size = data.len() as u64;
    let corruptible = range.is_some_and(|(start, _)| start > 0);
    let (status, start, last) = if let Some((start, end)) = range {
        let last = end.unwrap_or(size - 1).min(size - 1);
        if corruption == Corruption::WrongStatus && corruptible {
            ("200 OK", 0, size - 1)
        } else {
            ("206 Partial Content", start, last)
        }
    } else {
        ("200 OK", 0, size - 1)
    };
    let length = last - start + 1;
    let (content_range, length) = if corruptible && corruption == Corruption::WrongContentRange {
        // The body matches the asked range, but its header names a different start.
        (
            format!("bytes {}-{last}/{size}", start.saturating_sub(1)),
            length,
        )
    } else if corruptible && corruption == Corruption::ShortBody {
        (
            format!("bytes {start}-{last}/{size}"),
            length.saturating_sub(1),
        )
    } else {
        (format!("bytes {start}-{last}/{size}"), length)
    };
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n"
    );
    if status.starts_with("206") {
        response.push_str(&format!("Content-Range: {content_range}\r\n"));
    }
    response.push_str("\r\n");
    stream.write_all(response.as_bytes())?;
    let end = (start + length).min(size);
    let body = &data[start as usize..end as usize];
    if range.is_none() {
        // The full response of the initial request streams in slowly, as the audio of a peer
        // over a network does. The throttle is far slower than any decoder probe: a probe that
        // passes the downloaded part always asks a range before the stream could have completed.
        // 64 bytes every 10ms is about 17 seconds for this file, against a probe that takes
        // milliseconds.
        for chunk in body.chunks(64) {
            stream.write_all(chunk)?;
            stream.flush()?;
            std::thread::sleep(Duration::from_millis(10));
        }
    } else {
        stream.write_all(body)?;
    }
    stream.flush()?;
    Ok(())
}

/// Validates a peer's audio responses the way the syncing worker does, so that the playback
/// engine in these tests refuses the same corrupt responses the real peer client refuses.
/// (The real validation runs inside the worker's TLS client; these tests exercise the playback
/// half of the fault contract over plain HTTP.)
struct ValidatingRequest {
    url: String,
    size: u64,
    failure_reason: Arc<Mutex<Option<UnavailableReason>>>,
}

impl AudioRequest for ValidatingRequest {
    fn get(
        &self,
        range: Option<(u64, Option<u64>)>,
    ) -> BoxFuture<'static, Result<reqwest::Response>> {
        let url = self.url.clone();
        let size = self.size;
        let failure_reason = self.failure_reason.clone();
        Box::pin(async move {
            let client = reqwest::Client::new();
            let request = client.get(&url);
            let request = if let Some((start, end)) = range {
                let range_header = match end {
                    Some(end) => format!("bytes={start}-{end}"),
                    None => format!("bytes={start}-"),
                };
                request.header(header::RANGE, range_header)
            } else {
                request
            };
            let response = request.send().await?;
            if let Err(error) = validate_audio_response(&response, size, range) {
                // As the worker's audio client does: the typed cause cannot survive the io
                // errors of the transport, so the failure slot of the track carries it.
                if let Ok(mut reason) = failure_reason.lock() {
                    reason.get_or_insert(UnavailableReason::classify(&error));
                }
                return Err(error);
            }
            Ok(response)
        })
    }
}

#[derive(Clone)]
struct RangeFailureResolver {
    url: String,
    size: u64,
    file_uuid: Uuid,
    id: i32,
}

impl PlaybackSourceResolver for RangeFailureResolver {
    fn for_library(&self, _: String) -> BoxFuture<'static, Result<Option<LibraryPlaybackSource>>> {
        let resolver = self.clone();
        Box::pin(async move {
            Ok(Some(LibraryPlaybackSource {
                fsio: Arc::new(FsIo::new_noop()),
                resolver: Arc::new(resolver),
            }))
        })
    }

    fn resolve_id(&self, file_uuid: Uuid) -> BoxFuture<'static, Result<Option<i32>>> {
        let id = (file_uuid == self.file_uuid).then_some(self.id);
        Box::pin(async move { Ok(id) })
    }

    fn requires_stream(&self, _: i32) -> BoxFuture<'static, Result<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn resolve(&self, _: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        let url = self.url.clone();
        let size = self.size;
        Box::pin(async move {
            let failure_reason = Arc::new(Mutex::new(None));
            Ok(RemoteAudioSource {
                request: Arc::new(ValidatingRequest {
                    url: url.clone(),
                    size,
                    failure_reason: failure_reason.clone(),
                }),
                url,
                extension: "ogg".into(),
                size,
                cancellation: CancellationToken::new(),
                failure_reason,
            })
        })
    }
}

/// A resolver for several tracks, each with its own server and corruption.
struct MultiRangeResolver {
    tracks: std::collections::HashMap<Uuid, (String, i32)>,
}

impl PlaybackSourceResolver for MultiRangeResolver {
    fn for_library(&self, _: String) -> BoxFuture<'static, Result<Option<LibraryPlaybackSource>>> {
        let tracks: Vec<(Uuid, String, i32)> = self
            .tracks
            .iter()
            .map(|(uuid, (url, id))| (*uuid, url.clone(), *id))
            .collect();
        Box::pin(async move {
            let resolver = Arc::new(MultiRangeResolver {
                tracks: tracks
                    .into_iter()
                    .map(|(u, url, id)| (u, (url, id)))
                    .collect(),
            });
            Ok(Some(LibraryPlaybackSource {
                fsio: Arc::new(FsIo::new_noop()),
                resolver,
            }))
        })
    }

    fn resolve_id(&self, file_uuid: Uuid) -> BoxFuture<'static, Result<Option<i32>>> {
        let id = self.tracks.get(&file_uuid).map(|(_, id)| *id);
        Box::pin(async move { Ok(id) })
    }

    fn requires_stream(&self, _: i32) -> BoxFuture<'static, Result<bool>> {
        Box::pin(async { Ok(true) })
    }

    fn resolve(&self, id: i32) -> BoxFuture<'static, Result<RemoteAudioSource>> {
        let url = self
            .tracks
            .values()
            .find(|(_, track_id)| *track_id == id)
            .map(|(url, _)| url.clone());
        let size = STARTUP_OGG.len() as u64;
        Box::pin(async move {
            let url = url.context("unknown track")?;
            let failure_reason = Arc::new(Mutex::new(None));
            Ok(RemoteAudioSource {
                request: Arc::new(ValidatingRequest {
                    url: url.clone(),
                    size,
                    failure_reason: failure_reason.clone(),
                }),
                url,
                extension: "ogg".into(),
                size,
                cancellation: CancellationToken::new(),
                failure_reason,
            })
        })
    }
}

fn remote_multi_engine(
    resolver: Arc<MultiRangeResolver>,
) -> (
    PlayerInternal,
    mpsc::UnboundedSender<PlayerCommand>,
    mpsc::UnboundedReceiver<PlayerEvent>,
) {
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let (event_sender, events) = mpsc::unbounded_channel();
    let internal = PlayerInternal::new(
        commands,
        event_sender,
        CancellationToken::new(),
        commands_sender.clone(),
        Arc::new(FsIo::new_noop()),
        PcmTap::new(),
        Arc::new(RwLock::new(Some(resolver))),
        Arc::new(TestOutputs::default()),
    );
    (internal, commands_sender, events)
}

const STARTUP_OGG: &[u8] = include_bytes!("../../../../assets/startup_0.ogg");

fn remote_engine(
    resolver: Arc<RangeFailureResolver>,
) -> (
    PlayerInternal,
    mpsc::UnboundedSender<PlayerCommand>,
    mpsc::UnboundedReceiver<PlayerEvent>,
) {
    let (commands_sender, commands) = mpsc::unbounded_channel();
    let (event_sender, events) = mpsc::unbounded_channel();
    let internal = PlayerInternal::new(
        commands,
        event_sender,
        CancellationToken::new(),
        commands_sender.clone(),
        Arc::new(FsIo::new_noop()),
        PcmTap::new(),
        Arc::new(RwLock::new(Some(resolver))),
        Arc::new(TestOutputs::default()),
    );
    (internal, commands_sender, events)
}

async fn range_failure_scenario(corruption: Corruption) -> Result<()> {
    let url = audio_server(STARTUP_OGG, corruption)?;
    let file_uuid = Uuid::new_v4();
    let file_uuids = [file_uuid];
    let resolver = Arc::new(RangeFailureResolver {
        url,
        size: STARTUP_OGG.len() as u64,
        file_uuid,
        id: 1,
    });
    let (mut internal, commands_sender, mut events) = remote_engine(resolver);
    let local = LocalSet::new();
    local
        .run_until(async move {
            let runner = task::spawn_local(async move { internal.run().await });
            let library_id = Uuid::new_v4();
            commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
            let play = completed_command(&commands_sender, PlayerCommand::Play);
            let track = library_item(library_id, file_uuid);

            // The corruption hits the first ranged request, and whether that request belongs to
            // the probe or to playing is timing: a decoder that seeks while probing fails the
            // load itself, a linearly reading decoder loads fine and meets the corruption only
            // once a seek lands ahead of the throttled download. Both orders are the one fault
            // contract, so the play answer may be either, and the scenario drives the second
            // order itself: a seek near the end always asks a range, because the download is
            // far slower than any playback.
            let play_outcome = tokio_time::timeout(TEST_DEADLINE, play)
                .await?
                .context("the play completion channel closed unanswered")?;
            if let Err(error) = &play_outcome {
                assert_eq!(
                    *error,
                    PlayerCommandError::SourceUnavailable,
                    "the play request failed with something else than the typed source failure: {error:?}"
                );
            }

            // The one event ordering this test does not fix is whether the corruption strikes
            // the probe or the playing track; both orders surface the same events.
            let mut seeked = false;
            let mut unavailable = false;
            let mut logged = false;
            let deadline = tokio_time::Instant::now() + TEST_DEADLINE;
            while !(unavailable && logged) {
                ensure!(
                    tokio_time::Instant::now() < deadline,
                    "the playback failure did not reach the events it must: unavailable reported:                      {unavailable}, application log entry reported: {logged}, play outcome:                      {play_outcome:?}"
                );
                match tokio_time::timeout(Duration::from_millis(100), events.recv()).await {
                    Ok(Some(PlayerEvent::Unavailable(item, UnavailableReason::SourceUnreachable)))
                        if item == track =>
                    {
                        unavailable = true;
                    }
                    Ok(Some(PlayerEvent::Log(log))) => {
                        assert_eq!(
                            log.domain, "playback",
                            "the playback failure was logged under the wrong domain"
                        );
                        assert_eq!(
                            log.item.as_ref(),
                            Some(&track),
                            "the playback failure was logged without its track"
                        );
                        assert_eq!(
                            log.failure,
                            Some(UnavailableReason::SourceUnreachable),
                            "the playback failure was logged without its typed reason"
                        );
                        logged = true;
                    }
                    Ok(Some(PlayerEvent::Playing { item, .. })) if item == track => {
                        // The decoder read linearly past the probe, so the corruption has not
                        // been asked for yet. A seek near the end of the track lands ahead of
                        // the throttled download for certain and asks the range that is.
                        if play_outcome.is_ok() && !seeked {
                            seeked = true;
                            commands_sender.send(PlayerCommand::Seek(8.0))?;
                        }
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => bail!("the player's event channel closed"),
                    Err(_) => {}
                }
            }

            runner.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

/// The broadcast decision rides the log entry: the first failure of a chain is reported, and a
/// failure playback reaches by advancing past the reported one is a chain continuation, so the
/// user-facing failure report is one per chain instead of one per track skipped.
#[tokio::test]
async fn failures_of_one_chain_tell_the_user_once() -> Result<()> {
    // Two corrupt tracks ahead of a healthy one: the first failure answers the play request, and
    // the second comes from the chain the first starts.
    let url = audio_server(STARTUP_OGG, Corruption::WrongStatus)?;
    let file_uuid = Uuid::new_v4();
    let second_uuid = Uuid::new_v4();
    let third_url = audio_server(STARTUP_OGG, Corruption::Correct)?;
    let third_uuid = Uuid::new_v4();
    let resolver = Arc::new(MultiRangeResolver {
        tracks: [
            (file_uuid, (url.clone(), 1)),
            (second_uuid, (url, 1)),
            (third_uuid, (third_url, 2)),
        ]
        .into_iter()
        .collect(),
    });
    let (mut internal, commands_sender, mut events) = remote_multi_engine(resolver);
    let local = LocalSet::new();
    local
        .run_until(async move {
            let runner = task::spawn_local(async move { internal.run().await });
            let library_id = Uuid::new_v4();
            commands_sender.send(add_library_tracks(
                library_id,
                &[file_uuid, second_uuid, third_uuid],
            ))?;
            let play = completed_command(&commands_sender, PlayerCommand::Play);
            let play_outcome = tokio_time::timeout(TEST_DEADLINE, play)
                .await?
                .context("the play completion channel closed unanswered")?;
            let first = library_item(library_id, file_uuid);
            let second = library_item(library_id, second_uuid);

            let mut continuations = HashMap::new();
            let mut seeked = Vec::new();
            let mut retry: Option<oneshot::Receiver<result::Result<(), PlayerCommandError>>> = None;
            tokio_time::timeout(TEST_DEADLINE, async {
                while continuations.len() < 2 {
                    match events.recv().await {
                        Some(PlayerEvent::Log(log))
                            if log.item.as_ref() == Some(&first)
                                || log.item.as_ref() == Some(&second) =>
                        {
                            if log.item.as_ref() == Some(&first) && retry.is_none() {
                                // A play pressed as the chain advances rides the load of the
                                // track the chain moves to: when that load fails, the chain the
                                // user was already told about must not fail the press again.
                                retry = Some(completed_command(&commands_sender, PlayerCommand::Play));
                            }
                            continuations.insert(log.item.clone().unwrap(), log.chain_continuation);
                        }
                        Some(PlayerEvent::Playing { item, .. })
                            if item == first || item == second =>
                        {
                            // A decoder can read linearly past the probe, so the corruption
                            // has not been asked for yet. A seek near the end lands ahead of
                            // the throttled download for certain and asks the range that is,
                            // for every track that comes up playing.
                            if play_outcome.is_ok() && !seeked.contains(&item) {
                                seeked.push(item);
                                commands_sender.send(PlayerCommand::Seek(8.0))?;
                            }
                        }
                        Some(_) => {}
                        None => bail!("the player's event channel closed"),
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context("the chain's failures were never logged")??;
            if let Some(retry) = retry {
                assert_eq!(
                    tokio_time::timeout(TEST_DEADLINE, retry)
                        .await?
                        .context("the retry completion channel closed unanswered")?,
                    Ok(()),
                    "a chain continuation must not answer a retrying request with the failure the chain already told"
                );
            }
            assert_eq!(
                continuations.get(&first),
                Some(&false),
                "the first failure of the chain must be reported"
            );
            assert_eq!(
                continuations.get(&second),
                Some(&true),
                "a failure the chain advances to must be a chain continuation"
            );

            runner.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_correct_range_response_does_not_fail_playback() -> Result<()> {
    let url = audio_server(STARTUP_OGG, Corruption::Correct)?;
    let file_uuid = Uuid::new_v4();
    let file_uuids = [file_uuid];
    let resolver = Arc::new(RangeFailureResolver {
        url,
        size: STARTUP_OGG.len() as u64,
        file_uuid,
        id: 1,
    });
    let (mut internal, commands_sender, mut events) = remote_engine(resolver);
    let local = LocalSet::new();
    local
        .run_until(async move {
            let runner = task::spawn_local(async move { internal.run().await });
            let library_id = Uuid::new_v4();
            commands_sender.send(add_library_tracks(library_id, &file_uuids))?;
            let play = completed_command(&commands_sender, PlayerCommand::Play);
            assert_eq!(
                tokio_time::timeout(TEST_DEADLINE, play).await??,
                Ok(()),
                "correct range responses failed the load"
            );
            tokio_time::timeout(
                TEST_DEADLINE,
                expect_playing(&mut events, &library_item(library_id, file_uuid)),
            )
            .await??;
            runner.abort();
            Ok::<(), anyhow::Error>(())
        })
        .await?;
    Ok(())
}

#[tokio::test]
async fn a_range_answered_with_200_surfaces_a_typed_failure() -> Result<()> {
    range_failure_scenario(Corruption::WrongStatus).await
}

#[tokio::test]
async fn a_content_range_mismatch_surfaces_a_typed_failure() -> Result<()> {
    range_failure_scenario(Corruption::WrongContentRange).await
}

#[tokio::test]
async fn a_short_content_range_body_surfaces_a_typed_failure() -> Result<()> {
    range_failure_scenario(Corruption::ShortBody).await
}
