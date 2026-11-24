use fsio::{FsIo, FsNode};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    vec::IntoIter,
};

fn is_audio_file(entry: &FsNode) -> bool {
    if !entry.is_file {
        return false;
    }
    if let Some(ext) = entry.path.extension() {
        matches!(
            ext.to_str().unwrap_or("").to_lowercase().as_str(),
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "vorbis"
        )
    } else {
        false
    }
}

struct ScanEntries {
    files: Vec<FsNode>,
    skipped: Vec<PathBuf>,
    cues: Vec<PathBuf>,
}

fn scan_audio_files(fsio: &FsIo, root: &Path) -> Result<ScanEntries, fsio::FileIoError> {
    let walk = fsio.walk_dir(root, true)?;
    let mut candidates = Vec::new();
    let mut cues = Vec::new();
    for node in walk.nodes {
        if is_audio_file(&node) {
            candidates.push(node);
        } else if is_cue_file(&node) {
            cues.push(node.path);
        }
    }
    let albums: HashSet<_> = candidates
        .iter()
        .map(|entry| {
            crate::resolve::folder_layout(&entry.path, root)
                .0
                .to_owned()
        })
        .collect();

    // Treat files below an album's immediate SCANS directory as attachments.
    Ok(ScanEntries {
        files: candidates
            .into_iter()
            .filter(|entry| {
                !entry
                    .path
                    .ancestors()
                    .skip(1)
                    .take_while(|directory| *directory != root)
                    .any(|directory| {
                        directory
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.trim().eq_ignore_ascii_case("SCANS"))
                            && directory
                                .parent()
                                .is_some_and(|parent| albums.contains(parent))
                    })
            })
            .collect(),
        skipped: walk.skipped,
        cues,
    })
}

/// Recognize audio layout sidecars in a filesystem listing.
pub fn is_cue_file(node: &FsNode) -> bool {
    node.is_file
        && node
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("cue"))
}

pub struct AudioScanner {
    root_path: PathBuf,
    iterator: IntoIter<FsNode>,
    skipped: Vec<PathBuf>,
    cues: Vec<PathBuf>,
    ended: bool,
}

impl AudioScanner {
    pub fn new<P: AsRef<Path>>(fsio: &FsIo, path: &P) -> Result<Self, fsio::FileIoError> {
        let root = path.as_ref();
        let ScanEntries {
            files,
            skipped,
            cues,
        } = scan_audio_files(fsio, root)?;
        Ok(AudioScanner {
            root_path: root.to_path_buf(),
            iterator: files.into_iter(),
            skipped,
            cues,
            ended: false,
        })
    }

    /// Entries under the root that cannot be scanned because their path is not
    /// valid Unicode. A skipped directory stands for everything inside it.
    pub fn skipped(&self) -> &[PathBuf] {
        &self.skipped
    }

    /// Sidecar paths found during the same walk as the audio files.
    pub fn take_cue_files(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.cues)
    }

    pub fn read_files(&mut self, count: usize) -> Vec<FsNode> {
        let mut files = Vec::new();
        for _ in 0..count {
            if let Some(file) = self.iterator.next() {
                files.push(file);
            } else {
                self.ended = true;
                break;
            }
        }
        files
    }

    pub fn has_ended(&self) -> bool {
        self.ended
    }

    pub fn root_path(&self) -> &Path {
        &self.root_path
    }
}
