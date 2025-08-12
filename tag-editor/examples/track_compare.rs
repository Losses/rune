use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use rusty_chromaprint::match_fingerprints;

use ::fsio::FsIo;
use ::tag_editor::music_brainz::fingerprint::{
    calc_fingerprint, calculate_similarity_score, get_track_duration_in_secs,
};

#[derive(Parser, Debug)]
#[command(version, about = "Audio fingerprint comparison tool", long_about = None)]
struct CliArgs {
    /// Path to first audio file
    file1: PathBuf,

    /// Path to second audio file
    file2: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();

    // Initialize Chromaprint configuration with default parameters
    let config = rusty_chromaprint::Configuration::default();
    let fsio = FsIo::new();

    // Process first file
    println!("Processing {}...", args.file1.display());
    let (fp1, duration1) =
        calc_fingerprint(&fsio, &args.file1, &config).context("Failed to process first file")?;
    println!(
        "Processed {} ({:.2}s)",
        args.file1.display(),
        duration1.as_secs_f32()
    );

    // Process second file
    println!("\nProcessing {}...", args.file2.display());
    let (fp2, duration2) =
        calc_fingerprint(&fsio, &args.file2, &config).context("Failed to process second file")?;
    println!(
        "Processed {} ({:.2}s)",
        args.file2.display(),
        duration2.as_secs_f32()
    );

    // Find matching segments
    let segments =
        match_fingerprints(&fp1, &fp2, &config).context("Fingerprint comparison failed")?;

    // Print results in a formatted way
    println!("\nMATCHING SEGMENTS:");
    println!("{}", "-".repeat(80));
    println!(
        "{:<12} {:<12} {:<12} {:<12} {:<12} {:<12}",
        "Start 1", "Start 2", "Duration(s)", "Items", "Score", "Quality"
    );
    println!("{}", "-".repeat(80));

    for seg in &segments {
        let duration = seg.duration(&config);
        let quality = 1.0 - (seg.score / 32.0);
        let score = seg.score;
        println!(
            "{:<12} {:<12} {:<12.2} {:<12} {:<12.3} {:<12.3}",
            seg.offset1, seg.offset2, duration, seg.items_count, score, quality
        );
    }
    println!("{}", "-".repeat(80));

    // Calculate and display final similarity score
    let similarity = calculate_similarity_score(
        &segments,
        get_track_duration_in_secs(&fp1, &config).max(get_track_duration_in_secs(&fp2, &config)),
        &config,
    );
    println!("\nOverall similarity score: {:.3}", { similarity });

    Ok(())
}
