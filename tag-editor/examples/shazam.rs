use anyhow::Result;
use clap::{Arg, Command};
use std::sync::mpsc::channel;
use tokio_util::sync::CancellationToken;

use tag_editor::sampler::Sampler;
use tag_editor::shazam::spectrogram::SpectrogramProgessor;

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
    let sample_count = 3;
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
        let mut spectrogram_processor = SpectrogramProgessor::new(5000.0);
        let audio_duration = event.duration;
        spectrogram_processor.pipe_sample_event(event)?;
        // Extract peaks or output spectrogram
        let peaks = spectrogram_processor.extract_peaks(audio_duration);
        println!("{}", peaks);
    }

    Ok(())
}
