#[cfg(test)]
mod tests {
    use fsio::FsIo;
    use log::info;
    use std::path::Path;

    use crate::{
        analyzer::core_analyzer::Analyzer, measure_time, utils::computing_device::ComputingDevice,
    };

    #[test]
    fn test_analyze_cpu_v2_is_deterministic() {
        let fsio = FsIo::new();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");
        let run = || {
            Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None)
                .process(&fsio, path)
                .unwrap()
                .unwrap()
        };
        let a = run();
        let b = run();
        assert_eq!(a.spectrum, b.spectrum);
        assert_eq!(a.rms, b.rms);
        assert_eq!(a.energy, b.energy);
        assert_eq!(a.zcr, b.zcr);
        assert!(a.total_samples > 0 && a.rms.is_finite());
        assert!(a.spectrum.iter().any(|x| x.re > 0.0));
    }

    #[tokio::test]
    async fn test_fft_cpu_vs_gpu() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let fsio = FsIo::new();
        let window_size = 1024;
        let overlap_size = 512;

        let mut analyzer =
            Analyzer::new(ComputingDevice::Cpu, window_size, overlap_size, None, None);

        let cpu_result = measure_time!("CPU FFT", analyzer.process(&fsio, file_path))
            .unwrap()
            .unwrap();

        let mut analyzer =
            Analyzer::new(ComputingDevice::Gpu, window_size, overlap_size, None, None);

        let gpu_result = measure_time!("GPU FFT", analyzer.process(&fsio, file_path))
            .unwrap()
            .unwrap();

        info!("CPU result: {cpu_result:?}");
        info!("GPU result: {gpu_result:?}");

        // Compare results with tolerance
        assert!(
            (cpu_result.rms - gpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            cpu_result.rms,
            gpu_result.rms
        );
        assert!(
            (cpu_result.energy - gpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            cpu_result.energy,
            gpu_result.energy
        );
        assert!(
            cpu_result.zcr.abs_diff(gpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            cpu_result.zcr,
            gpu_result.zcr
        );

        // Compare spectrum values
        for (i, (cpu_value, gpu_value)) in cpu_result
            .spectrum
            .iter()
            .zip(gpu_result.spectrum.iter())
            .enumerate()
        {
            assert!(
                (cpu_value.norm() - gpu_value.norm()).abs() < 0.001,
                "Spectrum difference too large at index {}: {} vs {}",
                i,
                cpu_value.norm(),
                gpu_value.norm()
            );
        }
    }

    #[test]
    fn test_analyze_opus() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "/tmp/test_startup.opus";
        if !Path::new(file_path).exists() {
            return;
        }
        let fsio = FsIo::new();
        let mut analyzer = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        let result = analyzer.process(&fsio, file_path).unwrap();
        assert!(result.is_some(), "Opus analysis should succeed");
        let desc = result.unwrap();
        assert!(desc.total_samples > 0);
        assert_eq!(desc.sample_rate, 48000);
        info!(
            "Opus analysis result: sample_rate={}, duration={}, total_samples={}",
            desc.sample_rate, desc.duration, desc.total_samples
        );
    }

    #[test]
    fn test_analyze_aac() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "/tmp/test_startup.m4a";
        if !Path::new(file_path).exists() {
            return;
        }
        let fsio = FsIo::new();
        let mut analyzer = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        let result = analyzer.process(&fsio, file_path).unwrap();
        assert!(result.is_some(), "AAC analysis should succeed");
        let desc = result.unwrap();
        assert!(desc.total_samples > 0);
        assert_eq!(desc.sample_rate, 44100);
        info!(
            "AAC analysis result: sample_rate={}, duration={}, total_samples={}",
            desc.sample_rate, desc.duration, desc.total_samples
        );
    }
}
