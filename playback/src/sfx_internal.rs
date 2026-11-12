use crate::decoder::RuneDecoder;
use std::{fs::File, io::BufReader, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use log::{debug, error, info};
use rodio::{OutputStream, Sink};
use tokio::{sync::mpsc, time::interval};
use tokio_util::sync::CancellationToken;

// How often the player looks whether its sound has finished.
const FINISHED_CHECK: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub enum SfxPlayerCommand {
    Load { path: PathBuf },
    SetVolume(f32),
}

#[derive(Debug, PartialEq)]
enum InternalSfxPlaybackState {
    Playing,
    Stopped,
    Empty,
}

pub(crate) struct SfxPlayerInternal {
    commands: mpsc::UnboundedReceiver<SfxPlayerCommand>,
    current_track_path: Option<PathBuf>,
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
    state: InternalSfxPlaybackState,
    cancellation_token: CancellationToken,
    volume: f32,
}

impl SfxPlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<SfxPlayerCommand>,
        cancellation_token: CancellationToken,
    ) -> Self {
        Self {
            commands,
            current_track_path: None,
            sink: None,
            _stream: None,
            state: InternalSfxPlaybackState::Stopped,
            cancellation_token,
            volume: 1.0,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut finished_check = interval(FINISHED_CHECK);
        loop {
            if self.cancellation_token.is_cancelled() {
                break;
            }

            let cmd = tokio::select! {
                _ = self.cancellation_token.cancelled() => break,
                _ = finished_check.tick(), if self.sink.is_some() => {
                    self.close_finished_output();
                    continue;
                },
                cmd = self.commands.recv() => cmd,
            };
            if let Some(cmd) = cmd {
                if self.cancellation_token.is_cancelled() {
                    debug!("Cancellation token triggered, exiting run loop");
                    if let Some(sink) = &self.sink {
                        sink.stop();
                    }
                    break;
                }

                debug!("Received command: {cmd:?}");
                match cmd {
                    SfxPlayerCommand::Load { path } => self.load(Some(path)),
                    SfxPlayerCommand::SetVolume(volume) => self.set_volume(volume),
                }?;
            } else {
                break;
            }
        }

        Ok(())
    }

    fn load(&mut self, path: Option<PathBuf>) -> Result<()> {
        if let Some(sink) = &self.sink {
            sink.stop();
        }
        self.sink = None;
        self._stream = None;

        if let Some(path) = path {
            debug!("Loading track at index: {:?}", path.clone());
            let file = File::open(path.clone())
                .with_context(|| format!("Failed to open file: {:?}", path.clone()))?;
            let ext = path.extension().and_then(|e| e.to_str());
            let source = RuneDecoder::new(BufReader::new(file), ext)
                .with_context(|| "Failed to decode audio")?;
            let (stream, stream_handle) =
                OutputStream::try_default().context("Failed to create output stream")?;
            let sink = Sink::try_new(&stream_handle).context("Failed to create sink")?;

            sink.set_volume(self.volume);
            sink.append(source);

            self.sink = Some(sink);
            self._stream = Some(stream);
            self.current_track_path = Some(path.clone());
            self.state = InternalSfxPlaybackState::Playing;
            info!("SFX track loaded and playing: {path:?}");
        } else {
            error!("Load command received without index");
        }
        Ok(())
    }

    // Closes the output once its sound has finished. An open output keeps the device's audio
    // running, and on iOS the system then takes the app for one that plays.
    fn close_finished_output(&mut self) {
        if self.sink.as_ref().is_some_and(Sink::empty) {
            self.sink = None;
            self._stream = None;
            self.state = InternalSfxPlaybackState::Empty;
            debug!("SFX finished, so its output is closed");
        }
    }

    fn set_volume(&mut self, volume: f32) -> Result<()> {
        self.volume = volume;
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;
    use rodio::source::SineWave;

    // Advances the player's timer through several finished-output checks.
    async fn run_briefly(
        internal: &mut SfxPlayerInternal,
        commands_sender: &mpsc::UnboundedSender<SfxPlayerCommand>,
    ) {
        let cancellation = internal.cancellation_token.clone();
        {
            let run = internal.run();
            tokio::pin!(run);
            for _ in 0..3 {
                assert!(!commands_sender.is_closed());
                tokio::select! {
                    _ = tokio::time::advance(FINISHED_CHECK) => {},
                    result = &mut run => panic!("SFX player stopped unexpectedly: {result:?}"),
                }
                tokio::select! {
                    _ = tokio::task::yield_now() => {},
                    result = &mut run => panic!("SFX player stopped unexpectedly: {result:?}"),
                }
            }
            cancellation.cancel();
            run.await.unwrap();
        }
        internal.cancellation_token = CancellationToken::new();
    }

    #[tokio::test(start_paused = true)]
    async fn the_output_closes_once_the_sound_has_finished() {
        let (commands_sender, commands) = mpsc::unbounded_channel();
        let mut internal = SfxPlayerInternal::new(commands, CancellationToken::new());
        // A sink without a device, whose queue the test plays by hand.
        let (sink, mut queue) = Sink::new_idle();
        sink.append(SineWave::new(440.0).take_duration(Duration::from_millis(10)));
        internal.sink = Some(sink);
        internal.state = InternalSfxPlaybackState::Playing;

        // A sound that still plays keeps its output.
        run_briefly(&mut internal, &commands_sender).await;
        assert!(internal.sink.is_some());

        for _ in 0..48_000 {
            queue.next();
        }
        assert!(internal.sink.as_ref().is_some_and(Sink::empty));
        run_briefly(&mut internal, &commands_sender).await;
        assert!(internal.sink.is_none());
        assert_eq!(internal.state, InternalSfxPlaybackState::Empty);
    }
}
