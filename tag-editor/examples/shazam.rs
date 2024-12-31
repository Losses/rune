use std::sync::mpsc::channel;

use anyhow::Result;
use clap::{Arg, Command};
use tokio_util::sync::CancellationToken;

use tag_editor::{
    sampler::Sampler,
    shazam::{
        api::identify,
        spectrogram::{compute_signature, Signature},
    },
};

#[tokio::main]
async fn main() -> Result<()> {
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
    let sample_rate = 8000;
    let sample_count = 8;
    let sample_duration = 20.0;

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
        let signature: Signature = compute_signature(event.sample_rate.try_into()?, &event.data);
        println!("{}", signature);

        let identified_result = identify(signature).await;
        println!("{:?}", identified_result);
    }

    Ok(())
}
