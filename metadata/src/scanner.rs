use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

fn is_audio_file(entry: &DirEntry) -> bool {
    if let Some(ext) = entry.path().extension() {
        matches!(
            ext.to_str().unwrap_or("").to_lowercase().as_str(),
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "opus" | "vorbis"
        )
    } else {
        false
    }
}

fn scan_audio_files<P: AsRef<Path>>(path: &P) -> impl Iterator<Item = DirEntry> + Send {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_audio_file(entry))
}

pub struct AudioScanner<'a> {
    root_path: PathBuf,
    iterator: Box<dyn Iterator<Item = DirEntry> + Send + 'a>,
    ended: bool,
}

impl<'a> AudioScanner<'a> {
    pub fn new<P: AsRef<Path> + Send + 'a>(path: &'a P) -> Self {
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
