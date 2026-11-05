use std::ffi::OsString;
use std::io;

use super::{Command, SpawnCommand, SpawnedCommand};

/// Helper to construct a valid `yt-dlp` command that outputs to `stdout`.
#[derive(Debug, Clone)]
pub struct YtDlpCommand {
    url: String,
    cmd_name: OsString,
    extract_audio: bool,
    format: Option<String>,
}

impl From<YtDlpCommand> for Command {
    fn from(value: YtDlpCommand) -> Self {
        value.into_command()
    }
}

impl SpawnCommand for YtDlpCommand {
    fn spawn(self) -> io::Result<SpawnedCommand> {
        self.into_command().spawn()
    }
}

impl YtDlpCommand {
    /// Creates a mew [`YtDlpCommand`].
    pub fn new<S>(url: S) -> Self
    where
        S: Into<String>,
    {
        Self {
            url: url.into(),
            cmd_name: "yt-dlp".into(),
            extract_audio: false,
            format: None,
        }
    }

    /// Creates a [`Command`] from the given parameters.
    #[must_use]
    pub fn into_command(self) -> Command {
        let mut cmd = Command::new(&self.cmd_name).args([
            &self.url,
            "--quiet",
            // don't store .part files because they can conflict with each other during concurrent
            // downloads
            "--no-part",
            // don't reuse previous fragments since they could be from a different file
            "--no-continue",
            "-o",
            "-",
        ]);
        if self.extract_audio {
            cmd = cmd.arg("-x");
        }
        if let Some(format) = &self.format {
            cmd = cmd.args(["-f", format]);
        }
        cmd
    }

    /// Extract audio from the given URL.
    #[must_use]
    pub fn extract_audio(mut self, extract_audio: bool) -> Self {
        self.extract_audio = extract_audio;
        self
    }

    /// Extract content using the provided format.
    /// An error will be thrown when running the command if the format is not available.
    #[must_use]
    pub fn format<S>(mut self, format: S) -> Self
    where
        S: Into<String>,
    {
        self.format = Some(format.into());
        self
    }

    /// Sets the path to the `yt-dlp` binary.
    #[must_use]
    pub fn yt_dlp_path<S>(mut self, path: S) -> Self
    where
        S: Into<OsString>,
    {
        self.cmd_name = path.into();
        self
    }
}
