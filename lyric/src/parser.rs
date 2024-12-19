use std::{fs, io::Read, path::PathBuf};

use anyhow::Result;

use crate::{lrc::parse_lrc, types::LyricFile};

pub fn parse_audio_lyrics(path: PathBuf) -> Option<Result<LyricFile>> {
    // Get the file name and replace the extension with .lrc
    let mut lrc_path = path.clone();
    lrc_path.set_extension("lrc");

    // Check if the .lrc file exists
    if lrc_path.exists() {
        // Read the file content
        let mut content = String::new();
        match fs::File::open(&lrc_path).and_then(|mut file| file.read_to_string(&mut content)) {
            Ok(_) => Some(parse_lrc(&content)),
            Err(_) => None,
        }
    } else {
        None
    }
}
