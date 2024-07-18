use log::{debug, error, info};
use rodio::{Decoder, OutputStream, Sink};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};

#[derive(Debug)]
pub enum PlayerCommand {
    Load { index: usize },
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Seek(u32),
    AddToPlaylist { path: PathBuf },
    RemoveFromPlaylist { index: usize },
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Stopped,
    Playing,
    Paused,
    EndOfPlaylist,
    Error {
        index: usize,
        path: PathBuf,
        error: String,
    },
    Progress {
        position: Duration,
    },
}

pub(crate) struct PlayerInternal {
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    event_sender: mpsc::UnboundedSender<PlayerEvent>,
    playlist: Vec<PathBuf>,
    current_track_index: Option<usize>,
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
}

impl PlayerInternal {
    pub fn new(
        commands: mpsc::UnboundedReceiver<PlayerCommand>,
        event_sender: mpsc::UnboundedSender<PlayerEvent>,
    ) -> Self {

        Self {
            commands,
            event_sender,
            playlist: Vec::new(),
            current_track_index: None,
            sink: None,
            _stream: None,
        }
    }

    pub async fn run(&mut self) {
        let mut progress_interval = interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    debug!("Received command: {:?}", cmd);
                    match cmd {
                        PlayerCommand::Load { index } => self.load(Some(index)),
                        PlayerCommand::Play => self.play(),
                        PlayerCommand::Pause => self.pause(),
                        PlayerCommand::Stop => self.stop(),
                        PlayerCommand::Next => self.next(),
                        PlayerCommand::Previous => self.previous(),
                        PlayerCommand::Seek(position_ms) => self.seek(position_ms),
                        PlayerCommand::AddToPlaylist { path } => self.add_to_playlist(path),
                        PlayerCommand::RemoveFromPlaylist { index } => self.remove_from_playlist(index),
                    }
                },
                _ = progress_interval.tick() => {
                    self.send_progress();
                }
            }
        }
    }

    fn load(&mut self, index: Option<usize>) {
        if let Some(index) = index {
            debug!("Loading track at index: {}", index);
            let path = &self.playlist[index];
            let file = File::open(path);
            match file {
                Ok(file) => {
                    let source = Decoder::new(BufReader::new(file));
                    match source {
                        Ok(source) => {
                            let (stream, stream_handle) = OutputStream::try_default().unwrap();
                            let sink = Sink::try_new(&stream_handle).unwrap();
                            sink.append(source);
                            self.sink = Some(sink);
                            self._stream = Some(stream);
                            self.current_track_index = Some(index);
                            info!("Track loaded: {:?}", path);
                            self.event_sender.send(PlayerEvent::Playing).unwrap();
                        }
                        Err(e) => {
                            error!("Failed to decode audio: {:?}", e);
                            self.event_sender
                                .send(PlayerEvent::Error {
                                    index,
                                    path: path.clone(),
                                    error: "Failed to decode audio".to_string(),
                                })
                                .unwrap();
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to open file: {:?}", e);
                    self.event_sender
                        .send(PlayerEvent::Error {
                            index,
                            path: path.clone(),
                            error: "Failed to open file".to_string(),
                        })
                        .unwrap();
                }
            }
        } else {
            error!("Load command received without index");
        }
    }

    fn play(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
            info!("Playback started");
            self.event_sender.send(PlayerEvent::Playing).unwrap();
        } else {
            info!("Loading the first track");
            self.load(Some(0));
            self.play();
        }
    }

    fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
            info!("Playback paused");
            self.event_sender.send(PlayerEvent::Paused).unwrap();
        } else {
            error!("Pause command received but no track is loaded");
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            info!("Playback stopped");
            self.event_sender.send(PlayerEvent::Stopped).unwrap();
        } else {
            error!("Stop command received but no track is loaded");
        }
    }

    fn next(&mut self) {
        if let Some(index) = self.current_track_index {
            if index + 1 < self.playlist.len() {
                self.current_track_index = Some(index + 1);
                debug!("Moving to next track: {}", index + 1);
                self.load(Some(index + 1));
            } else {
                info!("End of playlist reached");
                self.event_sender.send(PlayerEvent::EndOfPlaylist).unwrap();
            }
        } else {
            error!("Next command received but no track is currently playing");
        }
    }

    fn previous(&mut self) {
        if let Some(index) = self.current_track_index {
            if index > 0 {
                self.current_track_index = Some(index - 1);
                debug!("Moving to previous track: {}", index - 1);
                self.load(Some(index - 1));
            } else {
                error!("Previous command received but already at the first track");
            }
        } else {
            error!("Previous command received but no track is currently playing");
        }
    }

    fn seek(&mut self, position_ms: u32) {
        if let Some(sink) = &self.sink {
            sink.try_seek(std::time::Duration::from_millis(position_ms as u64))
                .unwrap();
            info!("Seeking to position: {} ms", position_ms);
            self.event_sender.send(PlayerEvent::Playing).unwrap();
        } else {
            error!("Seek command received but no track is loaded");
        }
    }

    fn add_to_playlist(&mut self, path: PathBuf) {
        debug!("Adding to playlist: {:?}", path);
        self.playlist.push(path);
    }

    fn remove_from_playlist(&mut self, index: usize) {
        if index < self.playlist.len() {
            debug!("Removing from playlist at index: {}", index);
            self.playlist.remove(index);
        } else {
            error!(
                "Remove command received but index {} is out of bounds",
                index
            );
        }
    }

    fn send_progress(&self) {
        if let Some(sink) = &self.sink {
            let position = sink.get_pos();
            debug!("Sending progress: {:?}", position);
            self.event_sender
                .send(PlayerEvent::Progress { position })
                .unwrap();
        } else {
            error!("Progress update attempted but no track is loaded");
        }
    }
}
