use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AddMode {
    PlayNext,
    AppendToEnd,
}

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

/// Generates a random sequence from 1 to max_value and returns the nth value
///
/// # Parameters
///
/// * `seed` - The seed for the random number generator
/// * `max_value` - The maximum value of the sequence
/// * `n` - The nth value to return (1-based index)
///
/// # Returns
///
/// Returns the nth value, or None if n is out of range
pub fn get_random_sequence(seed: u64, max_value: usize) -> Vec<usize> {
    // Create a sequence from 1 to max_value
    let mut values: Vec<usize> = (0..=(max_value - 1)).collect();

    // Create a random number generator with the given seed
    let mut rng = StdRng::seed_from_u64(seed);

    // Shuffle the sequence
    values.shuffle(&mut rng);

    values
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
            let shuffle_seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            self.random_map = get_random_sequence(shuffle_seed, playlist_len);
        } else {
            self.random_map.clear();
        }
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
                        for i in 0..self.random_map.len() {
                            if self.random_map[i] >= insert_index {
                                self.random_map[i] += 1;
                            }
                        }
                        self.random_map.insert(insert_index, insert_index);
                    }
                }
                AddMode::AppendToEnd => {
                    self.update_random_map(playlist_len);
                }
            },
            _ => {
                self.update_random_map(playlist_len);
            }
        }
    }
}
