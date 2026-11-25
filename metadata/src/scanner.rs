use fsio::{FsIo, FsNode};
use std::{
    path::{Path, PathBuf},
    vec::IntoIter,
};

fn is_audio_file(entry: &FsNode) -> bool {
    if let Some(ext) = entry.path.extension() {
        matches!(
            ext.to_str().unwrap_or("").to_lowercase().as_str(),
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "vorbis"
        )
    } else {
        false
    }
}

/// The audio files under `path`, and the entries left out because their path
/// is not valid Unicode.
fn scan_audio_files<P: AsRef<Path>>(
    fsio: &FsIo,
    path: &P,
) -> Result<(Vec<FsNode>, Vec<PathBuf>), fsio::FileIoError> {
    let walk = fsio.walk_dir(path.as_ref(), true)?;

    Ok((
        walk.nodes.into_iter().filter(is_audio_file).collect(),
        walk.skipped,
    ))
}

pub struct AudioScanner {
    root_path: PathBuf,
    iterator: IntoIter<FsNode>,
    skipped: Vec<PathBuf>,
    ended: bool,
}

impl AudioScanner {
    pub fn new<P: AsRef<Path>>(fsio: &FsIo, path: &P) -> Result<Self, fsio::FileIoError> {
        let (files, skipped) = scan_audio_files(fsio, path)?;
        Ok(AudioScanner {
            root_path: path.as_ref().to_path_buf(),
            iterator: files.into_iter(),
            skipped,
            ended: false,
        })
    }

    /// Entries under the root that cannot be scanned because their path is not
    /// valid Unicode. A skipped directory stands for everything inside it.
    pub fn skipped(&self) -> &[PathBuf] {
        &self.skipped
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
