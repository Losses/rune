use std::fs::File;
use std::io::Read;

use anyhow::{Result, bail};
use clap::{Arg, Command};

use tag_editor::shazam::{
    api::identify,
    spectrogram::{Signature, compute_signature},
};

fn parse_pcm_to_samples(pcm_data: Vec<u8>) -> Result<Vec<f64>> {
    if pcm_data.len() % 2 != 0 {
        bail!("PCM data length is not even, indicating incomplete samples.");
    }

    let mut samples = Vec::new();

    for chunk in pcm_data.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);

        samples.push(sample as f64 / 32768.0);
    }

    Ok(samples)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("PCM Recognizer")
        .version("1.0")
        .author("Rune Developers")
        .about("Processes PCM audio files and identifies them")
        .arg(
            Arg::new("INPUT")
                .help("Sets the input PCM file")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("SAMPLERATE")
                .help("Sets the sample rate of the PCM file")
                .required(true)
                .index(2),
        )
        .get_matches();

    // Get the input file and sample rate from arguments
    let input_file = matches.get_one::<String>("INPUT").unwrap();
    let sample_rate: u32 = matches.get_one::<String>("SAMPLERATE").unwrap().parse()?;

    // Read PCM file
    let mut file = File::open(input_file)?;
    let mut pcm_data = Vec::new();
    file.read_to_end(&mut pcm_data)?;

    let samples = parse_pcm_to_samples(pcm_data)?;

    let signature: Signature = compute_signature(sample_rate.try_into()?, &samples);
    println!("{signature}");

    let identified_result = identify(signature).await;
    println!("{identified_result:?}");

    println!();
    println!();

    Ok(())
}
