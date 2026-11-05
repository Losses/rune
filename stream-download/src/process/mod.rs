//! A [`SourceStream`] adapter for reading data from external processes. This is a wrapper on top of
//! the [`async_read`][crate::async_read] module.
//!
//! Due to limitations with reading `stdout` and `stderr` simultaneously while piping large amounts
//! of data to `stdin` (see [`std::process::Stdio::piped`]), the child process' `stderr` handle
//! will be redirected to a temporary file rather than piped directly into the program unless
//! explicitly overridden with [`Command::stderr_handle`].
//!
//! This implementation makes the assumption that any commands used will output binary data to
//! `stdout` and any error messages will be logged to `stderr`. You probably want to disable
//! [`Settings::cancel_on_drop`][crate::Settings::cancel_on_drop] when using this module in order
//! to ensure all error messages are flushed before the process is stopped.
//!
//! Helpers for interacting with `yt-dlp` for extracting media from specific sites and `ffmpeg` for
//! post-processing are also included.

use std::convert::Infallible;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::mem;
use std::pin::Pin;
use std::process::{ChildStdout, Stdio};
use std::task::Poll;

use bytes::Bytes;
pub use command_builder::*;
pub use ffmpeg::*;
use futures_util::Stream;
use tempfile::NamedTempFile;
use tracing::{debug, error, warn};
pub use yt_dlp::*;

use crate::WrapIoResult;
use crate::async_read::AsyncReadStream;
use crate::source::{SourceStream, StreamOutcome};

mod command_builder;
mod ffmpeg;
mod yt_dlp;

/// Trait used by objects that can spawn a command suitable for use with a [`ProcessStream`].
pub trait SpawnCommand {
    /// Spawns the command with the stdout and stderr handles configured appropriately.
    fn spawn(self) -> io::Result<SpawnedCommand>;
}

/// A simplified representation of an OS command that can be used to create a [`SpawnedCommand`].
#[derive(Debug)]
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    stderr_handle: Option<Stdio>,
}

impl Command {
    /// Creates a new [`Command`].
    pub fn new<S>(program: S) -> Self
    where
        S: Into<OsString>,
    {
        Self {
            program: program.into(),
            args: Vec::new(),
            stderr_handle: None,
        }
    }

    /// Adds multiple arguments to the [`Command`].
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

    /// Adds a single argument to the [`Command`].
    #[must_use]
    pub fn arg<S>(mut self, arg: S) -> Self
    where
        S: Into<OsString>,
    {
        self.args.push(arg.into());
        self
    }

    /// Inserts an argument at the given position.
    pub fn insert_arg<S>(mut self, index: usize, arg: S) -> Self
    where
        S: Into<OsString>,
    {
        self.args.insert(index, arg.into());
        self
    }

    /// Sets the [`Stdio`] handle for the `stderr` stream of the command. This is used for error
    /// reporting in case of a failure.
    ///
    /// Setting this value will override the default behavior of creating a temporary file to store
    /// the output. See [the module documentation][crate::process] for why this is necessary.
    #[must_use]
    pub fn stderr_handle<S>(mut self, stderr_handle: S) -> Self
    where
        S: Into<Stdio>,
    {
        self.stderr_handle = Some(stderr_handle.into());
        self
    }
}

impl SpawnCommand for Command {
    fn spawn(self) -> io::Result<SpawnedCommand> {
        SpawnedCommand::new(self, None, Vec::new())
    }
}

/// A representation of a [`tokio::process::Command`] that's been spawned.
#[derive(Debug)]
pub struct SpawnedCommand {
    child_handle: tokio::process::Child,
    stderr_files: Vec<NamedTempFile>,
}

impl SpawnedCommand {
    fn new(
        command: Command,
        prev_out: Option<ChildStdout>,
        mut stderr_files: Vec<NamedTempFile>,
    ) -> Result<Self, io::Error> {
        let mut tokio_command = tokio::process::Command::new(command.program);

        tokio_command.args(command.args).stdout(Stdio::piped());
        if let Some(handle) = command.stderr_handle {
            tokio_command.stderr(handle);
        } else {
            let (stdio, stderr_file) = stdio_to_tmp_file()?;
            tokio_command.stderr(stdio);
            stderr_files.push(stderr_file);
        }

        if let Some(prev_out) = prev_out {
            tokio_command.stdin(prev_out);
        }

        tokio_command.kill_on_drop(true);
        #[cfg(target_os = "windows")]
        {
            // CREATE_NO_WINDOW
            tokio_command.creation_flags(0x08000000);
        }

        Ok(Self {
            child_handle: tokio_command.spawn().wrap_err("error spawning process")?,
            stderr_files,
        })
    }
}

fn stdio_to_tmp_file() -> io::Result<(Stdio, NamedTempFile)> {
    let stderr_file = tempfile::NamedTempFile::new().wrap_err("error creating temp file")?;
    let stdio = Stdio::from(
        stderr_file
            .as_file()
            .try_clone()
            .wrap_err("error cloning file")?,
    );
    Ok((stdio, stderr_file))
}

/// Parameters for creating a [`ProcessStream`].
#[derive(Debug)]
pub struct ProcessStreamParams {
    content_length: Option<u64>,
    command: SpawnedCommand,
}

impl ProcessStreamParams {
    /// Creates a new [`ProcessStreamParams`].
    pub fn new<C>(command: C) -> io::Result<Self>
    where
        C: SpawnCommand,
    {
        Ok(Self {
            command: command.spawn()?,
            content_length: None,
        })
    }

    /// Set the content length of the stream.
    #[must_use]
    pub fn content_length<L>(self, content_length: L) -> Self
    where
        L: Into<Option<u64>>,
    {
        Self {
            content_length: content_length.into(),
            ..self
        }
    }
}

/// A [`SourceStream`] implementation that asynchronously reads the `stdout` byte stream from a
/// [`tokio::process::Command`].
#[derive(Debug)]
pub struct ProcessStream {
    stream: AsyncReadStream<tokio::process::ChildStdout>,
    child_handle: tokio::process::Child,
    stderr_files: Vec<NamedTempFile>,
}

impl ProcessStream {
    fn check_stderr_files(&mut self) {
        for file in &mut self.stderr_files {
            let _ = file
                .flush()
                .inspect_err(|e| error!("error flushing file: {e:?}"));
            // Need to reopen the file to access the contents since it was written to from an
            // external process
            if let Ok(mut file_handle) = file
                .reopen()
                .inspect_err(|e| error!("error opening file: {e:?}"))
            {
                let mut buf = String::new();
                let _ = file_handle
                    .read_to_string(&mut buf)
                    .inspect_err(|e| error!("error reading file: {e:?}"));
                warn!("stderr from child process: {buf}");
            }
        }
    }

    fn close_stderr_files(&mut self) {
        for file in mem::take(&mut self.stderr_files) {
            let _ = file
                .close()
                .inspect_err(|e| warn!("error closing file: {e:?}"));
        }
    }
}

impl SourceStream for ProcessStream {
    type Params = ProcessStreamParams;
    type StreamCreationError = Infallible;

    async fn create(params: Self::Params) -> Result<Self, Self::StreamCreationError> {
        let ProcessStreamParams {
            content_length,
            mut command,
        } = params;

        Ok(Self {
            stream: AsyncReadStream::new(
                command.child_handle.stdout.take().expect("stdout missing"),
                content_length,
            ),
            child_handle: command.child_handle,
            stderr_files: command.stderr_files,
        })
    }

    fn content_length(&self) -> Option<u64> {
        self.stream.content_length()
    }

    fn supports_seek(&self) -> bool {
        self.stream.supports_seek()
    }

    async fn seek_range(&mut self, start: u64, end: Option<u64>) -> io::Result<()> {
        self.stream.seek_range(start, end).await
    }

    async fn reconnect(&mut self, current_position: u64) -> io::Result<()> {
        self.stream.reconnect(current_position).await
    }

    async fn on_finish(
        &mut self,
        result: io::Result<()>,
        outcome: StreamOutcome,
    ) -> io::Result<()> {
        let check_command_error = if result.is_ok() {
            let wait_res = self.child_handle.wait().await?;
            let command_failed = !wait_res.success();
            if command_failed {
                warn!("command exited with error code: {wait_res:?}");
            }
            command_failed
        } else {
            debug!("killing child process");
            self.child_handle.kill().await?;
            debug!("child process killed");
            // Don't need to check process for errors if the user explicitly cancelled the stream
            outcome == StreamOutcome::Completed
        };

        if check_command_error {
            self.check_stderr_files();
        }
        self.close_stderr_files();

        result
    }
}

impl Stream for ProcessStream {
    type Item = io::Result<Bytes>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.stream).poll_next(cx)
    }
}
