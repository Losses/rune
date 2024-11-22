use std::{hint::black_box, time::Duration};
use analysis::legacy::legacy_fft_processor::cpu_fft;
use criterion::{criterion_group, criterion_main, Criterion};

fn cpu_analysis_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("cpu analysis");
    group.significance_level(0.01).sample_size(100).measurement_time(Duration::from_secs(20));
    group.bench_function(
        "cpu analysis", |b| b.iter(
            || cpu_fft(black_box("../assets/startup_0.ogg"), 1024, 512, None)
        )
    );
    group.finish();
}

criterion_group!(benches, cpu_analysis_benchmark);
criterion_main!(benches);