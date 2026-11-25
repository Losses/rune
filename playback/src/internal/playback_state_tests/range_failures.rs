use super::*;
use crate::player::{AudioRequest, LibraryPlaybackSource, RemoteAudioSource, UnavailableReason};
use anyhow::ensure;
use futures::future::BoxFuture;
use reqwest::header;
use std::net::TcpListener as StdTcpListener;
use tokio::task;
use tokio::time as tokio_time;

/// The responses of a peer's audio endpoint to ranged requests, correct or broken in one of
/// the ways a faulty peer broke them. A range whose start passes the first byte is corrupt,
/// everything else is correct.
#[derive(Clone, Copy, PartialEq, Eq)]
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
        // over a network does: reads and seeks that outrun it ask for their own ranges.
        for chunk in body.chunks(1024) {
            stream.write_all(chunk)?;
            stream.flush()?;
            std::thread::sleep(Duration::from_millis(20));
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
            if let Err(error) = validate_range_response(&response, size, range) {
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

fn validate_range_response(
    response: &reqwest::Response,
    size: u64,
    range: Option<(u64, Option<u64>)>,
) -> Result<()> {
    let expected_length = if let Some((start, end)) = range {
        ensure!(start < size, "audio range starts beyond content");
        let last = end.unwrap_or(size - 1).min(size - 1);
        ensure!(last >= start, "invalid audio range");
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
            bail!(RemoteAudioError::unreachable(format!(
                "unexpected status {}",
                response.status()
            )));
        }
        let expected = format!("bytes {start}-{last}/{size}");
        if response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            != Some(expected.as_str())
        {
            bail!(RemoteAudioError::unreachable(format!(
                "content range does not match {expected}"
            )));
        }
        last - start + 1
    } else {
        if response.status() != reqwest::StatusCode::OK {
            bail!(RemoteAudioError::unreachable(format!(
                "unexpected status {}",
                response.status()
            )));
        }
        size
    };
    if response.content_length() != Some(expected_length) {
        bail!(RemoteAudioError::Incomplete);
    }
    Ok(())
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

            // A playback failure must not pass silently: the play request hears a typed failure,
            // the queue hears the track is unavailable, and the application log hears what
            // happened.
            let outcome = tokio_time::timeout(TEST_DEADLINE, play).await??;
            assert_eq!(
                outcome,
                Err(PlayerCommandError::SourceUnavailable),
                "the play request was not answered with the typed source failure: {outcome:?}"
            );

            let mut unavailable = false;
            let mut logged = false;
            tokio_time::timeout(TEST_DEADLINE, async {
                while !(unavailable && logged) {
                    match events.recv().await {
                        Some(PlayerEvent::Unavailable(item, UnavailableReason::SourceUnreachable))
                            if item == track =>
                        {
                            unavailable = true;
                        }
                        Some(PlayerEvent::Log(log)) => {
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
                        Some(_) => {}
                        None => bail!("the player's event channel closed"),
                    }
                }
                Ok::<_, anyhow::Error>(())
            })
            .await
            .context(format!(
                "the playback failure did not reach the events it must: unavailable reported: {unavailable}, application log entry reported: {logged}"
            ))??;

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
