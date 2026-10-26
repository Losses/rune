//! `analyze FILE... --out DIR`: analyse each file into `DIR/<stem>.json` (the stem of the given
//! path, as the prototype names its charts) and print how long decoding and analysing took
//! (the front end runs as the file is decoded, so the time is split between its two halves
//! only after that: the front end with the decoding, then the back end).
//!
//! A file ending in `.f32` is read as raw little-endian `f32` mono samples at 22050 Hz instead
//! of decoded, so that the back end can be fed exactly the samples the Python analysed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use chart_analyzer::{back::back, decode::decode_with, front::FrontBuilder, np::round_to};
use fsio::FsIo;

/// Feeds the samples of a raw `.f32` file to `front`, a piece at a time.
fn read_raw(path: &Path, front: &mut FrontBuilder) -> Result<()> {
    let mut file = std::fs::File::open(path)?;
    let (mut bytes, mut samples) = (vec![0u8; 4 * 16384], Vec::new());
    let mut filled = 0;
    loop {
        let n = file.read(&mut bytes[filled..])?;
        filled += n;
        let whole = filled / 4 * 4;
        samples.clear();
        samples.extend(
            bytes[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b)),
        );
        front.push(&samples);
        bytes.copy_within(whole..filled, 0);
        filled -= whole;
        if n == 0 {
            return Ok(());
        }
    }
}

fn main() -> Result<()> {
    let mut files = vec![];
    let mut out = PathBuf::from("out");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().context("--out needs a directory")?.into(),
            _ => files.push(PathBuf::from(a)),
        }
    }
    if files.is_empty() {
        bail!("usage: analyze FILE... --out DIR");
    }
    std::fs::create_dir_all(&out)?;
    let fsio = FsIo::new();
    for file in files {
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let t0 = Instant::now();
        let mut front = FrontBuilder::new();
        let read = if file.extension().is_some_and(|e| e == "f32") {
            read_raw(&file, &mut front)
        } else {
            decode_with(&fsio, &file, &AtomicBool::new(false), &mut |samples| {
                front.push(samples)
            })
        };
        if let Err(e) = read {
            println!("{stem}: failed: {e:#}");
            continue;
        }
        let front = front.finish();
        let decode_s = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        let mut chart = back(&front, None);
        let analysis_s = t1.elapsed().as_secs_f64();
        chart.title = stem.clone();
        chart.audio = file
            .canonicalize()
            .unwrap_or(file.clone())
            .display()
            .to_string();
        chart.analysis_seconds = round_to(decode_s + analysis_s, 2);
        std::fs::write(
            out.join(format!("{stem}.json")),
            serde_json::to_vec(&chart)?,
        )?;
        println!(
            "{stem}: decode and front {decode_s:.2} s, back {analysis_s:.2} s, {} bpm conf {}, {} beats",
            chart.tempo.bpm,
            chart.tempo.confidence,
            chart.beats.len()
        );
    }
    Ok(())
}
