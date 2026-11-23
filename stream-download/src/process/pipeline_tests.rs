use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

use tokio::task::spawn_blocking;
use tokio::time::{Instant, sleep, timeout};

use super::{Command, CommandBuilder, ProcessStream, ProcessStreamParams};
use crate::storage::memory::MemoryStorageProvider;
use crate::{Settings, StreamDownload};

const PEER_STEP_TIMEOUT: Duration = Duration::from_secs(5);

struct TestFiles {
    _dir: tempfile::TempDir,
    pid_file: PathBuf,
    marker: PathBuf,
}

fn test_files(name: &str) -> TestFiles {
    let dir = tempfile::tempdir().expect("create test directory");
    let pid_file = dir.path().join(format!("{name}.pid"));
    let marker = dir.path().join(format!("{name}.marker"));
    fs::write(&marker, []).expect("create marker file");
    TestFiles {
        _dir: dir,
        pid_file,
        marker,
    }
}

/// Builds the first command of a pipeline. It records its pid, writes one chunk of output and
/// then replaces itself with a long-running program that keeps the marker in its argv. The last
/// step is an `exec` so that the pid in the file is the process the pipeline owns, not a shell
/// that would leave a grandchild behind.
fn intermediate(files: &TestFiles, close_stdout: bool) -> Command {
    let script = if close_stdout {
        "printf '%s' \"$$\" > \"$1\"; printf data; exec >&-; exec tail -f \"$2\""
    } else {
        "printf '%s' \"$$\" > \"$1\"; printf data; exec tail -f \"$2\""
    };
    Command::new("sh")
        .args(["-c", script, "sh"])
        .arg(files.pid_file.as_os_str())
        .arg(files.marker.as_os_str())
}

async fn pipeline_stream(
    intermediate: Command,
    final_command: Command,
    settings: Settings<ProcessStream>,
) -> StreamDownload<MemoryStorageProvider> {
    let params = ProcessStreamParams::new(CommandBuilder::new(intermediate).pipe(final_command))
        .expect("spawn test pipeline");
    StreamDownload::new_process(params, MemoryStorageProvider, settings)
        .await
        .expect("create process stream")
}

async fn read_to_end(mut stream: StreamDownload<MemoryStorageProvider>) -> io::Result<Vec<u8>> {
    let handle = stream.handle();
    // The reader hands the stream back: dropping it would cancel the download and end the wait
    // for its completion at once.
    let read = spawn_blocking(move || {
        let mut output = Vec::new();
        let result = stream.read_to_end(&mut output).map(|_| output);
        (result, stream)
    });
    let (result, _stream) = timeout(PEER_STEP_TIMEOUT, read)
        .await
        .expect("peer step timed out: read process output")
        .expect("reader thread");
    timeout(PEER_STEP_TIMEOUT, handle.wait_for_completion())
        .await
        .expect("peer step timed out: process stream completion");
    result
}

async fn wait_for_pid(path: &Path) -> libc::pid_t {
    let deadline = Instant::now() + PEER_STEP_TIMEOUT;
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse()
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "peer step timed out: child pid file"
        );
        sleep(Duration::from_millis(5)).await;
    }
}

/// A child whose exit has not been waited for yet. `assert_reaped` distinguishes a running child
/// from a zombie, and cleans up after itself so a failing test does not leave processes behind.
struct ObservedChild {
    pid: libc::pid_t,
    reaped: bool,
}

impl ObservedChild {
    fn new(pid: libc::pid_t) -> Self {
        Self { pid, reaped: false }
    }

    fn assert_reaped(&mut self, description: &str) {
        let mut status = 0;
        let waited = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
        if waited == 0 {
            panic!("{description}: process {} is still running", self.pid);
        }
        if waited == self.pid {
            self.reaped = true;
            panic!(
                "{description}: process {} exited but was not reaped",
                self.pid
            );
        }
        let error = io::Error::last_os_error();
        self.reaped = true;
        assert_eq!(
            error.raw_os_error(),
            Some(libc::ECHILD),
            "{description}: unexpected waitpid error for process {}: {error}",
            self.pid
        );
    }
}

impl Drop for ObservedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        unsafe {
            libc::kill(self.pid, libc::SIGKILL);
            libc::waitpid(self.pid, ptr::null_mut(), 0);
        }
    }
}

#[tokio::test]
async fn completing_a_pipeline_reaps_a_long_running_intermediate() {
    let files = test_files("complete");
    let stream = pipeline_stream(
        intermediate(&files, true),
        Command::new("cat"),
        Settings::default(),
    )
    .await;

    let output = read_to_end(stream).await.expect("stream completes");
    assert_eq!(output, b"data");

    let pid = wait_for_pid(&files.pid_file).await;
    ObservedChild::new(pid).assert_reaped("completed pipeline");
}

#[tokio::test]
async fn cancelling_a_pipeline_reaps_a_long_running_intermediate() {
    let files = test_files("cancel");
    let stream = pipeline_stream(
        intermediate(&files, false),
        Command::new("cat"),
        Settings::default(),
    )
    .await;
    let mut child = ObservedChild::new(wait_for_pid(&files.pid_file).await);

    stream.cancel_download();
    let result = read_to_end(stream).await;
    assert!(
        result.is_err(),
        "a cancelled stream must not complete, got {result:?}"
    );
    child.assert_reaped("cancelled pipeline");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_failed_later_spawn_reaps_the_started_pipeline_children() {
    let files = test_files("spawn-failure");
    let intermediate = Command::new("sh")
        .args(["-c", "exec tail -f \"$1\"", "sh"])
        .arg(files.marker.as_os_str());
    let spawn_result = ProcessStreamParams::new(
        CommandBuilder::new(intermediate)
            .pipe(Command::new("stream-download-test-missing-command")),
    );

    let pids = processes_with_marker(&files.marker);
    for pid in &pids {
        unsafe {
            libc::kill(*pid, libc::SIGKILL);
            libc::waitpid(*pid, ptr::null_mut(), 0);
        }
    }

    assert!(
        spawn_result.is_err(),
        "spawning a missing command must fail"
    );
    assert!(
        pids.is_empty(),
        "children started before the failed spawn are still alive: {pids:?}"
    );
}

/// Finds the children of this test that still carry the unique marker in their argv. The scan does
/// not rely on the child having written its pid before the later command failed to spawn.
#[cfg(target_os = "linux")]
fn processes_with_marker(marker: &Path) -> Vec<libc::pid_t> {
    use std::os::unix::ffi::OsStrExt;

    let marker = marker.as_os_str().as_bytes();
    let mut pids = Vec::new();
    for entry in fs::read_dir("/proc").expect("read /proc").flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<libc::pid_t>().ok())
        else {
            continue;
        };
        let Ok(cmdline) = fs::read(entry.path().join("cmdline")) else {
            continue;
        };
        if cmdline.split(|byte| *byte == 0).any(|arg| arg == marker) {
            pids.push(pid);
        }
    }
    pids
}
