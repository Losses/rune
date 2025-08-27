use std::{
    ffi::OsStr,
    fmt,
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use anyhow::{Context, Result, bail};
use symphonia::core::codecs::CODEC_TYPE_NULL;

use ::analysis::utils::audio_metadata_reader::{get_codec_information, get_format};
use ::fsio::{FsIo, FsNode};

use crate::crc::media_crc32;

fn to_unix_path_string(path_buf: PathBuf) -> Option<String> {
    let path = path_buf.as_path();
    path.to_str().map(|path_str| path_str.replace("\\", "/"))
}

#[derive(Debug)]
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
    pub last_modified: String,
    pub raw_node: FsNode,
}

impl FileDescription {
    pub fn get_crc(&mut self, fsio: &FsIo) -> Result<String> {
        let full_path = match &self.lib_path {
            Some(root_path) => root_path.join(&self.directory).join(&self.file_name),
            None => Path::new(&self.directory).join(&self.file_name),
        };

        if self.file_hash.is_none() {
            let file = fsio.open(&full_path, "r")?;
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

            let result = format!("{crc:08x}");
            self.file_hash = Some(result.clone());
            Ok(result)
        } else if let Some(result) = self.file_hash.clone() {
            Ok(result)
        } else {
            bail!("No file hash found")
        }
    }

    pub fn get_codec_information(&mut self, fsio: &FsIo) -> Result<(u32, f64)> {
        let codec_information = get_codec_information_from_node(fsio, &self.raw_node)?;

        Ok(codec_information)
    }
}

pub fn get_codec_information_from_node(fsio: &FsIo, fs_node: &FsNode) -> Result<(u32, f64)> {
    let full_path = match fs_node.path.to_str() {
        Some(full_path) => full_path,
        _none => bail!("Failed to convert file path while getting codec information"),
    };

    let format = get_format(fsio, full_path)
        .with_context(|| format!("No supported format found: {full_path}"))?;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .with_context(|| "No supported audio tracks")?;

    let codec_information = get_codec_information(track)
        .with_context(|| format!("Failed to get codec information: {full_path}"))?;

    Ok(codec_information)
}

const CHUNK_SIZE: usize = 1024 * 400;

pub fn describe_file(fs_node: &FsNode, lib_path: &Option<PathBuf>) -> Result<FileDescription> {
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

    // Get last modified time
    let metadata = file_path.metadata()?;
    let last_modified = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_secs();
    let last_modified = format!("{last_modified}");

    Ok(FileDescription {
        lib_path: lib_path.clone(),
        rel_path: rel_path.to_path_buf(),
        raw_path: fs_node.raw_path.clone(),
        actual_path: file_path.to_path_buf(),
        file_name: fs_node.filename.clone(),
        directory,
        extension,
        file_hash: None,
        last_modified,
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
