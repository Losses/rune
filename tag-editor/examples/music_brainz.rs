use anyhow::Result;
use clap::{Arg, Command};
use rusty_chromaprint::Configuration;

use ::fsio::FsIo;
use ::tag_editor::music_brainz::{api::identify, fingerprint::calc_fingerprint};

#[tokio::main]
async fn main() -> Result<()> {
    // Set up CLI arguments
    let matches = Command::new("MusicBrainz Audio Identifier")
        .version("1.0")
        .author("Rune Developers")
        .about("Identifies audio files using AcoustID")
        .arg(
            Arg::new("API_KEY")
                .help("Sets the AcoustID API key")
                .required(true)
                .index(1),
        )
        .arg(
            Arg::new("FILE_PATH")
                .help("Sets the input audio file path")
                .required(true)
                .index(2),
        )
        .get_matches();

    // Get the API key and file path from arguments
    let api_key = matches.get_one::<String>("API_KEY").unwrap();
    let file_path = matches.get_one::<String>("FILE_PATH").unwrap();
    let fsio = FsIo::new();

    // Create Chromaprint configuration
    let config = Configuration::default();

    // Calculate fingerprint
    let (fingerprint, duration) = calc_fingerprint(&fsio, file_path, &config)?;

    // Call the identify function
    match identify(
        api_key,
        fingerprint,
        &config,
        duration.as_secs().try_into()?,
    )
    .await
    {
        Ok(response) => {
            println!("Identification successful: {:#?}", { response });
        }
        Err(e) => {
            eprintln!("Identification failed: {e:?}");
        }
    }

    Ok(())
}
