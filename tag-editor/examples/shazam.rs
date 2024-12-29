use anyhow::Result;
use clap::{Arg, Command};
use std::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;

use tag_editor::{
    sampler::Sampler,
    shazam::{signature::Signature, spectrogram::SpectrogramProcessor},
};

fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("Spectrogram CLI")
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

    // Collect and process sample events
    for event in receiver.iter() {
        // Initialize SpectrogramProcessor
        let mut spectrogram_processor = SpectrogramProcessor::new(11025.0);
        spectrogram_processor.process_samples(&event.data)?;
        // Extract peaks or output spectrogram
        let peaks = spectrogram_processor.extract_peaks();
        println!("{}", peaks);
        let signature: Signature = peaks.into();
        let encoded_data = signature.encode();

        println!("Encoded signature lengtrh: {:?}", encoded_data.len());
    }

    Ok(())
}
