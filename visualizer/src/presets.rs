//! The `.milk` / `.milk2` presets of a directory , picked the way
//! MilkDrop 2 does it: weighted by rating, with a history, and on a timer.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::Rng;

/// Where a preset's bytes are.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Location {
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Preset {
    pub location: Location,
    /// The file name without its extension.
    pub name: String,
}

impl Preset {
    fn new(location: Location) -> Self {
        let Location::File(path) = &location;
        let full = path.to_string_lossy().into_owned();
        let file = full.rsplit(['/', '\\']).next().unwrap_or(&full);
        let name = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        Self {
            name: if name.is_empty() { "preset" } else { name }.to_string(),
            location,
        }
    }

    pub fn from_path(path: &Path) -> Self {
        Self::new(Location::File(path.to_path_buf()))
    }
}

fn is_preset_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".milk") || lower.ends_with(".milk2")
}

/// Rating of presets that have none, and the range ratings are clamped to (MilkDrop 2).
const DEFAULT_RATING: f32 = 3.0;
const MAX_RATING: f32 = 5.0;

/// How many loaded presets "previous" can walk back through (MilkDrop 2: `PRESET_HIST_LEN`).
const HISTORY_LEN: usize = 64;

/// How many bytes of a preset file are searched for its rating; it sits near the top.
const RATING_SCAN_BYTES: u64 = 16 * 1024;

/// The `fRating=` value of a preset file, `3.0` if there is none or it is not a number, clamped
/// to `0..=5`.
fn parse_rating(text: &[u8]) -> f32 {
    text.split(|&b| b == b'\n')
        .find_map(|line| line.strip_prefix(b"fRating="))
        .and_then(|value| std::str::from_utf8(value).ok()?.trim().parse::<f32>().ok())
        .filter(|rating| rating.is_finite())
        .map_or(DEFAULT_RATING, |rating| rating.clamp(0.0, MAX_RATING))
}

fn read_rating(path: &Path) -> f32 {
    let mut head = Vec::new();
    let read = std::fs::File::open(path)
        .and_then(|file| file.take(RATING_SCAN_BYTES).read_to_end(&mut head));
    if read.is_err() {
        return DEFAULT_RATING;
    }
    parse_rating(&head)
}

struct Entry {
    preset: Preset,
    rating: f32,
}

/// The presets that take part in rotation, with the history of what was loaded. The rules are
/// those of MilkDrop 2 (Nullsoft, BSD-3): random picks are weighted by rating, "previous" walks
/// back through the last [`HISTORY_LEN`] presets and "next" walks forward again before it picks
/// a new one.
#[derive(Default)]
pub struct PresetList {
    entries: Vec<Entry>,
    /// The presets loaded through this list, oldest first.
    history: VecDeque<Preset>,
    /// Index into `history` of the preset that is loaded. Meaningless while it is empty.
    cursor: usize,
}

impl PresetList {
    /// Scan `dir` and its subfolders and replace the list. Hidden folders and symlinked folders
    /// (possible cycles) are skipped. Returns the number of presets found.
    pub fn scan(&mut self, dir: &Path) -> Result<usize> {
        let mut entries = Vec::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(current) = pending.pop() {
            let read = std::fs::read_dir(&current)
                .with_context(|| format!("Failed to read {}", current.display()))?;
            for entry in read {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    if !entry.file_name().to_string_lossy().starts_with('.') {
                        pending.push(path);
                    }
                    continue;
                }
                if path.to_str().is_some_and(is_preset_name) {
                    let rating = read_rating(&path);
                    entries.push(Entry {
                        preset: Preset::new(Location::File(path)),
                        rating,
                    });
                }
            }
        }
        Ok(self.replace(entries))
    }

    /// Empty the list and its history.
    pub fn clear(&mut self) {
        self.replace(Vec::new());
    }

    fn replace(&mut self, mut entries: Vec<Entry>) -> usize {
        entries.sort_by(|a, b| a.preset.cmp(&b.preset));
        self.entries = entries;
        self.history.clear();
        self.cursor = 0;
        self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// MilkDrop 2's `NextPreset`: forward in the history if the user went back, else a new
    /// rating-weighted random preset.
    pub fn next(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        if !self.history.is_empty() && self.cursor + 1 < self.history.len() {
            self.cursor += 1;
            return Ok(self.history[self.cursor].clone());
        }
        self.random(rng)
    }

    /// One step back in the history. `None` (nothing to do) at its start, like MilkDrop 2.
    pub fn previous(&mut self) -> Result<Option<Preset>> {
        self.check_not_empty()?;
        if self.cursor == 0 || self.history.is_empty() {
            return Ok(None);
        }
        self.cursor -= 1;
        Ok(Some(self.history[self.cursor].clone()))
    }

    /// A new rating-weighted random preset. Forward history is dropped.
    pub fn random(&mut self, rng: &mut impl Rng) -> Result<Preset> {
        self.check_not_empty()?;
        let preset = self.entries[self.pick(rng)].preset.clone();
        if !self.history.is_empty() {
            self.history.truncate(self.cursor + 1);
        }
        self.history.push_back(preset.clone());
        if self.history.len() > HISTORY_LEN {
            self.history.pop_front();
        }
        self.cursor = self.history.len() - 1;
        Ok(preset)
    }

    fn check_not_empty(&self) -> Result<()> {
        if self.entries.is_empty() {
            bail!("No presets in the preset directory");
        }
        Ok(())
    }

    /// Index of a random entry with a probability proportional to its rating, so a rating of 0
    /// is never picked; uniform if the ratings add up to (almost) nothing.
    fn pick(&self, rng: &mut impl Rng) -> usize {
        let total: f32 = self.entries.iter().map(|e| e.rating).sum();
        if total < 0.1 {
            return rng.gen_range(0..self.entries.len());
        }
        let target = rng.gen_range(0.0..total);
        let mut cumulative = 0.0;
        for (i, entry) in self.entries.iter().enumerate() {
            cumulative += entry.rating;
            if target < cumulative {
                return i;
            }
        }
        // Rounding of the sum: the last preset that can be picked.
        self.entries
            .iter()
            .rposition(|e| e.rating > 0.0)
            .unwrap_or(0)
    }
}

/// When the next automatic switch is due, on the render loop's clock (seconds). MilkDrop 2:
/// a preset is shown for the blend time, 16 s and up to 10 s more at random.
#[derive(Default)]
pub struct AutoSwitch {
    due: Option<f64>,
}

impl AutoSwitch {
    /// Blend time of an automatic switch.
    pub const BLEND_SECONDS: f64 = 2.7;
    const BETWEEN_SECONDS: f64 = 16.0;
    const RANDOM_SECONDS: f64 = 10.0;

    /// A preset was loaded: count again from the next frame.
    pub fn restart(&mut self) {
        self.due = None;
    }

    /// Call once per frame. Returns whether a switch is due now; the caller then loads a preset
    /// and calls [`Self::restart`]. While `paused` the timer keeps starting over.
    pub fn tick(&mut self, now: f64, paused: bool, rng: &mut impl Rng) -> bool {
        if paused {
            self.due = None;
            return false;
        }
        let due = *self.due.get_or_insert_with(|| {
            now + Self::BLEND_SECONDS
                + Self::BETWEEN_SECONDS
                + rng.gen_range(0.0..1.0) * Self::RANDOM_SECONDS
        });
        now > due
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::*;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(7)
    }

    fn rated(ratings: &[(&str, &str)]) -> PresetList {
        let entries = ratings
            .iter()
            .map(|(name, rating)| Entry {
                preset: Preset::new(Location::File(format!("{name}.milk").into())),
                rating: parse_rating(format!("[preset00]\nfRating={rating}\n").as_bytes()),
            })
            .collect();
        let mut list = PresetList::default();
        list.replace(entries);
        list
    }

    #[test]
    fn scan_is_recursive_but_skips_hidden_and_symlinked_folders() {
        let dir = std::env::temp_dir().join(format!("rune-presets-{}", std::process::id()));
        for sub in ["Fractal", ".git"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
        }
        for name in [
            "b.milk",
            "a.MILK",
            "c.milk2",
            "ignored.txt",
            "Fractal/deep.milk",
            ".git/hidden.milk",
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        let mut list = PresetList::default();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
            std::os::unix::fs::symlink(dir.join("Fractal"), dir.join("link")).unwrap();
        }
        assert_eq!(list.scan(&dir).unwrap(), 4);
        let mut rng = rng();
        let mut names: Vec<String> = (0..50)
            .map(|_| list.random(&mut rng).unwrap().name)
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names, ["a", "b", "c", "deep"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_scan_reads_ratings() {
        let dir = std::env::temp_dir().join(format!("rune-presets-rated-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zero.milk"), "[preset00]\nfRating=0.0\n").unwrap();
        std::fs::write(dir.join("five.milk"), "[preset00]\r\nfRating=5.0\r\n").unwrap();
        let mut list = PresetList::default();
        assert_eq!(list.scan(&dir).unwrap(), 2);
        let mut rng = rng();
        for _ in 0..30 {
            assert_eq!(list.random(&mut rng).unwrap().name, "five");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_ratings() {
        assert_eq!(parse_rating(b"[preset00]\nfRating=2.5\nzoom=1"), 2.5);
        assert_eq!(parse_rating(b"[preset00]\r\nfRating=1\r\n"), 1.0);
        assert_eq!(parse_rating(b"[preset00]\nzoom=1\n"), 3.0);
        assert_eq!(parse_rating(b""), 3.0);
        assert_eq!(parse_rating(b"fRating=abc\n"), 3.0);
        assert_eq!(parse_rating(b"fRating=nan\n"), 3.0);
        assert_eq!(parse_rating(b"fRating=\xff\xfe\n"), 3.0);
        assert_eq!(parse_rating(b"fRating=9\n"), 5.0);
        assert_eq!(parse_rating(b"fRating=-2\n"), 0.0);
    }

    #[test]
    fn weighted_pick_never_returns_a_zero_rating() {
        let mut list = rated(&[("zero", "0"), ("one", "1"), ("five", "5")]);
        let mut rng = rng();
        let (mut one, mut five) = (0, 0);
        for _ in 0..600 {
            match list.random(&mut rng).unwrap().name.as_str() {
                "one" => one += 1,
                "five" => five += 1,
                other => panic!("picked {other}"),
            }
        }
        // 1:5 odds, so "five" comes up far more often.
        assert!(five > 2 * one, "{one} {five}");
    }

    #[test]
    fn all_zero_ratings_pick_uniformly() {
        let mut list = rated(&[("a", "0"), ("b", "0"), ("c", "0")]);
        let mut rng = rng();
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..100 {
            seen.insert(list.random(&mut rng).unwrap().name);
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn history_walks_back_and_forward() {
        let mut list = rated(&[("a", "3"), ("b", "3"), ("c", "3"), ("d", "3")]);
        let mut rng = rng();
        assert!(list.previous().unwrap().is_none());
        let first = list.random(&mut rng).unwrap();
        assert!(list.previous().unwrap().is_none());
        let second = list.random(&mut rng).unwrap();
        assert_eq!(list.previous().unwrap(), Some(first));
        assert_eq!(list.next(&mut rng).unwrap(), second);
    }

    #[test]
    fn random_after_previous_drops_forward_history() {
        let mut list = rated(&[("a", "3"), ("b", "3"), ("c", "3")]);
        let mut rng = rng();
        list.random(&mut rng).unwrap();
        list.random(&mut rng).unwrap();
        list.random(&mut rng).unwrap();
        list.previous().unwrap();
        list.previous().unwrap();
        list.random(&mut rng).unwrap();
        assert_eq!(list.history.len(), 2);
        // Nothing to go forward to: next picks a new one and extends the history.
        list.next(&mut rng).unwrap();
        assert_eq!(list.history.len(), 3);
        assert_eq!(list.cursor, 2);
    }

    #[test]
    fn history_keeps_the_last_64() {
        let mut list = rated(&[("a", "3"), ("b", "3")]);
        let mut rng = rng();
        for _ in 0..100 {
            list.random(&mut rng).unwrap();
        }
        assert_eq!(list.history.len(), HISTORY_LEN);
        for _ in 0..HISTORY_LEN - 1 {
            assert!(list.previous().unwrap().is_some());
        }
        assert!(list.previous().unwrap().is_none());
    }

    #[test]
    fn empty_list_is_an_error() {
        let mut list = PresetList::default();
        list.clear();
        assert!(list.next(&mut rng()).is_err());
        assert!(list.previous().is_err());
        assert!(list.random(&mut rng()).is_err());
    }

    #[test]
    fn auto_switch_schedule() {
        let mut rng = rng();
        let mut auto = AutoSwitch::default();
        // The first tick fixes the due time: 2.7 + 16 + 0..10 s after it.
        assert!(!auto.tick(100.0, false, &mut rng));
        let due = auto.due.unwrap();
        assert!((118.7..128.7).contains(&due), "{due}");
        assert!(!auto.tick(due - 0.01, false, &mut rng));
        assert!(auto.tick(due + 0.01, false, &mut rng));
        // A load starts the count again.
        auto.restart();
        assert!(!auto.tick(due + 0.02, false, &mut rng));
        assert!(auto.due.unwrap() >= due + 0.02 + 18.7);
    }

    #[test]
    fn auto_switch_pause_keeps_pushing_the_start() {
        let mut rng = rng();
        let mut auto = AutoSwitch::default();
        assert!(!auto.tick(0.0, false, &mut rng));
        for t in [10.0, 100.0, 1000.0] {
            assert!(!auto.tick(t, true, &mut rng));
        }
        // Unpausing counts from zero, not from the old due time.
        assert!(!auto.tick(1000.5, false, &mut rng));
        let due = auto.due.unwrap();
        assert!((1019.2..1029.2).contains(&due), "{due}");
    }
}
