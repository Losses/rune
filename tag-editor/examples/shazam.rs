use anyhow::Result;
use clap::{Arg, Command};
use fsio::FsIo;
use tokio_util::sync::CancellationToken;

use tag_editor::{
    sampler::interval_sampler::IntervalSampler,
    shazam::{
        api::identify,
        spectrogram::{Signature, compute_signature},
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
    let fsio = FsIo::new();

    // Set parameters
    let sample_rate = 16000;
    let sample_duration = 12.0;
    let interval_duration = 24.0;

    // Create a cancellation token (if needed)
    let cancel_token = CancellationToken::new();

    // Create a channel for SampleEvents

    // Initialize Sampler
    let mut sampler = IntervalSampler::new(
        input_file,
        sample_duration,
        interval_duration,
        sample_rate,
        Some(cancel_token.clone()),
    );

    // Process the audio file
    sampler.process(&fsio)?;

    // Collect and process sample events
    for event in sampler.receiver.iter() {
        println!("Event:");
        println!("= Sample Index: {}", event.sample_index);
        println!("= Start Time: {:?}", event.start_time);
        println!("= End Time: {:?}", event.end_time);

        let signature: Signature = compute_signature(event.sample_rate.try_into()?, &event.data);
        println!("{signature}");

        let identified_result = identify(signature).await;
        println!("{identified_result:?}");

        println!();
        println!();
    }

    Ok(())
}
