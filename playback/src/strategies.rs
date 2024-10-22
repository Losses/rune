use rand::seq::SliceRandom;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddMode {
    PlayNext,
    AppendToEnd,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UpdateReason {
    AddToPlaylist { mode: AddMode, index: Option<usize> },
    RemoveFromPlaylist { index: usize },
    ClearPlaylist,
    MovePlaylistItem { old_index: usize, new_index: usize },
}

pub trait PlaybackStrategy {
    fn next(&self, current_index: usize, playlist_len: usize) -> Option<usize>;
    fn previous(&self, current_index: usize, playlist_len: usize) -> Option<usize>;
    fn on_playlist_end(&self, playlist_len: usize) -> Option<usize>;
    fn get_mapped_track_index(&self, index: usize, playlist_len: usize) -> usize;
    fn on_playlist_updated(&mut self, playlist_len: usize, reason: UpdateReason);
}

pub struct SequentialStrategy;
pub struct RepeatOneStrategy;
pub struct RepeatAllStrategy;
pub struct ShuffleStrategy {
    random_map: Vec<usize>,
}

/// Generates a random sequence from 0 to max_value, keeping 0 at the first position
///
/// # Parameters
///
/// * `seed` - The seed for the random number generator
/// * `max_value` - The maximum value of the sequence (exclusive)
///
/// # Returns
///
/// Returns a Vec<usize> with a randomized sequence, 0 always at the first position
pub fn get_random_sequence(max_value: usize) -> Vec<usize> {
    if max_value == 0 {
        return vec![];
    }

    let mut values: Vec<usize> = (1..(max_value + 1)).collect();
    let mut rng = rand::thread_rng();
    values.shuffle(&mut rng);

    let mut result: Vec<usize> = vec![0];
    result.extend(values);
    result
}

impl PlaybackStrategy for SequentialStrategy {
    fn next(&self, current_index: usize, playlist_len: usize) -> Option<usize> {
        if current_index + 1 < playlist_len {
            Some(current_index + 1)
        } else {
            None
        }
    }

    fn previous(&self, current_index: usize, _playlist_len: usize) -> Option<usize> {
        if current_index > 0 {
            Some(current_index - 1)
        } else {
            None
        }
    }

    fn on_playlist_end(&self, _playlist_len: usize) -> Option<usize> {
        None
    }

    fn get_mapped_track_index(&self, index: usize, _playlist_len: usize) -> usize {
        index
    }

    fn on_playlist_updated(&mut self, _playlist_len: usize, _reason: UpdateReason) {}
}

impl PlaybackStrategy for RepeatOneStrategy {
    fn next(&self, current_index: usize, _playlist_len: usize) -> Option<usize> {
        Some(current_index)
    }

    fn previous(&self, current_index: usize, _playlist_len: usize) -> Option<usize> {
        Some(current_index)
    }

    fn on_playlist_end(&self, _playlist_len: usize) -> Option<usize> {
        None
    }

    fn get_mapped_track_index(&self, index: usize, _playlist_len: usize) -> usize {
        index
    }

    fn on_playlist_updated(&mut self, _playlist_len: usize, _reason: UpdateReason) {}
}

impl PlaybackStrategy for RepeatAllStrategy {
    fn next(&self, current_index: usize, playlist_len: usize) -> Option<usize> {
        Some((current_index + 1) % playlist_len)
    }

    fn previous(&self, current_index: usize, playlist_len: usize) -> Option<usize> {
        Some((current_index + playlist_len - 1) % playlist_len)
    }

    fn on_playlist_end(&self, _playlist_len: usize) -> Option<usize> {
        Some(0)
    }

    fn get_mapped_track_index(&self, index: usize, _playlist_len: usize) -> usize {
        index
    }

    fn on_playlist_updated(&mut self, _playlist_len: usize, _reason: UpdateReason) {}
}

impl ShuffleStrategy {
    pub fn new(playlist_len: usize) -> Self {
        let mut strategy = ShuffleStrategy {
            random_map: Vec::new(),
        };
        strategy.update_random_map(playlist_len);
        strategy
    }

    fn update_random_map(&mut self, playlist_len: usize) {
        if playlist_len > 0 {
            self.random_map = get_random_sequence(playlist_len - 1);
        } else {
            self.random_map.clear();
        }
    }

    fn insert_randomized(&mut self, start: usize, count: usize) {
        let new_tracks: Vec<usize> = (start..start + count).collect();
        let mut rng = rand::thread_rng();
        let mut shuffled = new_tracks[1..].to_vec();
        shuffled.shuffle(&mut rng);

        let mut to_insert = vec![new_tracks[0]];
        to_insert.extend(shuffled);

        self.random_map.splice(start..start, to_insert);
    }
}

impl PlaybackStrategy for ShuffleStrategy {
    fn next(&self, current_index: usize, playlist_len: usize) -> Option<usize> {
        if current_index + 1 < playlist_len {
            Some(current_index + 1)
        } else {
            None
        }
    }

    fn previous(&self, current_index: usize, _playlist_len: usize) -> Option<usize> {
        if current_index > 0 {
            Some(current_index - 1)
        } else {
            None
        }
    }

    fn on_playlist_end(&self, _playlist_len: usize) -> Option<usize> {
        Some(0)
    }

    fn get_mapped_track_index(&self, index: usize, _playlist_len: usize) -> usize {
        self.random_map[index]
    }

    fn on_playlist_updated(&mut self, playlist_len: usize, reason: UpdateReason) {
        match reason {
            UpdateReason::AddToPlaylist { mode, index } => match mode {
                AddMode::PlayNext => {
                    if let Some(insert_index) = index {
                        // Shift existing indices
                        for i in 0..self.random_map.len() {
                            if self.random_map[i] >= insert_index {
                                self.random_map[i] += 1;
                            }
                        }
                        // Insert new randomized tracks
                        self.insert_randomized(insert_index, 1);
                    }
                }
                AddMode::AppendToEnd => {
                    let new_tracks_count = playlist_len - self.random_map.len();
                    self.insert_randomized(self.random_map.len(), new_tracks_count);
                }
            },
            _ => {
                if playlist_len > 0 {
                    self.random_map = get_random_sequence(playlist_len - 1);
                } else {
                    self.random_map.clear();
                }
            }
        }
    }
}
