//! Opt-in process test checkpoints, excluded from ordinary builds.

pub(crate) fn stop(stage: &str) {
    if std::env::var("RUNE_SYNC_FAULT_STAGE").ok().as_deref() != Some(stage) {
        return;
    }
    let marker = std::env::var_os("RUNE_SYNC_FAULT_MARKER").expect("missing fault marker path");
    std::fs::write(marker, stage).expect("could not write fault marker");
    loop {
        std::thread::park();
    }
}
