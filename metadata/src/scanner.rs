use fsio::{FsIo, FsNode};
use std::path::{Path, PathBuf};

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

fn scan_audio_files<P: AsRef<Path>>(
    fsio: &FsIo,
    path: &P,
) -> Result<impl Iterator<Item = FsNode> + Send, fsio::FileIoError> {
    let files = fsio.walk_dir(path.as_ref(), true)?;

    Ok(files.into_iter().filter(is_audio_file))
}

pub struct AudioScanner<'a> {
    root_path: PathBuf,
    iterator: Box<dyn Iterator<Item = FsNode> + Send + 'a>,
    ended: bool,
}

impl<'a> AudioScanner<'a> {
    pub fn new<P: AsRef<Path> + Send + 'a>(
        fsio: &'a FsIo,
        path: &'a P,
    ) -> Result<Self, fsio::FileIoError> {
        let iterator = scan_audio_files(fsio, path)?;
        Ok(AudioScanner {
            root_path: path.as_ref().to_path_buf(),
            iterator: Box::new(iterator),
            ended: false,
        })
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
