use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use fsio::FileStream;

pub struct FsioMediaSource {
    stream: Box<dyn FileStream>,
    size: Option<u64>,
}

impl FsioMediaSource {
    // Constructor to get the file size
    pub fn new(mut stream: Box<dyn FileStream>) -> Self {
        let size = stream.seek(SeekFrom::End(0)).ok();
        // Important: seek back to the beginning
        let _ = stream.seek(SeekFrom::Start(0));
        Self { stream, size }
    }
}

impl Read for FsioMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Seek for FsioMediaSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.stream.seek(pos)
    }
}

impl MediaSource for FsioMediaSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.size
    }
}
