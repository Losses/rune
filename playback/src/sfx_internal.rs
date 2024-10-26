use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{Context, Result};
use log::{debug, error, info};
use rodio::{Decoder, OutputStream, Sink};
use tokio::sync::mpsc;
use tokio::time::{sleep_until, Instant};
use tokio_util::sync::CancellationToken;

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
    debounce_timer: Option<Instant>,
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
            debounce_timer: None,
            cancellation_token,
            volume: 1.0,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            if let Some(sink) = &self.sink {
                if sink.empty() && self.state == InternalSfxPlaybackState::Playing {
                    self.state = InternalSfxPlaybackState::Empty;
                }
            }

            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    if self.cancellation_token.is_cancelled() {
                        debug!("Cancellation token triggered, exiting run loop");
                        if let Some(sink) = &self.sink {
                            sink.stop();
                        }
                        break;
                    }

                    debug!("Received command: {:?}", cmd);
                    match cmd {
                        SfxPlayerCommand::Load { path } => self.load(Some(path)),
                        SfxPlayerCommand::SetVolume(volume) => self.set_volume(volume),
                    }?;
                },
                _ = async {
                    if let Some(timer) = self.debounce_timer {
                        sleep_until(timer).await;
                        true
                    } else {
                        false
                    }
                }, if self.debounce_timer.is_some() => {
                    self.debounce_timer = None;
                },
                _ = self.cancellation_token.cancelled() => {
                    debug!("Cancellation token triggered, exiting run loop");
                    if let Some(sink) = &self.sink {
                        sink.stop();
                    }
                    break;
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
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
            let source =
                Decoder::new(BufReader::new(file)).with_context(|| "Failed to decode audio")?;
            let (stream, stream_handle) =
                OutputStream::try_default().context("Failed to create output stream")?;
            let sink = Sink::try_new(&stream_handle).context("Failed to create sink")?;

            sink.set_volume(self.volume);
            sink.append(source);
            sink.play();

            self.sink = Some(sink);
            self._stream = Some(stream);
            self.current_track_path = Some(path.clone());
            self.state = InternalSfxPlaybackState::Playing;
            info!("SFX track loaded and playing: {:?}", path);
        } else {
            error!("Load command received without index");
        }
        Ok(())
    }

    fn set_volume(&mut self, volume: f32) -> Result<()> {
        self.volume = volume;
        if let Some(sink) = &self.sink {
            sink.set_volume(volume);
        }

        Ok(())
    }
}
