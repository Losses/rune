use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};

use crate::{
    lrc::parse_lrc, srt::parse_srt, ttml::parse_ttml, types::LyricFile, utils::strip_bom,
    vtt::parse_vtt,
};

type ParseFn = fn(&str) -> Result<LyricFile>;

/// Sidecar extensions in the order they are tried.
const SIDECARS: [(&str, ParseFn); 5] = [
    ("ttml", parse_ttml),
    ("lrc", parse_lrc),
    ("lrcx", parse_lrc),
    ("vtt", parse_vtt),
    ("srt", parse_srt),
];

/// Returns the first sidecar that parses; when none does, the last parse error.
pub fn parse_audio_lyrics(path: PathBuf) -> Option<Result<LyricFile>> {
    let mut last_error = None;

    for (extension, parse_fn) in SIDECARS {
        match parse_lyrics_with_extension(&path, extension, parse_fn) {
            Some(Ok(lyric)) => return Some(Ok(lyric)),
            Some(Err(err)) => {
                log::warn!("Unable to parse the .{extension} lyric of {path:?}: {err:#}");
                last_error = Some(Err(err));
            }
            None => {}
        }
    }

    last_error
}

/// UTF-8 first, then whatever encoding the bytes look like (GBK, Big5, Shift-JIS, ...).
fn decode_text(bytes: &[u8]) -> Cow<'_, str> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Cow::Borrowed(strip_bom(text));
    }

    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    let (text, _, _) = detector.guess(None, Utf8Detection::Allow).decode(bytes);
    text
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
        match fs::read(&file_path) {
            Ok(bytes) => Some(parse_fn(&decode_text(&bytes))),
            Err(err) => {
                log::warn!("Unable to read {file_path:?}: {err}");
                None
            }
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_text_legacy_encodings_and_bom() {
        assert_eq!(
            decode_text("\u{FEFF}[00:01.00]歌".as_bytes()),
            "[00:01.00]歌"
        );

        let (gbk, _, _) = encoding_rs::GBK.encode("[00:01.00]我们的歌曲在这里唱给你听");
        assert_eq!(decode_text(&gbk), "[00:01.00]我们的歌曲在这里唱给你听");
    }

    #[test]
    fn test_decode_text_utf16_with_bom() {
        let text = "[00:01.00]歌";
        let mut le = vec![0xFF, 0xFE];
        let mut be = vec![0xFE, 0xFF];
        for unit in text.encode_utf16() {
            le.extend(unit.to_le_bytes());
            be.extend(unit.to_be_bytes());
        }
        assert_eq!(decode_text(&le), text);
        assert_eq!(decode_text(&be), text);
    }

    #[test]
    fn test_sidecar_falls_through_to_next_extension() {
        let dir = std::env::temp_dir().join(format!("lyric-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("song.ttml"), "not xml").unwrap();
        let (gbk, _, _) = encoding_rs::GBK.encode("[00:01.00]我们的歌曲在这里唱给你听");
        fs::write(dir.join("song.lrc"), &*gbk).unwrap();

        let lyric = parse_audio_lyrics(dir.join("song.flac")).unwrap().unwrap();
        assert_eq!(lyric.lyrics.len(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }
}
