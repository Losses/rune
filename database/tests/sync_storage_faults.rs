#![cfg(all(
    feature = "sync-crash-probe",
    target_os = "linux",
    target_arch = "x86_64"
))]

#[path = "support/storage_replay.rs"]
mod storage_replay;

#[test]
fn synchronized_page_survives_storage_fault_replay() -> anyhow::Result<()> {
    storage_replay::run()
}
