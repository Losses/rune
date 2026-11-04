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
            PlayingItem::InLibrary(_, library_id) => Some(library_id.clone()),
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

    #[cfg(not(target_os = "macos"))]
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
    use super::macos_cover_file_url;
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
}
