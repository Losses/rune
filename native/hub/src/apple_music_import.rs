//! Imports the playlists and the liked songs of the user's Apple Music library into Rune's
//! playlists and likes kept on this device, each once per library. Apple Music content comes into
//! Rune once and what Rune changes afterwards stays in Rune: nothing Rune does writes to the
//! user's Apple account.

use std::collections::HashSet;

use anyhow::Result;
use apple_music::models::LibraryPlaylist;

use crate::apple_music_account;
use crate::apple_music_binding::{AppleMusicBinding, SongRef, is_library_song, listed_track};
use crate::apple_music_translate::{APPLE_MUSIC_PLAYLIST_GROUP, track_about};

/// How many songs of the library have their ratings read in one step.
const RATING_BATCH: usize = 400;

/// Imports what comes in from Apple once: first the Rune playlists, mixes and likes that a
/// removed library of the same account left are taken over, with the notes that they were
/// imported (see `apple_music_account::attach`); then the library's playlists, unless they were
/// imported before (see `import_playlists`); then its likes, unless they were imported before
/// (see `import_likes`); last, the ids of the account are kept beside the records (see
/// `apple_music_account::note`). Each step that fails is logged and left for the next sign-in.
pub(crate) async fn import(binding: &AppleMusicBinding) {
    let changes = binding.token_changes();
    let (ready, mut read) = apple_music_account::attach(binding).await;
    if !ready {
        return;
    }
    if !binding.rune_playlists.imported().await {
        match import_playlists(binding, read.take()).await {
            Ok((complete, playlists)) => {
                read = Some(playlists);
                if complete && let Err(error) = binding.rune_playlists.mark_imported().await {
                    log::warn!(
                        "Could not note that the Apple Music playlists were imported: {error:#}"
                    );
                }
            }
            Err(error) => log::warn!(
                "Could not import the Apple Music playlists, trying again at the next sign-in: {error:#}"
            ),
        }
    }
    if !binding.rune_likes.imported().await {
        match import_likes(binding).await {
            Ok(true) => {
                if let Err(error) = binding.rune_likes.mark_imported().await {
                    log::warn!(
                        "Could not note that the Apple Music likes were imported: {error:#}"
                    );
                }
            }
            Ok(false) => {}
            Err(error) => log::warn!(
                "Could not import the Apple Music likes, trying again at the next sign-in: {error:#}"
            ),
        }
    }
    // What was read may be of the account that signed in before; the import that the new
    // sign-in starts notes its own.
    if binding.token_changes() != changes {
        read = None;
    }
    apple_music_account::note(binding, read.as_deref()).await;
}

/// Imports the playlists of the library into Rune's, with their names and their songs in order,
/// in the group of Apple Music's playlists, from `read` or else as Apple has them now. Every
/// playlist of the library comes in, those the user cannot change too, as the Favourite Songs or
/// one subscribed to: from then on they are Rune's own and every change to them stays in Rune.
/// Each is written whole as it is imported, so an import cut short imports what is still missing
/// at the next sign-in and nothing twice, and the lists that show the playlists reload. The
/// playlists are read one after another, so the import adds little to what Apple is asked at
/// once. One that fails is logged and left for the next sign-in. A change of the sign-in stops
/// it. Answers whether every playlist is in, and Apple's playlists.
async fn import_playlists(
    binding: &AppleMusicBinding,
    read: Option<Vec<LibraryPlaylist>>,
) -> Result<(bool, Vec<LibraryPlaylist>)> {
    let changes = binding.token_changes();
    let playlists = match read {
        Some(playlists) => playlists,
        None => binding
            .read_library_playlists()
            .await
            .map_err(|error| binding.request_error(error, "Apple Music playlist"))?,
    };
    let imported: HashSet<String> = binding
        .rune_playlists
        .list()
        .await?
        .iter()
        .filter_map(|playlist| playlist.imported_from.clone())
        .collect();
    let mut complete = true;
    for playlist in playlists
        .iter()
        .filter(|playlist| !imported.contains(&playlist.id))
    {
        if binding.token_changes() != changes {
            log::info!("Stopped importing the Apple Music playlists: the sign-in changed");
            return Ok((false, playlists));
        }
        match import_playlist(binding, playlist).await {
            Ok(true) => binding.content_changed(),
            Ok(false) => {}
            Err(error) => {
                log::warn!(
                    "Could not import an Apple Music playlist, trying again at the next sign-in: {error:#}"
                );
                complete = false;
            }
        }
    }
    Ok((complete && binding.token_changes() == changes, playlists))
}

/// Imports one playlist; answers whether it was made, which it is not when it was imported
/// meanwhile.
async fn import_playlist(binding: &AppleMusicBinding, playlist: &LibraryPlaylist) -> Result<bool> {
    let songs = binding
        .read_library_playlist_songs(&playlist.id)
        .await
        .map_err(|error| binding.request_error(error, "Apple Music playlist"))?;
    let tracks = songs
        .iter()
        .filter_map(|song| {
            let track = listed_track(song)?;
            let about = track_about(&track, SongRef::Library(song));
            Some((track, about))
        })
        .collect();
    let name = playlist
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.name.as_deref())
        .unwrap_or_default();
    binding
        .rune_playlists
        .import(name, APPLE_MUSIC_PLAYLIST_GROUP, &playlist.id, tracks)
        .await
}

/// Imports the songs the user likes on Apple into Rune's likes. Apple has no list of liked songs,
/// only a rating per song, so the ratings of the library's songs are read in steps. The liked
/// songs are added once every step was read, in the order the library lists them, which is the
/// order they were added in, as older than every song Rune's likes know: a song liked or unliked
/// in Rune meanwhile or before keeps what the user made of it, and one unliked in Rune is not
/// liked again, under either key of the song. Answers whether the import is finished; it is not
/// when the sign-in changed meanwhile, which stops it. A step that fails fails it, and the next
/// sign-in reads everything again.
async fn import_likes(binding: &AppleMusicBinding) -> Result<bool> {
    let changes = binding.token_changes();
    let list = binding.songs().await?;
    let tracks: Vec<String> = list
        .entries()
        .filter(|(_, song)| is_library_song(song))
        .map(|(track, _)| track.to_owned())
        .collect();
    let mut liked = Vec::new();
    for batch in tracks.chunks(RATING_BATCH) {
        let rated = binding
            .liked_library_songs(batch)
            .await
            .map_err(|error| binding.request_error(error, "Apple Music rating"))?;
        if binding.token_changes() != changes {
            log::info!("Stopped importing the Apple Music likes: the sign-in changed");
            return Ok(false);
        }
        liked.extend(
            batch
                .iter()
                .filter(|track| rated.contains(*track))
                .map(|track| binding.like_keys(track)),
        );
    }
    if !liked.is_empty() {
        binding.rune_likes.import(liked).await?;
    }
    Ok(true)
}
