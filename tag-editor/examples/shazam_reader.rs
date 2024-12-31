use anyhow::Result;
use clap::{Arg, Command};
use std::fs::File;
use std::io::{BufRead, BufReader};

use tag_editor::shazam::spectrogram::{compute_signature, Signature};

#[tokio::main]
async fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("Shazam Reader CLI")
        .version("1.0")
        .author("Rune Developers")
        .about("Processes audio files and outputs spectrograms")
        .arg(
            Arg::new("INPUT")
                .help("Sets the input audio file")
                .required(true)
                .index(1),
        )
        .get_matches();

    // Get the input file from arguments
    let input_file = matches.get_one::<String>("INPUT").unwrap();

    // Open the input file
    let file = File::open(input_file)?;
    let reader = BufReader::new(file);

    // Read samples from the file
    let mut samples = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let sample: f64 = line.parse()?;
        samples.push(sample);
    }

    // Set the sample rate
    let sample_rate = 11025;

    // Compute the signature
    let signature: Signature = compute_signature(sample_rate, &samples);

    // Print the signature (mimicking the Go code's output)
    for (band, peaks) in signature.peaks_by_band.iter().enumerate() {
        println!("Band {}: {} peaks", band, peaks.len());

        for (i, peak) in peaks.iter().enumerate() {
            println!(
                "  Peak {}: {{Pass: {}, Magnitude: {}, Bin: {}}}",
                i + 1,
                peak.pass,
                peak.magnitude,
                peak.bin
            );
        }
    }

    Ok(())
}
