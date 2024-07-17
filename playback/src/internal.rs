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
        }
    }

    pub async fn run(&mut self) {
        let mut progress_interval = interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
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
        if index.is_none() {
            return;
        }

        if let Ok((_, stream_handle)) = OutputStream::try_default() {
            let _index = index.unwrap();
            let path = &self.playlist[_index];
            let file = File::open(path);
            match file {
                Ok(file) => {
                    let source = Decoder::new(BufReader::new(file));
                    match source {
                        Ok(source) => {
                            let sink = Sink::try_new(&stream_handle).unwrap();
                            sink.append(source);
                            self.sink = Some(sink);
                            self.event_sender.send(PlayerEvent::Playing).unwrap();
                        }
                        Err(_) => {
                            self.event_sender
                                .send(PlayerEvent::Error {
                                    index: _index.clone(),
                                    path: path.clone(),
                                    error: "Failed to decode audio".to_string(),
                                })
                                .unwrap();
                        }
                    }
                }
                Err(_) => {
                    self.event_sender
                        .send(PlayerEvent::Error {
                            index: _index.clone(),
                            path: path.clone(),
                            error: "Failed to open file".to_string(),
                        })
                        .unwrap();
                }
            }
        }
    }

    fn play(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
            self.event_sender.send(PlayerEvent::Playing).unwrap();
        }
    }

    fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
            self.event_sender.send(PlayerEvent::Paused).unwrap();
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = self.sink.take() {
            sink.stop();
            self.event_sender.send(PlayerEvent::Stopped).unwrap();
        }
    }

    fn next(&mut self) {
        if let Some(index) = self.current_track_index {
            if index + 1 < self.playlist.len() {
                self.current_track_index = Some(index + 1);
                self.load(Some(index + 1));
            } else {
                self.event_sender.send(PlayerEvent::EndOfPlaylist).unwrap();
            }
        }
    }

    fn previous(&mut self) {
        if let Some(index) = self.current_track_index {
            if index > 0 {
                self.current_track_index = Some(index - 1);
                self.load(self.current_track_index.clone());
            }
        }
    }

    fn seek(&mut self, position_ms: u32) {
        if let Some(sink) = &self.sink {
            sink.try_seek(std::time::Duration::from_millis(position_ms as u64))
                .unwrap();
            self.event_sender.send(PlayerEvent::Playing).unwrap();
        }
    }

    fn add_to_playlist(&mut self, path: PathBuf) {
        self.playlist.push(path);
    }

    fn remove_from_playlist(&mut self, index: usize) {
        self.playlist.remove(index);
    }

    fn send_progress(&self) {
        if let Some(sink) = &self.sink {
            let position = sink.get_pos();
            self.event_sender
                .send(PlayerEvent::Progress { position })
                .unwrap();
        }
    }
}
