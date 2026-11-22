//! Opt-in process test checkpoints, excluded from ordinary builds.

use std::env;
use std::fs;
use std::thread;
pub(crate) fn stop(stage: &str) {
    if env::var("RUNE_SYNC_FAULT_STAGE").ok().as_deref() != Some(stage) {
        return;
    }
    let marker = env::var_os("RUNE_SYNC_FAULT_MARKER").expect("missing fault marker path");
    fs::write(marker, stage).expect("could not write fault marker");
    loop {
        thread::park();
    }
}
