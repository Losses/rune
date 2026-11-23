use std::{
    ffi::OsStr,
    fmt,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

use ::fsio::{FsIo, FsNode, OpenMode};

use media_compute::checksum::MediaCrc;

fn to_unix_path_string(path_buf: PathBuf) -> Option<String> {
    let path = path_buf.as_path();
    path.to_str().map(|path_str| path_str.replace("\\", "/"))
}

#[derive(Debug, Clone)]
pub struct FileDescription {
    pub lib_path: Option<PathBuf>,
    /// This is only for debug purpose, to show a shorter path
    pub rel_path: PathBuf,
    /// This is the path which is not normalized.
    pub raw_path: String,
    /// This is the path which is normalized by fsio!
    pub actual_path: PathBuf,
    pub file_name: String,
    pub directory: String,
    pub extension: String,
    pub file_hash: Option<String>,
    codec_information: Option<(u32, f64)>,
    pub last_modified: String,
    pub change_token: Option<String>,
    pub raw_node: FsNode,
}

impl FileDescription {
    pub fn get_crc(&mut self, fsio: &FsIo) -> Result<String> {
        let full_path = match &self.lib_path {
            Some(root_path) => root_path.join(&self.directory).join(&self.file_name),
            None => Path::new(&self.directory).join(&self.file_name),
        };

        if self.file_hash.is_none() {
            let file = fsio.open(&full_path, OpenMode::Read)?;
            let mut reader = BufReader::new(file);
            let mut buffer = vec![0; CHUNK_SIZE];
            let mut crc = MediaCrc::new();

            loop {
                let bytes_read = reader.read(&mut buffer)?;
                if bytes_read == 0 {
                    break;
                }
                crc.update(&buffer[..bytes_read]);
            }

            let result = format!("{:08x}", crc.finalize());
            self.file_hash = Some(result.clone());
            Ok(result)
        } else if let Some(result) = self.file_hash.clone() {
            Ok(result)
        } else {
            bail!("No file hash found")
        }
    }

    pub fn get_codec_information(&mut self, fsio: &FsIo) -> Result<(u32, f64)> {
        if let Some(info) = self.codec_information {
            return Ok(info);
        }
        let info = get_codec_information_from_node(fsio, &self.raw_node)?;
        self.codec_information = Some(info);
        Ok(info)
    }

    /// Reuse the stream properties collected while reading the track's tags.
    pub fn set_codec_information(&mut self, sample_rate: u32, duration: f64) {
        self.codec_information = Some((sample_rate, duration));
    }
}

pub fn get_codec_information_from_node(fsio: &FsIo, fs_node: &FsNode) -> Result<(u32, f64)> {
    let snapshot = crate::taglib::read_file(fsio, &fs_node.path, crate::taglib::ReadFields::Tags)?;
    let rate = snapshot
        .sample_rate
        .context("Audio stream has no sample rate")?;
    Ok((
        rate,
        snapshot.duration_ms.unwrap_or_default() as f64 / 1000.0,
    ))
}

const CHUNK_SIZE: usize = 1024 * 400;

pub fn describe_file(
    fsio: &FsIo,
    fs_node: &FsNode,
    lib_path: &Option<PathBuf>,
) -> Result<FileDescription> {
    let file_path = fs_node.path.clone();

    let rel_path: PathBuf = match lib_path {
        Some(x) => file_path.strip_prefix(x)?.to_path_buf(),
        None => file_path.to_path_buf(),
    };

    // Get directory
    let directory = match to_unix_path_string(
        rel_path
            .parent()
            .and_then(Path::to_str)
            .map(String::from)
            .unwrap_or_else(|| String::from(""))
            .into(),
    ) {
        Some(x) => x,
        _none => bail!("Failed to convert path to UNIX style: {:#?}", file_path),
    };

    // Get file extension
    let extension = file_path
        .extension()
        .and_then(OsStr::to_str)
        .map(String::from)
        .unwrap_or_else(|| String::from(""));

    // Get last modified time through the fs abstraction: on Android the
    // tree-relative path does not exist on the local filesystem.
    let last_modified = fsio.modified_time(&file_path)?.to_string();
    let change_token = fsio.change_token(&file_path)?;

    Ok(FileDescription {
        lib_path: lib_path.clone(),
        rel_path: rel_path.to_path_buf(),
        raw_path: fs_node.raw_path.clone(),
        actual_path: file_path.to_path_buf(),
        file_name: fs_node.filename.clone(),
        directory,
        extension,
        file_hash: None,
        codec_information: None,
        last_modified,
        change_token,
        raw_node: fs_node.clone(),
    })
}

// Helper function to format errors
impl fmt::Display for FileDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FileDescription {{ file_name: {}, directory: {}, extension: {}, last_modified: {} }}",
            self.file_name, self.directory, self.extension, self.last_modified
        )
    }
}
