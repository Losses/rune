use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};

use anyhow::Result;
use clap::{Arg, Command};

use tag_editor::shazam::spectrogram::{Signature, compute_signature};

#[tokio::main]
async fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("Shazam Reader CLI")
        .version("1.0")
        .author("Rune Developers")
        .about("Processes text files and outputs spectrograms")
        .arg(
            Arg::new("INPUT")
                .help("Sets the input text file")
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
    let sample_rate = 16000;

    // Compute the signature
    let signature: Signature = compute_signature(sample_rate, &samples);

    // Serialize the signature to JSON
    let serialized_signature = serde_json::to_string(&signature)?;

    // Define the output file name
    let output_file_base = "output";
    let counter = 1; // Adjust as needed
    let output_file = format!("{output_file_base}_{counter}.signature.log");

    // Write the serialized signature to a file
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(output_file)?;

    file.write_all(serialized_signature.as_bytes())?;

    Ok(())
}
