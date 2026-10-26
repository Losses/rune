//! A port of a Python prototype of the music analyzer that analyses a song into a
//! `Chart`. Parity with the Python is the goal, so that the
//! modules follow its functions one for one: `front` is `front()`, `back` is `back()` with the
//! tempo and beat pieces in `tempo`, `decode` is `load()`.
//!
//! This is the rhythm half: the tempo, the beats, the meter and the downbeats. The sections,
//! the event streams' peaks, the lead and the curves are left empty in the `Chart` for now.

pub mod back;
pub mod chart;
pub mod decode;
pub mod front;
pub mod ndimage;
pub mod np;
pub mod tempo;

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use fsio::FsIo;

pub use chart::Chart;

/// The chart of mono samples at 22050 Hz.
pub fn analyze(samples: &[f32]) -> Chart {
    back::back(&front::front(samples), None)
}

/// The chart of the audio file at `path`: its title is the file's stem, its `audio` the real
/// path, its `analysis_seconds` the time decoding and analysing took.
pub fn analyze_file(fsio: &FsIo, path: &Path) -> Result<Chart> {
    let start = Instant::now();
    let samples = decode::decode(fsio, path)?;
    let mut chart = analyze(&samples);
    chart.title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    chart.audio = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string();
    chart.analysis_seconds = np::round_to(start.elapsed().as_secs_f64(), 2);
    Ok(chart)
}
