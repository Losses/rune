use anyhow::{Error, Result, bail};
use database::playing_item::PlayingItemMetadataSummary;
use playback::player::PlayingItem;
use playback::{
    MediaMetadata, MediaPlayback,
    controller::{MediaControlManager, get_default_cover_art_path},
};
use scrobbling::ScrobblingTrack;
#[cfg(any(target_os = "macos", target_os = "ios", test))]
use std::path;
#[cfg(any(target_os = "ios", test))]
use std::path::Path;
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;
#[cfg(any(target_os = "macos", test))]
use url::Url;

/// Describes a listen of `metadata` for the scrobbling services. `started_at` is when the listen
/// began, and the final submission carries it as the listen's time. With `None` the service
/// client stamps the submission with the time it is sent.
pub fn metadata_summary_to_scrobbling_track(
    metadata: &PlayingItemMetadataSummary,
    started_at: Option<SystemTime>,
) -> ScrobblingTrack {
    ScrobblingTrack {
        library_id: match &metadata.item {
            PlayingItem::LibraryTrack { library_id, .. } => Some(library_id.to_string()),
            _ => None,
        },
        artist: metadata.artist.clone(),
        album: Some(metadata.album.clone()),
        track: metadata.title.clone(),
        duration: Some(metadata.duration.clamp(0.0, u32::MAX as f64)),
        album_artist: None,
        timestamp: started_at
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
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
fn macos_cover_file_url(path: &path::Path) -> Option<String> {
    if !path.is_absolute() || !path.is_file() {
        return None;
    }
    Url::from_file_path(path).ok().map(Into::into)
}

// On iOS this value is read as a plain filesystem path, so a cover that is
// relative or gone is replaced by the default cover. Linux and Windows pass
// the path through unchecked, and macOS shows no cover in that case.
#[cfg(any(target_os = "ios", test))]
fn ios_cover_path(path: Option<&str>, default: &path::Path) -> Option<String> {
    let usable = |path: &path::Path| path.is_absolute() && path.is_file();
    path.map(Path::new)
        .filter(|path| usable(path))
        .or(Some(default).filter(|path| usable(path)))
        .and_then(|path| path.to_str())
        .map(Into::into)
}

/// `buffering` says that a platform host's track that is to play does not move on yet.
pub(crate) async fn update_media_controls_progress(
    manager: Arc<Mutex<MediaControlManager>>,
    playback: MediaPlayback,
    buffering: bool,
) -> Result<()> {
    let mut manager = manager.lock().await;

    match manager.set_playback(playback, buffering) {
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
    use std::env;
    use std::fs;
    use std::path::Path;
    use std::process as std_process;
    use std::time as std_time;
    use std::time::SystemTime;
    use url::Url;

    #[test]
    fn macos_cover_url_encodes_and_round_trips_local_path() {
        let dir = env::temp_dir().join(format!(
            "rune-cover-url-{}-{}",
            std_process::id(),
            SystemTime::now()
                .duration_since(std_time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("album # \u{4f60}\u{597d}.png");
        fs::write(&path, b"test fixture").unwrap();
        let url = macos_cover_file_url(&path).unwrap();
        assert!(url.starts_with("file://"));
        assert!(url.contains("%20") && url.contains("%23"));
        assert_eq!(Url::parse(&url).unwrap().to_file_path().unwrap(), path);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn macos_cover_url_rejects_missing_relative_and_directory_paths() {
        let dir = env::temp_dir();
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
        fs::write(&cover, b"test fixture").unwrap();
        fs::write(&default, b"test fixture").unwrap();
        assert_eq!(
            ios_cover_path(cover.to_str(), &default),
            cover.to_str().map(String::from)
        );
    }

    #[test]
    fn ios_cover_path_falls_back_to_default() {
        let dir = tempfile::tempdir().unwrap();
        let default = dir.path().join("default.png");
        fs::write(&default, b"test fixture").unwrap();
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

#[cfg(test)]
mod scrobbling_track_tests {
    use super::metadata_summary_to_scrobbling_track;
    use database::playing_item::PlayingItemMetadataSummary;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn a_listen_is_scrobbled_at_its_start() {
        let started_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let track = metadata_summary_to_scrobbling_track(
            &PlayingItemMetadataSummary::default(),
            Some(started_at),
        );

        assert_eq!(track.timestamp, Some(1_700_000_000));
    }

    #[test]
    fn a_listen_without_a_start_leaves_the_time_to_the_service_client() {
        let track =
            metadata_summary_to_scrobbling_track(&PlayingItemMetadataSummary::default(), None);

        assert_eq!(track.timestamp, None);
    }
}
