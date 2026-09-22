use std::path::Path;
#[cfg(target_os = "android")]
use std::time::Instant;

use fsio::FsIo;
#[cfg(target_os = "android")]
use fsio::self_test::skipped_layer_results;
use fsio::self_test::{FsSelfTestLayerResult, run_fs_self_test};
use log::{error, info};
use rinf::{DartSignal, RustSignal};

use crate::messages::*;

const LAYER_SAF_INIT: &str = "L1 SAF initialization (AndroidFsIo)";

fn emit_progress(result: &FsSelfTestLayerResult) {
    FsSelfTestProgress {
        layer: result.layer.clone(),
        ok: result.ok,
        skipped: result.skipped,
        elapsed_ms: result.elapsed_ms as i64,
        detail: result.detail.clone(),
    }
    .send_signal_to_dart();
}

async fn run_self_test(path: &str) -> Vec<FsSelfTestLayerResult> {
    let mut results: Vec<FsSelfTestLayerResult> = Vec::new();

    #[cfg(target_os = "android")]
    {
        let start = Instant::now();
        match FsIo::new(Path::new(".rune/.android-fs.db"), path) {
            Ok(fsio) => {
                let result = FsSelfTestLayerResult {
                    layer: LAYER_SAF_INIT.to_string(),
                    ok: true,
                    skipped: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: "SAF tree accessible; ndk-saf initialized and cache refreshed"
                        .to_string(),
                };
                emit_progress(&result);
                results.push(result);

                let layer_results = run_fs_self_test(&fsio, Path::new(""), emit_progress).await;
                results.extend(layer_results);
            }
            Err(e) => {
                let detail = format!("Failed to construct AndroidFsIo: {e:#?}");
                let result = FsSelfTestLayerResult {
                    layer: LAYER_SAF_INIT.to_string(),
                    ok: false,
                    skipped: false,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: detail.clone(),
                };
                emit_progress(&result);
                results.push(result);

                let skip_reason = format!("Skipped: {detail}");
                for result in skipped_layer_results(&skip_reason) {
                    emit_progress(&result);
                    results.push(result);
                }
            }
        }
    }

    #[cfg(not(target_os = "android"))]
    {
        let result = FsSelfTestLayerResult {
            layer: LAYER_SAF_INIT.to_string(),
            ok: false,
            skipped: true,
            elapsed_ms: 0,
            detail: "Not an Android build; using std filesystem backend".to_string(),
        };
        emit_progress(&result);
        results.push(result);

        let fsio = FsIo::new();
        let layer_results = run_fs_self_test(&fsio, Path::new(path), emit_progress).await;
        results.extend(layer_results);
    }

    results
}

pub async fn receive_fs_self_test() {
    let receiver = RunFsSelfTestRequest::get_dart_signal_receiver();

    info!("FS self-test listener started");

    while let Some(dart_signal) = receiver.recv().await {
        let path = dart_signal.message.path;
        info!("Running filesystem self-test on: {path}");

        let results = std::panic::AssertUnwindSafe(run_self_test(&path));
        let results = match futures::FutureExt::catch_unwind(results).await {
            Ok(results) => results,
            Err(e) => {
                error!("Filesystem self-test panicked: {e:?}");
                let panic_message = e
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| e.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown panic");
                vec![FsSelfTestLayerResult {
                    layer: "Self-test runner".to_string(),
                    ok: false,
                    skipped: false,
                    elapsed_ms: 0,
                    detail: format!("panic during self-test: {panic_message}"),
                }]
            }
        };

        let success = results.iter().all(|r| r.skipped || r.ok);
        let report_json = match serde_json::to_string(&results) {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to serialize self-test report: {e:?}");
                "[]".to_string()
            }
        };

        RunFsSelfTestResponse {
            success,
            report_json,
        }
        .send_signal_to_dart();

        info!("Filesystem self-test finished, success: {success}");
    }
}
