//! Requests behind Rune's own file dialog. They read this device's storage,
//! so only its own interface may send them.

use rinf::{DartSignal, RustSignal, RustSignalBinary, SignalPiece};
use serde::{Deserialize, Serialize};

/// The storage a dialog location lives in.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileDialogRoot {
    /// The device's own file system, addressed by absolute paths.
    FileSystem,
    /// A folder granted through Android's document picker, addressed by
    /// paths relative to it.
    DocumentTree { uri: String },
}

/// A folder or file the dialog can show.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileDialogLocation {
    pub root: FileDialogRoot,
    pub path: String,
}

/// Why the dialog could not do what was asked, each with its own message.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogFailure {
    NotFound,
    NotAFolder,
    PermissionDenied,
    /// The storage did not answer: a drive was removed, a server is gone.
    Unavailable,
    /// Something with the new name is already there.
    AlreadyExists,
    /// The name is empty or holds characters the platform does not allow.
    InvalidName,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogEntryKind {
    Folder,
    File,
    /// A link whose target is gone, a socket, a device.
    Other,
}

/// One item of a folder.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct FileDialogEntry {
    pub name: String,
    pub location: FileDialogLocation,
    pub kind: FileDialogEntryKind,
    /// Bytes in a file; zero for anything else.
    pub size: u64,
    /// Seconds since the UNIX epoch of the last change, when the storage
    /// reports it.
    pub modified: Option<u64>,
    pub hidden: bool,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogKnownFolder {
    Home,
    Desktop,
    Documents,
    Downloads,
    Music,
    Pictures,
    Videos,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub enum FileDialogPlaceName {
    /// A well-known folder, which the dialog names in Rune's language.
    Known { folder: FileDialogKnownFolder },
    /// The name the system gives a pinned folder.
    Custom { name: String },
}

/// A folder in the navigation pane.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct FileDialogPlace {
    pub location: FileDialogLocation,
    pub name: FileDialogPlaceName,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogVolumeKind {
    System,
    Fixed,
    Removable,
    Optical,
    Network,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDialogVolumeSpace {
    pub total: u64,
    pub available: u64,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Debug, PartialEq, Eq)]
pub struct FileDialogVolume {
    pub location: FileDialogLocation,
    pub label: Option<String>,
    pub kind: FileDialogVolumeKind,
    pub space: Option<FileDialogVolumeSpace>,
}

/// The navigation pane's places as this device's system knows them.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListFileDialogPlacesRequest {
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListFileDialogPlacesResponse {
    /// Folders pinned in the system's file browser, in its order.
    pub pinned: Vec<FileDialogPlace>,
    /// The account's well-known folders that exist.
    pub folders: Vec<FileDialogPlace>,
    pub volumes: Vec<FileDialogVolume>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// What a person typed into the address bar or the file name box, resolved
/// against the folder the dialog shows.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct ResolveFileDialogPathRequest {
    pub typed: String,
    pub current: FileDialogLocation,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// What a resolved path names.
#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogPathTarget {
    Folder,
    File,
    /// Nothing, in a folder that exists: where a new file can be saved.
    Missing,
    /// Nothing, and its folder does not exist either.
    Unreachable,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ResolveFileDialogPathResponse {
    pub location: FileDialogLocation,
    pub target: FileDialogPathTarget,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// The items of one folder.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct ListFileDialogFolderRequest {
    pub location: FileDialogLocation,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct ListFileDialogFolderResponse {
    pub entries: Vec<FileDialogEntry>,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// A thumbnail of one item, scaled to fit a square of `size` pixels. The
/// reply is a [`FileDialogThumbnailResponse`] carrying the pixels as binary.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct FetchFileDialogThumbnailRequest {
    pub location: FileDialogLocation,
    pub size: u32,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Deserialize, Serialize, SignalPiece, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogThumbnail {
    /// The binary part holds `width * height` straight RGBA8 pixels, top row
    /// first.
    Drawn { width: u32, height: u32 },
    /// Neither the platform nor Rune draws one for this item; it keeps its
    /// type icon.
    NotDrawn,
}

#[derive(Serialize, Deserialize, RustSignalBinary)]
pub struct FileDialogThumbnailResponse {
    pub thumbnail: FileDialogThumbnail,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// A new folder named `name` inside `parent`.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct CreateFileDialogFolderRequest {
    pub parent: FileDialogLocation,
    pub name: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct CreateFileDialogFolderResponse {
    pub location: FileDialogLocation,
    #[serde(default)]
    pub call_id: Option<u64>,
}

/// A new name for a file or folder, within the folder it is in.
#[derive(Serialize, Deserialize, DartSignal)]
pub struct RenameFileDialogEntryRequest {
    pub location: FileDialogLocation,
    pub name: String,
    #[serde(default)]
    pub call_id: Option<u64>,
}

#[derive(Serialize, Deserialize, RustSignal)]
pub struct RenameFileDialogEntryResponse {
    pub location: FileDialogLocation,
    #[serde(default)]
    pub call_id: Option<u64>,
}
