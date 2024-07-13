use std::path::Path;
use walkdir::{WalkDir, DirEntry};

fn is_audio_file(entry: &DirEntry) -> bool {
    if let Some(ext) = entry.path().extension() {
        match ext.to_str().unwrap_or("").to_lowercase().as_str() {
            "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" => true,
            _ => false,
        }
    } else {
        false
    }
}

fn scan_audio_files<P: AsRef<Path>>(path: P) -> impl Iterator<Item = DirEntry> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_audio_file(entry))
}

pub struct MetadataScanner {
    iterator: Box<dyn Iterator<Item = DirEntry>>,
    ended: bool,
}

impl MetadataScanner {
    pub fn new<P: AsRef<Path> + 'static>(path: P) -> Self {
        MetadataScanner {
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
}