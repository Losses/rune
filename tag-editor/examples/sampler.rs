use std::fs::File;
use std::io::Write;
use std::sync::mpsc::channel;

use anyhow::Result;
use clap::{Arg, Command};
use tokio_util::sync::CancellationToken;

use tag_editor::sampler::Sampler;

fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("Sampler CLI")
        .version("1.0")
        .author("Rune Developers")
        .about("Processes audio files and outputs sample data")
        .arg(
            Arg::new("INPUT")
                .help("Sets the input audio file")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("OUTPUT")
                .help("Sets the output text file base name")
                .required(true)
                .index(2),
        )
        .get_matches();

    // Get the input and output files from arguments
    let input_file = matches.get_one::<String>("INPUT").unwrap();
    let output_file_base = matches.get_one::<String>("OUTPUT").unwrap();

    // Set parameters
    let sample_rate = 16000;
    let sample_count = 4;
    let sample_duration = 10.0;

    // Create a cancellation token (if needed)
    let cancel_token = CancellationToken::new();

    // Create a channel for SampleEvents
    let (sender, receiver) = channel();

    // Initialize Sampler
    let mut sampler = Sampler::new(
        sample_duration,
        sample_count,
        sample_rate,
        Some(cancel_token.clone()),
    );

    // Process the audio file
    sampler.process(input_file, sender)?;

    for (counter, event) in receiver.iter().enumerate() {
        // Create a numbered output file name
        let output_file = format!("{}_{}.txt", output_file_base, counter);

        // Log the output file path
        println!("Writing to output file: {}", output_file);

        // Open the output file for writing
        let mut file = File::create(&output_file)?;

        // Write the event data to the file
        writeln!(
            file,
            "{}",
            event
                .data
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )?;
    }

    Ok(())
}
