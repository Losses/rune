use std::ffi::OsString;

use super::Command;

/// Helper to construct a valid `ffmpeg` command that reads from `stdin`, converts the input to the
/// provided format, and sends the output to `stdout`. This is useful for performing
/// post-processing on a media stream that needs to be converted to a different format.
#[derive(Debug)]
pub struct FfmpegConvertAudioCommand(Command);

impl From<FfmpegConvertAudioCommand> for Command {
    fn from(value: FfmpegConvertAudioCommand) -> Self {
        value.0
    }
}

impl FfmpegConvertAudioCommand {
    /// Constructs a new [`FfmpegConvertAudioCommand`].
    pub fn new<S>(format: S) -> Self
    where
        S: AsRef<str>,
    {
        // -i pipe: = pipe input from stdin
        // -map a = select all audio streams
        // -f <format> output format
        // - = output to stdout
        Self(Command::new("ffmpeg").args([
            "-i",
            "pipe:",
            "-map",
            "a",
            "-f",
            format.as_ref(),
            "-loglevel",
            "error",
            "-",
        ]))
    }

    /// Sets the path to the `ffmpeg` binary.
    #[must_use]
    pub fn ffmpeg_path<S>(mut self, path: S) -> Self
    where
        S: Into<OsString>,
    {
        self.0.program = path.into();
        self
    }

    /// Adds an argument to the [`FfmpegConvertAudioCommand`].
    #[must_use]
    pub fn arg<S>(mut self, arg: S) -> Self
    where
        S: Into<OsString>,
    {
        // '-' needs to be the last arg, so we'll insert everything right before it
        let insert_pos = self.0.args.len() - 1;
        self.0 = self.0.insert_arg(insert_pos, arg);
        self
    }

    /// Adds multiple args to the [`FfmpegConvertAudioCommand`].
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        for arg in args {
            self = self.arg(arg);
        }
        self
    }
}
