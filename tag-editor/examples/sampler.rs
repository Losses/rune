use std::io::Write;
use std::{fs::File, sync::Arc};

use anyhow::Result;
use clap::{Arg, Command};
use fsio::FsIo;
use tokio_util::sync::CancellationToken;

use tag_editor::sampler::interval_sampler::IntervalSampler;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
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
                .help("Sets the output file base name")
                .required(true)
                .index(2),
        )
        .get_matches();

    // Get the input and output files from arguments
    let input_file = matches.get_one::<String>("INPUT").unwrap();
    let output_file_base = matches.get_one::<String>("OUTPUT").unwrap();

    // Set parameters
    let sample_rate = 16000;
    let sample_duration = 12.0;
    let interval_duration = 24.0;

    // Create a cancellation token (if needed)
    let cancel_token = CancellationToken::new();

    // Initialize Sampler
    let mut sampler = IntervalSampler::new(
        input_file,
        sample_duration,
        interval_duration,
        sample_rate,
        Some(cancel_token.clone()),
    );

    let fsio = Arc::new(FsIo::new());
    // Process the audio file
    sampler.process(&fsio)?;

    for (counter, event) in sampler.receiver.iter().enumerate() {
        // Create numbered output file names for both txt and pcm
        let txt_file = format!("{output_file_base}_{counter}.sample.log");
        // You can use ` ffplay ./YOUR_FILE.pcm.log  -f s16le -ar 8000` to debug this
        let pcm_file = format!("{output_file_base}_{counter}.pcm.log");

        // Log the output file paths
        println!("Writing to text file: {txt_file}");
        println!("Writing to PCM file: {pcm_file}");

        // Write text file
        let mut text_file = File::create(&txt_file)?;
        writeln!(
            text_file,
            "{}",
            event
                .data
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        )?;

        // Write PCM file
        let mut pcm_file = File::create(&pcm_file)?;
        for sample in &event.data {
            // Convert f32 to i16 PCM
            let pcm_sample = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            pcm_file.write_all(&pcm_sample.to_le_bytes())?;
        }
    }

    Ok(())
}
