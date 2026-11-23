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

#[test]
fn reaping_does_not_queue_behind_a_full_blocking_pool() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("build test runtime");
    runtime.block_on(async {
        // Hold the only blocking-pool slot while the stream finishes. The reaper must not need
        // a pool slot of its own, or completion would wait for this slot forever.
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = spawn_blocking(move || {
            started_tx.send(()).expect("signal blocker started");
            let _ = release_rx.recv();
        });
        started_rx.recv().expect("blocker started");

        let files = test_files("blocking-pool");
        let stream = pipeline_stream(
            intermediate(&files, true),
            Command::new("cat"),
            Settings::default(),
        )
        .await;
        let handle = stream.handle();

        let completed = timeout(PEER_STEP_TIMEOUT, handle.wait_for_completion()).await;
        let _ = release_tx.send(());
        let _ = blocker.await;
        completed.expect("pipeline completion must not wait for a blocking-pool slot");

        let pid = wait_for_pid(&files.pid_file).await;
        ObservedChild::new(pid).assert_reaped("pipeline with a full blocking pool");
    });
}

#[cfg(target_os = "linux")]
#[test]
fn a_failed_later_spawn_reaps_the_started_pipeline_children() {
    let exe = std::env::current_exe().expect("current test binary");
    let output = std::process::Command::new(exe)
        .args([
            "--exact",
            "process::pipeline_tests::failed_spawn_reaping_helper",
            "--nocapture",
        ])
        .env("STREAM_DOWNLOAD_SPAWN_FAILURE_HELPER", "1")
        .output()
        .expect("run the failed-spawn helper");
    assert!(
        output.status.success(),
        "failed-spawn helper exited with {:?}\nstdout: {}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs the failed-spawn scenario in a process of its own, where `waitpid(-1, WNOHANG)` sees
/// only the pipeline child. It returns early when the normal suite runs it directly.
#[cfg(target_os = "linux")]
#[test]
fn failed_spawn_reaping_helper() {
    if std::env::var_os("STREAM_DOWNLOAD_SPAWN_FAILURE_HELPER").is_none() {
        return;
    }
    let dir = tempfile::tempdir().expect("create helper directory");
    let marker = dir.path().join("marker");
    fs::write(&marker, []).expect("create marker file");
    let intermediate = Command::new("sh")
        .args(["-c", "exec tail -f \"$1\"", "sh"])
        .arg(marker.as_os_str());
    let spawn_result = ProcessStreamParams::new(
        CommandBuilder::new(intermediate)
            .pipe(Command::new("stream-download-test-missing-command")),
    );
    assert!(
        spawn_result.is_err(),
        "spawning a missing command must fail"
    );

    // A zero means a child is still running; a pid means it exited and was not waited for.
    // ECHILD is the only answer that proves every started child was reaped. Poll until the
    // killed child has had time to become a zombie, so the unreaped-exit case is observed too.
    let deadline = std::time::Instant::now() + PEER_STEP_TIMEOUT;
    let waited = loop {
        let mut status = 0;
        let waited = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if waited == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "started pipeline child is still running"
            );
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        break waited;
    };
    if waited > 0 {
        eprintln!("started pipeline child exited but was not reaped");
        std::process::exit(2);
    }
    let error = io::Error::last_os_error();
    assert_eq!(
        error.raw_os_error(),
        Some(libc::ECHILD),
        "unexpected waitpid error: {error}"
    );
}
