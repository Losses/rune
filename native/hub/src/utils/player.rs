use crate::messages::PlaybackStatus;
use anyhow::{Error, Result, bail};
use database::playing_item::PlayingItemMetadataSummary;
use playback::player::PlayingItem;
use playback::{
    MediaMetadata, MediaPlayback, MediaPosition,
    controller::{MediaControlManager, get_default_cover_art_path},
};
use scrobbling::ScrobblingTrack;
#[cfg(target_os = "macos")]
use std::path;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

pub fn metadata_summary_to_scrobbling_track(
    metadata: &PlayingItemMetadataSummary,
) -> ScrobblingTrack {
    ScrobblingTrack {
        library_id: match &metadata.item {
            PlayingItem::InLibrary(identity) => Some(identity.library_id.to_string()),
            _ => None,
        },
        artist: metadata.artist.clone(),
        album: Some(metadata.album.clone()),
        track: metadata.title.clone(),
        duration: Some(metadata.duration.clamp(0.0, u32::MAX as f64) as u32),
        album_artist: None,
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .map(|time| time.as_secs()),
    }
}

pub(crate) async fn update_media_controls_metadata(
    manager: Arc<Mutex<MediaControlManager>>,
    status: &PlayingItemMetadataSummary,
    cover_art_path: Option<&String>,
) -> Result<()> {
    let mut manager = manager.lock().await;

    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let cover_url = if cover_art_path.is_none() {
        get_default_cover_art_path().to_str()
    } else {
        cover_art_path.map(|x| x.as_str())
    };

    #[cfg(target_os = "macos")]
    let cover_url_owned = {
        let path = match cover_art_path {
            Some(path) => path::Path::new(path),
            None => get_default_cover_art_path(),
        };
        macos_cover_file_url(path)
    };
    #[cfg(target_os = "macos")]
    let cover_url = cover_url_owned.as_deref();

    #[cfg(target_os = "ios")]
    let cover_url_owned = ios_cover_path(
        cover_art_path.map(|x| x.as_str()),
        get_default_cover_art_path(),
    );
    #[cfg(target_os = "ios")]
    let cover_url = cover_url_owned.as_deref();

    let metadata = MediaMetadata {
        title: Some(&status.title),
        album: Some(&status.album),
        artist: Some(&status.artist),
        cover_url,
        duration: Some(Duration::from_secs_f64(status.duration)),
    };

    match manager.controls.set_metadata(metadata) {
        Ok(x) => x,
        Err(e) => bail!(Error::msg(format!("Failed to set media metadata: {e:?}"))),
    };

    Ok(())
}

// Souvlaki's macOS backend passes this value to NSURL URLWithString. A plain
// filesystem path is not a URL, particularly when it contains URL delimiters.
#[cfg(any(target_os = "macos", test))]
fn macos_cover_file_url(path: &std::path::Path) -> Option<String> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    url::Url::from_file_path(path).ok().map(Into::into)
}

// On iOS this value is read as a plain filesystem path, so a cover that is
// relative or gone is replaced by the default cover. Linux and Windows pass
// the path through unchecked, and macOS shows no cover in that case.
#[cfg(any(target_os = "ios", test))]
fn ios_cover_path(path: Option<&str>, default: &std::path::Path) -> Option<String> {
    let usable = |path: &std::path::Path| path.is_absolute() && path.is_file();
    path.map(std::path::Path::new)
        .filter(|path| usable(path))
        .or(Some(default).filter(|path| usable(path)))
        .and_then(|path| path.to_str())
        .map(Into::into)
}

pub(crate) async fn update_media_controls_progress(
    manager: Arc<Mutex<MediaControlManager>>,
    status: &PlaybackStatus,
) -> Result<()> {
    let mut manager = manager.lock().await;

    let playback = match status.state.as_str() {
        "Playing" => MediaPlayback::Playing {
            progress: Some(MediaPosition(Duration::from_secs_f32(
                status.progress_seconds,
            ))),
        },
        "Paused" => MediaPlayback::Paused {
            progress: Some(MediaPosition(Duration::from_secs_f32(
                status.progress_seconds,
            ))),
        },
        _ => MediaPlayback::Stopped,
    };

    match manager.controls.set_playback(playback) {
        Ok(x) => x,
        Err(e) => bail!(Error::msg(format!(
            "Failed to set media playback status: {e:?}"
        ))),
    };

    Ok(())
}

#[cfg(test)]
mod cover_file_url_tests {
    use super::{ios_cover_path, macos_cover_file_url};
    use std::path::Path;
    use std::time::SystemTime;

    #[test]
    fn macos_cover_url_encodes_and_round_trips_local_path() {
        let dir = std::env::temp_dir().join(format!(
            "rune-cover-url-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("album # \u{4f60}\u{597d}.png");
        std::fs::write(&path, b"test fixture").unwrap();
        let url = macos_cover_file_url(&path).unwrap();
        assert!(url.starts_with("file://"));
        assert!(url.contains("%20") && url.contains("%23"));
        assert_eq!(url::Url::parse(&url).unwrap().to_file_path().unwrap(), path);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn macos_cover_url_rejects_missing_relative_and_directory_paths() {
        let dir = std::env::temp_dir();
        assert!(macos_cover_file_url(Path::new("")).is_none());
        assert!(macos_cover_file_url(Path::new("relative.png")).is_none());
        assert!(macos_cover_file_url(&dir.join("rune-cover-url-nonexistent-53a246.png")).is_none());
        assert!(macos_cover_file_url(&dir).is_none());
    }

    #[test]
    fn ios_cover_path_prefers_existing_absolute_file() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.jpg");
        let default = dir.path().join("default.png");
        std::fs::write(&cover, b"test fixture").unwrap();
        std::fs::write(&default, b"test fixture").unwrap();
        assert_eq!(
            ios_cover_path(cover.to_str(), &default),
            cover.to_str().map(String::from)
        );
    }

    #[test]
    fn ios_cover_path_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("default.png");
        std::fs::write(&default, b"test fixture").unwrap();
        let expected = default.to_str().map(String::from);
        let missing = dir.path().join("missing.jpg");
        assert_eq!(
            ios_cover_path(Some("cache/covers/a.jpg"), &default),
            expected
        );
        assert_eq!(ios_cover_path(missing.to_str(), &default), expected);
        assert_eq!(ios_cover_path(dir.path().to_str(), &default), expected);
        assert_eq!(ios_cover_path(None, &default), expected);
    }

    #[test]
    fn ios_cover_path_is_none_without_usable_default() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.png");
        assert_eq!(ios_cover_path(Some("relative.jpg"), &missing), None);
        assert_eq!(ios_cover_path(None, &missing), None);
    }
}
