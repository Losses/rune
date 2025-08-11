#[cfg(test)]
mod tests {
    use fsio::FsIo;
    use log::info;

    use crate::{
        analyzer::core_analyzer::Analyzer, legacy::legacy_fft_v1, measure_time,
        utils::computing_device::ComputingDevice,
    };

    #[test]
    fn test_analyze_cpu_vs_legacy() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let fsio = FsIo::new();
        let window_size = 1024;
        let overlap_size = 512;

        let mut analyzer =
            Analyzer::new(ComputingDevice::Cpu, window_size, overlap_size, None, None);

        let cpu_result = measure_time!("CPU FFT", analyzer.process(&fsio, file_path)).unwrap();

        let legacy_cpu_result = measure_time!(
            "LEGACY CPU FFT",
            legacy_fft_v1::fft(&fsio, file_path, window_size, overlap_size, None)
        )
        .unwrap();

        info!("CPU result: {cpu_result:?}");
        info!("Legacy CPU result: {legacy_cpu_result:?}");

        // Compare results with tolerance
        assert!(
            (cpu_result.rms - legacy_cpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            cpu_result.rms,
            legacy_cpu_result.rms
        );
        assert!(
            (cpu_result.energy - legacy_cpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            cpu_result.energy,
            legacy_cpu_result.energy
        );
        assert!(
            cpu_result.zcr.abs_diff(legacy_cpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            cpu_result.zcr,
            legacy_cpu_result.zcr
        );
    }

    #[tokio::test]
    async fn test_analyze_gpu_vs_legacy() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let fsio = FsIo::new();
        let window_size = 1024;
        let overlap_size = 512;

        let mut analyzer =
            Analyzer::new(ComputingDevice::Gpu, window_size, overlap_size, None, None);

        let gpu_result = measure_time!("GPU FFT", analyzer.process(&fsio, file_path)).unwrap();

        let legacy_cpu_result = measure_time!(
            "LEGACY CPU FFT",
            legacy_fft_v1::fft(&fsio, file_path, window_size, overlap_size, None)
        )
        .unwrap();

        info!("GPU result: {gpu_result:?}");
        info!("Legacy CPU result: {legacy_cpu_result:?}");

        // Compare results with tolerance
        assert!(
            (gpu_result.rms - legacy_cpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            gpu_result.rms,
            legacy_cpu_result.rms
        );
        assert!(
            (gpu_result.energy - legacy_cpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            gpu_result.energy,
            legacy_cpu_result.energy
        );
        assert!(
            gpu_result.zcr.abs_diff(legacy_cpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            gpu_result.zcr,
            legacy_cpu_result.zcr
        );
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

        let cpu_result = measure_time!("CPU FFT", analyzer.process(&fsio, file_path)).unwrap();

        let mut analyzer =
            Analyzer::new(ComputingDevice::Gpu, window_size, overlap_size, None, None);

        let gpu_result = measure_time!("GPU FFT", analyzer.process(&fsio, file_path)).unwrap();

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
}
