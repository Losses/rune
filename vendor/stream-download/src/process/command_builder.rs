use std::io;
use std::process::Stdio;

use super::{Command, SpawnCommand, SpawnedCommand, WrapIoResult, stdio_to_tmp_file};

/// A builder object that can pipe multiple commands together and automatically configure the child
/// process' handles correctly.
#[derive(Debug)]
pub struct CommandBuilder {
    commands: Vec<Command>,
}

impl CommandBuilder {
    /// Creates a new [`CommandBuilder`].
    pub fn new<C>(command: C) -> Self
    where
        C: Into<Command>,
    {
        Self {
            commands: vec![command.into()],
        }
    }

    /// Adds a new [`Command`] to the pipeline. The previous command's `stdout` stream will be piped
    /// into this command's `stdin`. This is equivalent to doing `cmd1 | cmd2`.
    #[must_use]
    pub fn pipe<C>(mut self, command: C) -> Self
    where
        C: Into<Command>,
    {
        self.commands.push(command.into());
        self
    }
}

impl SpawnCommand for CommandBuilder {
    fn spawn(mut self) -> io::Result<SpawnedCommand> {
        let last = self.commands.pop().expect("prevented by constructor");
        let mut prev_stdout = None;
        let mut stderr_files = Vec::new();

        for command in self.commands {
            let mut std_command = std::process::Command::new(command.program);
            std_command
                .args(command.args.clone())
                .stdout(Stdio::piped());
            if let Some(handle) = command.stderr_handle {
                std_command.stderr(handle);
            } else {
                let (stdio, stderr_file) = stdio_to_tmp_file()?;
                std_command.stderr(stdio);
                stderr_files.push(stderr_file);
            }

            #[cfg(target_os = "windows")]
            {
                // CREATE_NO_WINDOW
                use std::os::windows::process::CommandExt;
                std_command.creation_flags(0x08000000);
            }

            if let Some(prev_stdout) = prev_stdout.take() {
                std_command.stdin(prev_stdout);
            }
            let mut command_out = std_command.spawn().wrap_err("error spawning process")?;
            prev_stdout = command_out.stdout.take();
        }
        SpawnedCommand::new(last, prev_stdout, stderr_files)
    }
}
