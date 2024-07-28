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
    Seek(f64),
    AddToPlaylist { id: i32, path: PathBuf },
    RemoveFromPlaylist { index: usize },
    ClearPlaylist,
}

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    Stopped,
    Playing {
        id: i32,
        index: usize,
        path: PathBuf,
        position: Duration,
    },
    Paused {
        id: i32,
        index: usize,
        path: PathBuf,
        position: Duration,
    },
    EndOfPlaylist,
    EndOfTrack {
        id: i32,
        index: usize,
        path: PathBuf,
    },
    Error {
        id: i32,
        index: usize,
        path: PathBuf,
        error: String,
    },
    Progress {
        id: i32,
        index: usize,
        path: PathBuf,
        position: Duration,
    },
}

pub struct PlaylistItem {
    pub id: i32,
    pub path: PathBuf,
}

#[derive(Debug, PartialEq)]
enum InternalPlaybackState {
    Playing,
    Paused,
    Stopped,
}

pub(crate) struct PlayerInternal {
    commands: mpsc::UnboundedReceiver<PlayerCommand>,
    event_sender: mpsc::UnboundedSender<PlayerEvent>,
    playlist: Vec<PlaylistItem>,
    current_track_id: Option<i32>,
    current_track_index: Option<usize>,
    current_track_path: Option<PathBuf>,
    sink: Option<Sink>,
    _stream: Option<OutputStream>,
    state: InternalPlaybackState,
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
            current_track_id: None,
            current_track_index: None,
            current_track_path: None,
            sink: None,
            _stream: None,
            state: InternalPlaybackState::Stopped,
        }
    }

    pub async fn run(&mut self) {
        let mut progress_interval = interval(Duration::from_millis(100));

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
                        PlayerCommand::Seek(position) => self.seek(position),
                        PlayerCommand::AddToPlaylist { id, path } => self.add_to_playlist(id, path),
                        PlayerCommand::RemoveFromPlaylist { index } => self.remove_from_playlist(index),
                        PlayerCommand::ClearPlaylist => self.clear_playlist(),
                    }
                },
                _ = progress_interval.tick() => {
                    if self.state != InternalPlaybackState::Stopped {
                        self.send_progress();
                    }
                }
            }
        }
    }

    fn load(&mut self, index: Option<usize>) {
        if let Some(index) = index {
            debug!("Loading track at index: {}", index);
            let item = &self.playlist[index];
            let file = File::open(item.path.clone());
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
                            self.current_track_id = Some(item.id);
                            self.current_track_path = Some(item.path.clone());
                            info!("Track loaded: {:?}", item.path);
                            self.event_sender
                                .send(PlayerEvent::Playing {
                                    id: self.current_track_id.unwrap(),
                                    index: self.current_track_index.unwrap(),
                                    path: self.current_track_path.clone().unwrap(),
                                    position: Duration::new(0, 0),
                                })
                                .unwrap();
                            self.state = InternalPlaybackState::Playing;
                        }
                        Err(e) => {
                            error!("Failed to decode audio: {:?}", e);
                            self.event_sender
                                .send(PlayerEvent::Error {
                                    id: self.current_track_id.unwrap(),
                                    index,
                                    path: item.path.clone(),
                                    error: "Failed to decode audio".to_string(),
                                })
                                .unwrap();
                            self.state = InternalPlaybackState::Stopped;
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to open file: {:?}", e);
                    self.event_sender
                        .send(PlayerEvent::Error {
                            id: self.current_track_id.unwrap(),
                            index,
                            path: item.path.clone(),
                            error: "Failed to open file".to_string(),
                        })
                        .unwrap();
                    self.state = InternalPlaybackState::Stopped;
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
            self.event_sender
                .send(PlayerEvent::Playing {
                    id: self.current_track_id.unwrap(),
                    index: self.current_track_index.unwrap(),
                    path: self.current_track_path.clone().unwrap(),
                    position: Duration::new(0, 0),
                })
                .unwrap();
            self.state = InternalPlaybackState::Playing;
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
            self.event_sender
                .send(PlayerEvent::Paused {
                    id: self.current_track_id.unwrap(),
                    index: self.current_track_index.unwrap(),
                    path: self.current_track_path.clone().unwrap(),
                    position: sink.get_pos(),
                })
                .unwrap();
            self.state = InternalPlaybackState::Paused;
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            info!("Playback stopped");
            self.event_sender.send(PlayerEvent::Stopped).unwrap();
            self.state = InternalPlaybackState::Stopped;
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
                self.state = InternalPlaybackState::Stopped;
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

    fn seek(&mut self, position: f64) {
        if let Some(sink) = &self.sink {
            match sink.try_seek(std::time::Duration::from_secs(position as u64)) {
                Ok(_) => {
                    info!("Seeking to position: {} s", position);
                    match self.event_sender.send(PlayerEvent::Playing {
                        id: self.current_track_id.unwrap(),
                        index: self.current_track_index.unwrap(),
                        path: self.current_track_path.clone().unwrap(),
                        position: sink.get_pos(),
                    }) {
                        Ok(_) => (),
                        Err(e) => error!("Failed to send Playing event: {:?}", e),
                    }
                    self.state = InternalPlaybackState::Playing;
                }
                Err(e) => error!("Failed to seek: {:?}", e),
            }
        } else {
            error!("Seek command received but no track is loaded");
        }
    }

    fn add_to_playlist(&mut self, id: i32, path: PathBuf) {
        debug!("Adding to playlist: {:?}", path);
        self.playlist.push(PlaylistItem { id, path });
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

    fn clear_playlist(&mut self) {
        self.playlist.clear();
        self.current_track_index = None;
        self.sink = None;
        self._stream = None;
        info!("Playlist cleared");
        self.event_sender.send(PlayerEvent::Stopped).unwrap();
        self.state = InternalPlaybackState::Stopped;
    }

    fn send_progress(&mut self) {
        if let Some(sink) = &self.sink {
            if sink.empty() {
                self.event_sender
                    .send(PlayerEvent::EndOfTrack {
                        id: self.current_track_id.unwrap(),
                        index: self.current_track_index.unwrap(),
                        path: self.current_track_path.clone().unwrap(),
                    })
                    .unwrap();

                if self.state != InternalPlaybackState::Stopped {
                    self.next();
                }
            } else {
                self.event_sender
                    .send(PlayerEvent::Progress {
                        id: self.current_track_id.unwrap(),
                        index: self.current_track_index.unwrap(),
                        path: self.current_track_path.clone().unwrap(),
                        position: sink.get_pos(),
                    })
                    .unwrap();
            }
        }
    }
}
