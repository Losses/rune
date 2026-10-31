use std::{hint::black_box, time::Duration};

use analysis::{analysis::analyze_audio, utils::computing_device::ComputingDevice};
use criterion::{Criterion, criterion_group, criterion_main};
use fsio::FsIo;

fn cpu_analysis_benchmark(c: &mut Criterion) {
    let fsio = FsIo::new();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");
    let mut group = c.benchmark_group("cpu analysis");
    group
        .significance_level(0.01)
        .sample_size(10)
        .measurement_time(Duration::from_secs(30));
    group.bench_function("current analysis", |b| {
        b.iter(|| {
            analyze_audio(
                &fsio,
                black_box(path),
                1024,
                512,
                ComputingDevice::Cpu,
                None,
            )
            .unwrap()
            .unwrap()
        })
    });
    group.finish();
}

criterion_group!(benches, cpu_analysis_benchmark);
criterion_main!(benches);
