use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::crc::media_crc32;

#[derive(Debug)]
pub struct FileDescription {
    pub file_name: String,
    pub directory: String,
    pub extension: String,
    pub file_hash: String,
    pub last_modified: String,
}

const CHUNK_SIZE: usize = 1024 * 400;

pub fn describe_file(
    rel_path: &PathBuf,
    root_path: &PathBuf,
) -> Result<FileDescription, Box<dyn std::error::Error>> {
    let full_path = root_path.join(&rel_path);

    // Get file name
    let file_name = full_path
        .file_name()
        .and_then(OsStr::to_str)
        .map(String::from)
        .ok_or("Failed to get file name")?;

    // Get directory
    let directory = rel_path
        .parent()
        .and_then(Path::to_str)
        .map(String::from)
        .unwrap_or_else(|| String::from(""));

    // Get file extension
    let extension = full_path
        .extension()
        .and_then(OsStr::to_str)
        .map(String::from)
        .unwrap_or_else(|| String::from(""));

    // Get last modified time
    let metadata = full_path.metadata()?;
    let last_modified = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
    let last_modified = format!("{}", last_modified);

    // Compute file hash
    let file = File::open(&full_path)?;
    let mut reader = BufReader::new(file);
    let mut buffer = vec![0; CHUNK_SIZE];
    let mut crc: u32 = 0;

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        crc = media_crc32(&buffer, crc, 0, bytes_read);
    }

    let file_hash = format!("{:08x}", crc);

    Ok(FileDescription {
        file_name,
        directory,
        extension,
        file_hash,
        last_modified,
    })
}

// Helper function to format errors
impl fmt::Display for FileDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileDescription {{ file_name: {}, directory: {}, extension: {}, file_hash: {}, last_modified: {} }}",
               self.file_name, self.directory, self.extension, self.file_hash, self.last_modified)
    }
}
