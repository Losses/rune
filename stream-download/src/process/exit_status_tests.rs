use std::io::{self, Read};
use std::time::Duration;

use tokio::task::spawn_blocking;
use tokio::time::timeout;

use super::{Command, ProcessStream, ProcessStreamParams};
use crate::storage::memory::MemoryStorageProvider;
use crate::{Settings, StreamDownload, StreamPhase};

async fn shell_stream(script: &str) -> StreamDownload<MemoryStorageProvider> {
    shell_stream_with(script, Settings::default()).await
}

async fn shell_stream_with(
    script: &str,
    settings: Settings<ProcessStream>,
) -> StreamDownload<MemoryStorageProvider> {
    let params = ProcessStreamParams::new(Command::new("sh").args(["-c", script]))
        .expect("spawn test command");
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
    let (result, _stream) = timeout(Duration::from_secs(5), read)
        .await
        .expect("peer step timed out: read process output")
        .expect("reader thread");
    timeout(Duration::from_secs(5), handle.wait_for_completion())
        .await
        .expect("peer step timed out: process stream completion");
    result
}

#[tokio::test]
async fn nonzero_exit_after_output_eof_fails_the_stream() {
    let result = read_to_end(shell_stream("printf data; exit 3").await).await;
    assert!(
        result.is_err(),
        "a command that exits 3 must fail the stream, got {result:?}"
    );
}

#[tokio::test]
async fn zero_exit_after_output_eof_completes_the_stream() {
    let output = read_to_end(shell_stream("printf data").await)
        .await
        .expect("a command that exits 0 must complete the stream");
    assert_eq!(output, b"data");
}

#[tokio::test]
async fn cancelling_fails_the_read_without_waiting_for_the_command() {
    let stream = shell_stream("printf data; exec sleep 60").await;
    stream.cancel_download();
    let result = read_to_end(stream).await;
    assert!(
        result.is_err(),
        "a cancelled stream must not complete, got {result:?}"
    );
}

#[tokio::test]
async fn cancelling_after_output_eof_does_not_wait_for_the_command() {
    // The command closes its output and keeps running, so the stream waits for it to exit. The
    // cancel comes once all of the output has been stored, while the stream waits.
    let settings = Settings::default().on_progress(|_, state, cancellation_token| {
        if matches!(state.phase, StreamPhase::Complete) {
            cancellation_token.cancel();
        }
    });
    let stream = shell_stream_with("printf data; exec >&-; exec sleep 60", settings).await;
    let result = read_to_end(stream).await;
    assert!(
        result.is_err(),
        "a stream cancelled before its command exits must not complete, got {result:?}"
    );
}
