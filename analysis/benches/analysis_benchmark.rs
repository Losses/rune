cfg_if::cfg_if! {
    if #[cfg(feature = "bench")] {
        use std::{hint::black_box, time::Duration};
        use analysis::legacy::{legacy_fft_v1::fft, legacy_fft_v2::cpu_fft};
        use criterion::{criterion_group, criterion_main, Criterion};
        use analysis::shared_utils::computing_device::ComputingDevice;
        use analysis::analyzer::core_analyzer::Analyzer;

        fn cpu_analysis_benchmark(c: &mut Criterion) {
            let mut group = c.benchmark_group("cpu analysis");
            group.significance_level(0.01).sample_size(10).measurement_time(Duration::from_secs(30));
            group.bench_function(
                "cpu analysis v3", |b| b.iter(
                    || Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None).process(black_box("../assets/startup_0.ogg"))
                )
            );
            group.bench_function(
                "cpu analysis v2", |b| b.iter(
                    || cpu_fft(black_box("../assets/startup_0.ogg"), 1024, 512, None)
                )
            );
            group.bench_function(
                "cpu analysis v1", |b| b.iter(
                    || fft(black_box("../assets/startup_0.ogg"), 1024, 512, None)
                )
            );
            group.finish();
        }

        criterion_group!(benches, cpu_analysis_benchmark);
        criterion_main!(benches);
    } else {
        fn main() {
            println!("Benchmarking is disabled. Please enable the 'bench' feature to run benchmarks.");
        }
    }
}
