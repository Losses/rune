//! Receivers for Rune's own file dialog. Each request runs on its own task,
//! so a slow drive or a slow thumbnailer never holds up the next folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Error, Result};
use fsio::{DirEntryDetails, EntryKind, FsIo};
use fsio_browse::{
    BrowseError, KnownFolder, Location, PathTarget, Place, PlaceName, Root, Thumbnail,
    ThumbnailSize, Thumbnailer, Volume, VolumeKind, create_folder, home_dir, inspect,
    list_directory, rename_entry, resolve_typed_path, system_places,
};
use rinf::{DartSignal, RustSignalBinary};
use tokio::sync::Semaphore;
use tokio::task;

use crate::macros::{StandaloneRequestReceiver, answer_node_request};
use crate::messages::*;
use crate::request_error::ExpectedRequestError;
use crate::utils::RinfRustSignal;

/// Thumbnails drawn at the same time. A platform thumbnailer can take a
/// while on a large video; more at once would only slow every one of them.
const THUMBNAIL_WORKERS: usize = 4;

/// The edge range a thumbnail request may ask for, in pixels.
const THUMBNAIL_EDGES: (u32, u32) = (16, 1024);

fn emit(signal: &dyn RinfRustSignal) {
    signal.send_to_dart();
}

fn dialog_error(error: BrowseError) -> Error {
    let failure = match error {
        BrowseError::NotFound => FileDialogFailure::NotFound,
        BrowseError::NotADirectory => FileDialogFailure::NotAFolder,
        BrowseError::PermissionDenied => FileDialogFailure::PermissionDenied,
        BrowseError::AlreadyExists => FileDialogFailure::AlreadyExists,
        BrowseError::InvalidName => FileDialogFailure::InvalidName,
        BrowseError::Unavailable(detail) => {
            log::warn!("File dialog storage unavailable: {detail}");
            FileDialogFailure::Unavailable
        }
    };
    ExpectedRequestError::FileDialog { failure }.into()
}

fn location(message: &FileDialogLocation) -> Location {
    Location {
        root: match &message.root {
            FileDialogRoot::FileSystem => Root::FileSystem,
            FileDialogRoot::DocumentTree { uri } => Root::DocumentTree { uri: uri.clone() },
        },
        path: PathBuf::from(&message.path),
    }
}

fn location_message(root: &Root, path: &Path) -> FileDialogLocation {
    FileDialogLocation {
        root: match root {
            Root::FileSystem => FileDialogRoot::FileSystem,
            Root::DocumentTree { uri } => FileDialogRoot::DocumentTree { uri: uri.clone() },
        },
        path: path.to_string_lossy().into_owned(),
    }
}

fn open(root: &Root) -> Result<FsIo> {
    root.open().map_err(dialog_error)
}

fn place_message(place: Place) -> FileDialogPlace {
    FileDialogPlace {
        location: location_message(&place.location.root, &place.location.path),
        name: match place.name {
            PlaceName::Known(folder) => FileDialogPlaceName::Known {
                folder: match folder {
                    KnownFolder::Home => FileDialogKnownFolder::Home,
                    KnownFolder::Desktop => FileDialogKnownFolder::Desktop,
                    KnownFolder::Documents => FileDialogKnownFolder::Documents,
                    KnownFolder::Downloads => FileDialogKnownFolder::Downloads,
                    KnownFolder::Music => FileDialogKnownFolder::Music,
                    KnownFolder::Pictures => FileDialogKnownFolder::Pictures,
                    KnownFolder::Videos => FileDialogKnownFolder::Videos,
                },
            },
            PlaceName::Custom(name) => FileDialogPlaceName::Custom { name },
        },
    }
}

fn volume_message(volume: Volume) -> FileDialogVolume {
    FileDialogVolume {
        location: location_message(&volume.location.root, &volume.location.path),
        label: volume.label,
        kind: match volume.kind {
            VolumeKind::System => FileDialogVolumeKind::System,
            VolumeKind::Fixed => FileDialogVolumeKind::Fixed,
            VolumeKind::Removable => FileDialogVolumeKind::Removable,
            VolumeKind::Optical => FileDialogVolumeKind::Optical,
            VolumeKind::Network => FileDialogVolumeKind::Network,
        },
        space: volume.space.map(|space| FileDialogVolumeSpace {
            total: space.total,
            available: space.available,
        }),
    }
}

fn entry_message(root: &Root, entry: DirEntryDetails) -> FileDialogEntry {
    FileDialogEntry {
        location: location_message(root, &entry.path),
        name: entry.name,
        kind: match entry.kind {
            EntryKind::Directory => FileDialogEntryKind::Folder,
            EntryKind::File => FileDialogEntryKind::File,
            EntryKind::Other => FileDialogEntryKind::Other,
        },
        size: entry.size,
        modified: entry.modified,
        hidden: entry.hidden,
    }
}

async fn list_places(
    _: &ListFileDialogPlacesRequest,
) -> Result<Option<ListFileDialogPlacesResponse>> {
    let places = task::spawn_blocking(system_places).await?;
    Ok(Some(ListFileDialogPlacesResponse {
        pinned: places.pinned.into_iter().map(place_message).collect(),
        folders: places.folders.into_iter().map(place_message).collect(),
        volumes: places.volumes.into_iter().map(volume_message).collect(),
        call_id: None,
    }))
}

async fn resolve_path(
    request: &ResolveFileDialogPathRequest,
) -> Result<Option<ResolveFileDialogPathResponse>> {
    let current = location(&request.current);
    let path = match current.root {
        Root::FileSystem => {
            resolve_typed_path(&request.typed, &current.path, home_dir().as_deref())
        }
        // A document tree has no home, and a leading slash means its own top.
        Root::DocumentTree { .. } => {
            let typed = request.typed.trim().trim_start_matches('/');
            resolve_typed_path(typed, &current.path, None)
        }
    };
    let fs = open(&current.root)?;
    let target = inspect(&fs, &path).await.map_err(dialog_error)?;
    Ok(Some(ResolveFileDialogPathResponse {
        location: location_message(&current.root, &path),
        target: match target {
            PathTarget::Folder => FileDialogPathTarget::Folder,
            PathTarget::File => FileDialogPathTarget::File,
            PathTarget::Missing => FileDialogPathTarget::Missing,
            PathTarget::Unreachable => FileDialogPathTarget::Unreachable,
        },
        call_id: None,
    }))
}

async fn list_folder(
    request: &ListFileDialogFolderRequest,
) -> Result<Option<ListFileDialogFolderResponse>> {
    let folder = location(&request.location);
    let fs = open(&folder.root)?;
    let entries = list_directory(&fs, &folder.path)
        .await
        .map_err(dialog_error)?;
    Ok(Some(ListFileDialogFolderResponse {
        entries: entries
            .into_iter()
            .map(|entry| entry_message(&folder.root, entry))
            .collect(),
        call_id: None,
    }))
}

async fn create_dialog_folder(
    request: &CreateFileDialogFolderRequest,
) -> Result<Option<CreateFileDialogFolderResponse>> {
    let parent = location(&request.parent);
    let fs = open(&parent.root)?;
    let created = create_folder(&fs, &parent.path, &request.name)
        .await
        .map_err(dialog_error)?;
    Ok(Some(CreateFileDialogFolderResponse {
        location: location_message(&parent.root, &created),
        call_id: None,
    }))
}

async fn rename_dialog_entry(
    request: &RenameFileDialogEntryRequest,
) -> Result<Option<RenameFileDialogEntryResponse>> {
    let item = location(&request.location);
    let fs = open(&item.root)?;
    let renamed = rename_entry(&fs, &item.path, &request.name)
        .await
        .map_err(dialog_error)?;
    Ok(Some(RenameFileDialogEntryResponse {
        location: location_message(&item.root, &renamed),
        call_id: None,
    }))
}

/// What the thumbnail receiver keeps across requests: the platform's
/// thumbnailer and the limit on how many it draws at once.
#[derive(Clone)]
struct ThumbnailWorkers {
    thumbnailer: Thumbnailer,
    permits: Arc<Semaphore>,
}

async fn answer_thumbnail(request: FetchFileDialogThumbnailRequest, workers: ThumbnailWorkers) {
    let result = async {
        let _permit = workers.permits.acquire().await?;
        let (smallest, largest) = THUMBNAIL_EDGES;
        let size = ThumbnailSize(request.size.clamp(smallest, largest));
        let item = location(&request.location);
        let fs = open(&item.root)?;
        workers.thumbnailer.item(&fs, &item, size).await
    }
    .await;
    match result {
        Ok(drawn) => {
            let (thumbnail, pixels) = match drawn {
                Some(Thumbnail {
                    width,
                    height,
                    rgba,
                }) => (FileDialogThumbnail::Drawn { width, height }, rgba),
                None => (FileDialogThumbnail::NotDrawn, Vec::new()),
            };
            FileDialogThumbnailResponse {
                thumbnail,
                call_id: request.call_id,
            }
            .send_signal_to_dart(pixels);
        }
        Err(error) => {
            answer_node_request::<FetchFileDialogThumbnailRequest>(&request, Err(error), &emit)
        }
    }
}

/// Answer every request of a type as it arrives, each on a task of its own.
macro_rules! serve_concurrently {
    ($request:ty, $handler:ident) => {
        impl StandaloneRequestReceiver for $request {
            fn start() {
                tokio::spawn(async {
                    let receiver = <$request>::get_dart_signal_receiver();
                    while let Some(signal) = receiver.recv().await {
                        tokio::spawn(async move {
                            let request = signal.message;
                            let result = $handler(&request).await;
                            answer_node_request(&request, result, &emit);
                        });
                    }
                });
            }
        }
    };
}

serve_concurrently!(ListFileDialogPlacesRequest, list_places);
serve_concurrently!(ResolveFileDialogPathRequest, resolve_path);
serve_concurrently!(ListFileDialogFolderRequest, list_folder);
serve_concurrently!(CreateFileDialogFolderRequest, create_dialog_folder);
serve_concurrently!(RenameFileDialogEntryRequest, rename_dialog_entry);

impl StandaloneRequestReceiver for FetchFileDialogThumbnailRequest {
    fn start() {
        tokio::spawn(async {
            let receiver = FetchFileDialogThumbnailRequest::get_dart_signal_receiver();
            let workers = ThumbnailWorkers {
                thumbnailer: Thumbnailer::new(),
                permits: Arc::new(Semaphore::new(THUMBNAIL_WORKERS)),
            };
            while let Some(signal) = receiver.recv().await {
                tokio::spawn(answer_thumbnail(signal.message, workers.clone()));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::request_error::request_failure_cause;

    fn file_system(path: &Path) -> FileDialogLocation {
        location_message(&Root::FileSystem, path)
    }

    fn failure(error: &Error) -> Option<FileDialogFailure> {
        match request_failure_cause(error) {
            Some(RequestFailureCause::FileDialogFailed { failure }) => Some(failure),
            _ => None,
        }
    }

    #[tokio::test]
    async fn folders_list_and_report_typed_failures() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("track.flac"), b"flac").unwrap();
        fs::create_dir(dir.path().join("Albums")).unwrap();

        let listed = list_folder(&ListFileDialogFolderRequest {
            location: file_system(dir.path()),
            call_id: None,
        })
        .await
        .unwrap()
        .unwrap();
        let mut names: Vec<_> = listed
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.kind, entry.location.path.clone()))
            .collect();
        names.sort_by(|a, b| a.0.cmp(b.0));
        assert_eq!(
            names,
            [
                (
                    "Albums",
                    FileDialogEntryKind::Folder,
                    dir.path().join("Albums").to_string_lossy().into_owned()
                ),
                (
                    "track.flac",
                    FileDialogEntryKind::File,
                    dir.path().join("track.flac").to_string_lossy().into_owned()
                ),
            ]
        );

        for (path, expected) in [
            (dir.path().join("missing"), FileDialogFailure::NotFound),
            (dir.path().join("track.flac"), FileDialogFailure::NotAFolder),
        ] {
            let error = list_folder(&ListFileDialogFolderRequest {
                location: file_system(&path),
                call_id: None,
            })
            .await
            .err()
            .unwrap();
            assert_eq!(failure(&error), Some(expected), "{path:?}");
        }
        let document_tree = list_folder(&ListFileDialogFolderRequest {
            location: FileDialogLocation {
                root: FileDialogRoot::DocumentTree {
                    uri: "content://tree".into(),
                },
                path: String::new(),
            },
            call_id: None,
        })
        .await
        .err()
        .unwrap();
        if cfg!(not(target_os = "android")) {
            assert_eq!(
                failure(&document_tree),
                Some(FileDialogFailure::Unavailable)
            );
        }
    }

    #[tokio::test]
    async fn typed_paths_resolve_against_the_current_folder() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Albums")).unwrap();
        let resolve = |typed: &str| {
            let request = ResolveFileDialogPathRequest {
                typed: typed.to_owned(),
                current: file_system(dir.path()),
                call_id: None,
            };
            async move { resolve_path(&request).await.unwrap().unwrap() }
        };

        let albums = resolve(" Albums ").await;
        assert_eq!(albums.target, FileDialogPathTarget::Folder);
        assert_eq!(albums.location, file_system(&dir.path().join("Albums")));
        let new_file = resolve("Albums/../cover.png").await;
        assert_eq!(new_file.target, FileDialogPathTarget::Missing);
        assert_eq!(
            new_file.location,
            file_system(&dir.path().join("cover.png"))
        );
        assert_eq!(
            resolve("nowhere/cover.png").await.target,
            FileDialogPathTarget::Unreachable
        );
    }

    #[tokio::test]
    async fn folders_are_created_and_renamed_with_conflicts_reported() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_dialog_folder(&CreateFileDialogFolderRequest {
            parent: file_system(dir.path()),
            name: "New folder".into(),
            call_id: None,
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            created.location,
            file_system(&dir.path().join("New folder"))
        );

        let conflict = create_dialog_folder(&CreateFileDialogFolderRequest {
            parent: file_system(dir.path()),
            name: "New folder".into(),
            call_id: None,
        })
        .await
        .err()
        .unwrap();
        assert_eq!(failure(&conflict), Some(FileDialogFailure::AlreadyExists));

        let renamed = rename_dialog_entry(&RenameFileDialogEntryRequest {
            location: created.location,
            name: "Live".into(),
            call_id: None,
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(renamed.location, file_system(&dir.path().join("Live")));
        assert!(dir.path().join("Live").is_dir());
    }
}
