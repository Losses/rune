//! A port of a Python prototype of the music analyzer that analyses a song into a
//! `Chart`. Parity with the Python is the goal, so that the
//! modules follow its functions one for one: `front` is `front()`, `back` is `back()` with the
//! tempo and beat pieces in `tempo`, the sections and phrases in `sections` and the event
//! streams and lead in `streams`, `decode` is `load()`.

pub mod back;
pub mod chart;
pub mod decode;
pub mod front;
#[cfg(test)]
mod front_whole;
pub mod ndimage;
pub mod np;
pub mod sections;
pub mod streams;
pub mod tempo;

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use fsio::FsIo;

pub use chart::Chart;

/// The chart of mono samples at 22050 Hz.
pub fn analyze(samples: &[f32]) -> Chart {
    back::back(&front::front(samples), None)
}

/// The chart of the audio file at `path`: its title is the file's stem, its `audio` the path
/// as given, its `analysis_seconds` the time decoding and analysing took. Setting `cancel` (from
/// another thread) stops it at the next decoded packet with [`decode::Cancelled`], so that an
/// analysis nobody waits for any more does not run to its end.
pub fn analyze_file(fsio: &FsIo, path: &Path, cancel: &AtomicBool) -> Result<Chart> {
    let start = Instant::now();
    let mut front = front::FrontBuilder::new();
    decode::decode_with(fsio, path, cancel, &mut |samples| front.push(samples))?;
    // The back end takes a good part of a second and cannot be stopped half way.
    if cancel.load(Ordering::Relaxed) {
        return Err(decode::Cancelled.into());
    }
    let mut chart = back::back(&front.finish(), None);
    chart.title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    // As given: on Android it can be a content URI, which is no real path.
    chart.audio = path.display().to_string();
    chart.analysis_seconds = np::round_to(start.elapsed().as_secs_f64(), 2);
    Ok(chart)
}
