use log::error;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

use crate::reader::get_metadata;

fn is_audio_file(entry: &DirEntry) -> bool {
    if let Some(ext) = entry.path().extension() {
        matches!(
            ext.to_str().unwrap_or("").to_lowercase().as_str(),
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a"
        )
    } else {
        false
    }
}

fn scan_audio_files<P: AsRef<Path>>(path: &P) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_audio_file(entry))
}

pub struct AudioScanner<'a> {
    root_path: PathBuf,
    iterator: Box<dyn Iterator<Item = DirEntry> + 'a>,
    ended: bool,
}

impl<'a> AudioScanner<'a> {
    pub fn new<P: AsRef<Path>>(path: &'a P) -> Self {
        AudioScanner {
            root_path: path.as_ref().to_path_buf(),
            iterator: Box::new(scan_audio_files(path)),
            ended: false,
        }
    }

    pub fn read_files(&mut self, count: usize) -> Vec<DirEntry> {
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

pub struct FileMetadata {
    pub path: PathBuf,
    pub metadata: Vec<(String, String)>,
}

pub struct MetadataScanner<'a> {
    audio_scanner: AudioScanner<'a>,
}

impl<'a> MetadataScanner<'a> {
    pub fn new<P: AsRef<Path>>(path: &'a P) -> Self {
        MetadataScanner {
            audio_scanner: AudioScanner::new(path),
        }
    }

    pub fn read_metadata(&mut self, count: usize) -> Vec<FileMetadata> {
        let mut metadata_list = Vec::new();
        let files = self.audio_scanner.read_files(count);
        for file in files {
            let abs_path = file.path().to_path_buf();
            let rel_path = abs_path
                .strip_prefix(self.audio_scanner.root_path())
                .unwrap()
                .to_path_buf();
            match get_metadata(abs_path.to_str().unwrap(), None) {
                Ok(metadata) => metadata_list.push(FileMetadata {
                    path: rel_path,
                    metadata,
                }),
                Err(err) => {
                    error!("Error reading metadata for {}: {}", abs_path.display(), err);
                    // Continue to the next file instead of returning an empty list
                }
            }
        }
        metadata_list
    }

    pub fn has_ended(&self) -> bool {
        self.audio_scanner.has_ended()
    }
}
