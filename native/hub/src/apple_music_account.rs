//! Tells which Apple Music account the Rune playlists, mixes and likes a removed library left
//! belong to, so a library added again takes them over only for the same account. Apple tells
//! apps no account id, but the ids of a library's songs and playlists are its account's alone:
//! some of them are kept beside the records (see `AccountIds`), and a library added again takes
//! over what a removed one left when it holds one of those ids. What another account left stays
//! as it is, so going back to that account takes it over then.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;

use apple_music::models::LibraryPlaylist;
use database::library_registry::LibraryRegistry;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::apple_music_binding::{
    AppleError, AppleMusicBinding, is_library_song, is_library_song_id,
};
use crate::service::device_records::{device_records_folder, read_records};
use crate::service::folder::{has_device_records, left_device_records, take_over_device_records};
use crate::service::rune_mixes::{MIXES_FILE, StoredMix};
use crate::service::rune_playlists::{PLAYLISTS_FILE, StoredPlaylist, account_file};

/// How many ids of each kind are kept.
const KEPT_IDS: usize = 64;

/// The format of the file that keeps them.
const FORMAT: u32 = 1;

/// Some ids of the songs and playlists of an account's library, as the account file beside the
/// records keeps them: the smallest of those known, so mostly the same ones are kept as the
/// library changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AccountIds {
    format: u32,
    songs: Vec<String>,
    playlists: Vec<String>,
}

/// The first `KEPT_IDS` of `ids` in order, each once.
fn smallest(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(KEPT_IDS);
    ids
}

/// Whether `id` names a playlist of a library, not of the catalog.
fn is_library_playlist(id: &str) -> bool {
    id.starts_with("p.")
}

impl AccountIds {
    fn new(
        songs: impl IntoIterator<Item = String>,
        playlists: impl IntoIterator<Item = String>,
    ) -> Self {
        Self {
            format: FORMAT,
            songs: smallest(songs.into_iter().filter(|id| is_library_song_id(id))),
            playlists: smallest(playlists.into_iter().filter(|id| is_library_playlist(id))),
        }
    }

    fn is_empty(&self) -> bool {
        self.songs.is_empty() && self.playlists.is_empty()
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Self>(bytes)
            .ok()
            .filter(|ids| ids.format == FORMAT)
    }

    /// The library ids the playlists and mixes hold: the songs they name by their library
    /// tracks, and the playlists the playlists were imported from.
    fn held_by(playlists: &[StoredPlaylist], mixes: &[StoredMix]) -> Self {
        let songs = playlists
            .iter()
            .flat_map(|playlist| &playlist.entries)
            .map(|entry| entry.track.clone())
            .chain(
                mixes
                    .iter()
                    .flat_map(|mix| &mix.queries)
                    .filter(|query| query.operator == "lib::track")
                    .map(|query| query.parameter.clone()),
            );
        let imported_from = playlists
            .iter()
            .filter_map(|playlist| playlist.imported_from.clone());
        Self::new(songs, imported_from)
    }

    /// The ids the records folder `folder` holds, read in place: the playlists' and the mixes'.
    fn held_in(folder: &Path) -> Self {
        Self::held_by(
            &read_records(&folder.join(PLAYLISTS_FILE)),
            &read_records(&folder.join(MIXES_FILE)),
        )
    }

    /// The ids that tell whose the records the removed library `removed` left are: as its account
    /// file keeps them, or else, for records without one, as the records hold them.
    fn left_by(config_dir: &Path, removed: Uuid) -> Self {
        let folder = device_records_folder(config_dir, removed);
        std::fs::read(account_file(&folder))
            .ok()
            .and_then(|bytes| Self::parse(&bytes))
            .unwrap_or_else(|| Self::held_in(&folder))
    }
}

/// The records the library registry in `config_dir` holds, valid or not; None when it cannot be
/// read.
fn known_records(config_dir: &Path) -> Option<HashSet<Uuid>> {
    match LibraryRegistry::load(config_dir) {
        Ok(registry) => Some(
            registry
                .libraries
                .iter()
                .chain(registry.invalid_records().map(|(record, _)| record))
                .map(|record| record.record_id.0)
                .collect(),
        ),
        Err(error) => {
            log::warn!("Could not read the library registry to find Rune records left: {error:#}");
            None
        }
    }
}

/// Tells, as the binding of the Apple Music library `library_id` is made, whether its Rune
/// playlists, mixes and likes must wait to be told whose they are (see `attach`): that is when
/// the library has no records of its own yet and a removed library left some. It reads files in
/// place: it runs as the binding is made, where nothing can wait for it.
pub(crate) fn prepare(config_dir: &Path, library_id: Uuid) -> bool {
    if has_device_records(config_dir, library_id) {
        return false;
    }
    known_records(config_dir)
        .is_some_and(|known| !left_device_records(config_dir, library_id, &known).is_empty())
}

/// The removed libraries that left records, with the ids that tell whose they are, the one
/// written to last first; None when the registry cannot be read, which does not tell that
/// nothing was left. Records that hold no id of a library cannot be told apart, and are left.
fn scan_left(config_dir: &Path, library_id: Uuid) -> Option<Vec<(Uuid, AccountIds)>> {
    let known = known_records(config_dir)?;
    Some(
        left_device_records(config_dir, library_id, &known)
            .into_iter()
            .map(|removed| (removed, AccountIds::left_by(config_dir, removed)))
            .filter(|(_, ids)| !ids.is_empty())
            .collect(),
    )
}

/// Whether the library `binding` signs in to holds any of `ids`, its playlists being
/// `playlists`. Fails when Apple cannot tell.
async fn holds_any(
    binding: &AppleMusicBinding,
    ids: &AccountIds,
    playlists: &HashSet<&str>,
) -> Result<bool, AppleError> {
    if ids
        .playlists
        .iter()
        .any(|id| playlists.contains(id.as_str()))
    {
        return Ok(true);
    }
    binding.holds_any_song(&ids.songs).await
}

/// The removed library whose records belong to the account `binding` signs in to, of those
/// `left`; None when none does. Fails when Apple cannot tell. Answers Apple's playlists too.
async fn owner_of(
    binding: &AppleMusicBinding,
    left: &[(Uuid, AccountIds)],
) -> Result<(Option<Uuid>, Vec<LibraryPlaylist>), AppleError> {
    let playlists = binding.read_library_playlists().await?;
    let ids: HashSet<&str> = playlists
        .iter()
        .map(|playlist| playlist.id.as_str())
        .collect();
    for (removed, left_ids) in left {
        if holds_any(binding, left_ids, &ids).await? {
            return Ok((Some(*removed), playlists));
        }
    }
    Ok((None, playlists))
}

/// Tells whose the Rune playlists, mixes and likes removed Apple Music libraries left are, when
/// the binding's records wait for that (see `prepare`), and takes over those of the account the
/// binding signs in to: of several, those written to last. Without any, the library starts with
/// none of its own and imports its account's playlists. What another account left stays as it is.
/// When Apple cannot tell, or the sign-in changes meanwhile, the records keep waiting until the
/// next sign-in, so nothing is written that would keep the library from taking its own over.
/// Answers whether the records are ready, with Apple's playlists when they were read for it.
pub(crate) async fn attach(binding: &AppleMusicBinding) -> (bool, Option<Vec<LibraryPlaylist>>) {
    if !binding.records_held.load(Ordering::SeqCst) {
        return (true, None);
    }
    let (config_dir, library_id) = (binding.config_dir.clone(), binding.library_id);
    // The files and the registry are read off the async threads.
    let scanned = tokio::task::spawn_blocking(move || scan_left(&config_dir, library_id)).await;
    let left = match scanned {
        Ok(Some(left)) => left,
        Ok(None) => return (false, None),
        Err(error) => {
            log::warn!("Could not look for the Rune records removed libraries left: {error}");
            return (false, None);
        }
    };
    let changes = binding.token_changes();
    let (owner, read) = if left.is_empty() {
        (None, None)
    } else {
        match owner_of(binding, &left).await {
            Ok((owner, playlists)) => (owner, Some(playlists)),
            Err(error) => {
                log::warn!(
                    "Could not tell whose the Rune records a removed Apple Music library left are, trying again at the next sign-in: {error}"
                );
                return (false, None);
            }
        }
    };
    // What was read may be of two accounts; the next sign-in tells again.
    if binding.token_changes() != changes {
        log::info!("Stopped telling whose the Rune records left are: the sign-in changed");
        return (false, None);
    }
    if let Some(removed) = owner {
        let config_dir = binding.config_dir.clone();
        let moved = tokio::task::spawn_blocking(move || {
            take_over_device_records(&config_dir, library_id, removed)
        })
        .await;
        match moved {
            Ok(Ok(())) => log::info!(
                "Took over the Rune records of the removed Apple Music library {removed} for {library_id}, of the same account"
            ),
            Ok(Err(error)) => {
                log::warn!("Could not take over the Rune records of {removed}: {error}");
                return (false, None);
            }
            Err(error) => {
                log::warn!("Could not take over the Rune records of {removed}: {error}");
                return (false, None);
            }
        }
    }
    release(binding, owner.is_some()).await;
    (true, read)
}

/// Lets the binding's records be read, which are what was `taken` over or else the library's
/// own; the lists that show them reload when they were taken over.
async fn release(binding: &AppleMusicBinding, taken: bool) {
    binding.rune_playlists.release().await;
    binding.rune_mixes.release().await;
    binding.rune_likes.release().await;
    binding.records_held.store(false, Ordering::SeqCst);
    if taken {
        binding.content_changed();
    }
}

/// Keeps beside the records the ids of the account the binding signs in to that are known without
/// asking Apple: those of the song list as last read and of `playlists` when Apple's playlists
/// were read, or else those kept before, and those the records hold. Nothing is written when none
/// are known or they did not change.
pub(crate) async fn note(binding: &AppleMusicBinding, playlists: Option<&[LibraryPlaylist]>) {
    // Read in place, so the stores are read only once something asks for them.
    let folder = device_records_folder(&binding.config_dir, binding.library_id);
    let Ok(held) = tokio::task::spawn_blocking(move || AccountIds::held_in(&folder)).await else {
        return;
    };
    let kept = binding
        .rune_playlists
        .account()
        .await
        .and_then(|bytes| AccountIds::parse(&bytes));
    let songs = match binding.known_songs() {
        Some(list) => list
            .songs()
            .iter()
            .filter(|song| is_library_song(song))
            .map(|song| song.id.clone())
            .collect(),
        None => kept
            .as_ref()
            .map(|kept| kept.songs.clone())
            .unwrap_or_default(),
    };
    let playlist_ids = match playlists {
        Some(playlists) => playlists
            .iter()
            .map(|playlist| playlist.id.clone())
            .collect(),
        None => kept
            .as_ref()
            .map(|kept| kept.playlists.clone())
            .unwrap_or_default(),
    };
    let ids = AccountIds::new(
        songs.into_iter().chain(held.songs),
        playlist_ids.into_iter().chain(held.playlists),
    );
    if ids.is_empty() || kept.as_ref() == Some(&ids) {
        return;
    }
    let bytes = match serde_json::to_vec_pretty(&ids) {
        Ok(bytes) => bytes,
        Err(error) => {
            log::warn!("Could not write the ids of the Apple Music account: {error}");
            return;
        }
    };
    if let Err(error) = binding.rune_playlists.write_account(bytes).await {
        log::warn!("Could not keep the ids of the Apple Music account: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_smallest_ids_are_kept_once_and_catalog_ones_are_not() {
        let songs = (0..100).rev().map(|n| format!("i.{n:03}"));
        let ids = AccountIds::new(
            songs.chain(["i.000".to_owned(), "catalog:1".to_owned()]),
            ["pl.catalog".to_owned(), "p.b".to_owned(), "p.a".to_owned()],
        );
        assert_eq!(ids.songs.len(), KEPT_IDS);
        assert_eq!(ids.songs[0], "i.000");
        assert_eq!(ids.songs[1], "i.001");
        assert_eq!(ids.playlists, ["p.a", "p.b"]);
        assert_eq!(
            AccountIds::parse(&serde_json::to_vec(&ids).unwrap()),
            Some(ids)
        );
        assert!(AccountIds::new(Vec::new(), ["pl.catalog".to_owned()]).is_empty());
    }
}
