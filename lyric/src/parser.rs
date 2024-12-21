use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::{lrc::parse_lrc, ttml::parse_ttml, types::LyricFile, vtt::parse_vtt};

pub fn parse_audio_lyrics(path: PathBuf) -> Option<Result<LyricFile>> {
    // Try to find and parse the .ttml file
    if let Some(lyric) = parse_lyrics_with_extension(&path, "ttml", parse_ttml) {
        return Some(lyric);
    }

    // Try to find and parse the .lrc file
    if let Some(lyric) = parse_lyrics_with_extension(&path, "lrc", parse_lrc) {
        return Some(lyric);
    }

    // Try to find and parse the .vtt file
    if let Some(lyric) = parse_lyrics_with_extension(&path, "vtt", parse_vtt) {
        return Some(lyric);
    }

    None
}

fn parse_lyrics_with_extension<F>(
    path: &Path,
    extension: &str,
    parse_fn: F,
) -> Option<Result<LyricFile>>
where
    F: Fn(&str) -> Result<LyricFile>,
{
    let mut file_path = path.to_path_buf();
    file_path.set_extension(extension);

    if file_path.exists() {
        let mut content = String::new();
        match fs::File::open(&file_path).and_then(|mut file| file.read_to_string(&mut content)) {
            Ok(_) => Some(parse_fn(&content)),
            Err(_) => None,
        }
    } else {
        None
    }
}
