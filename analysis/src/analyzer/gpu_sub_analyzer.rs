use rubato::Resampler;
use rustfft::num_complex::Complex;

use crate::{
    analyzer::{core_analyzer::Analyzer, sub_analyzer::SubAnalyzer},
    utils::{
        features::{energy, rms, zcr},
        hanning_window::build_hanning_window,
    },
};

use crate::wgpu_fft::wgpu_radix4;

pub struct GpuSubAnalyzer {
    batch_cache_buffer_count: usize,
    hanning_window: Vec<f32>,
    batch_fft_buffer: Vec<Complex<f32>>,
    gpu_fft: wgpu_radix4::FFTCompute,
}

impl GpuSubAnalyzer {
    pub fn new(window_size: usize, batch_size: usize) -> Self {
        GpuSubAnalyzer {
            batch_cache_buffer_count: 0,
            hanning_window: build_hanning_window(window_size),
            batch_fft_buffer: vec![Complex::new(0.0, 0.0); window_size * batch_size],
            gpu_fft: pollster::block_on(wgpu_radix4::FFTCompute::new(window_size * batch_size)),
        }
    }
}

impl SubAnalyzer for GpuSubAnalyzer {
    fn process_audio_chunk(&mut self, core_analyzer: &mut Analyzer, chunk: &[f32], force: bool) {
        let resampled_chunk = &core_analyzer
            .resampler
            .as_mut()
            .unwrap()
            .process(&[chunk], None)
            .unwrap()[0];

        core_analyzer.total_rms += rms(resampled_chunk);
        core_analyzer.total_zcr += zcr(resampled_chunk);
        core_analyzer.total_energy += energy(resampled_chunk);

        let start_idx = self.batch_cache_buffer_count * core_analyzer.window_size;
        let buffer_slice =
            &mut self.batch_fft_buffer[start_idx..start_idx + core_analyzer.window_size];
        for (i, sample) in buffer_slice.iter_mut().enumerate() {
            *sample = Complex::new(resampled_chunk[i] * self.hanning_window[i], 0.0);
        }

        self.batch_cache_buffer_count += 1;
        core_analyzer.count += 1;

        if force || self.batch_cache_buffer_count >= core_analyzer.batch_size {
            pollster::block_on(self.gpu_fft.compute_fft(&mut self.batch_fft_buffer));

            let batch_count = if force {
                self.batch_cache_buffer_count
            } else {
                core_analyzer.batch_size
            };

            // Accumulate spectrums for all batches
            for batch_idx in 0..batch_count {
                let start = batch_idx * core_analyzer.window_size;
                for i in 0..core_analyzer.window_size {
                    core_analyzer.avg_spectrum[i] += self.batch_fft_buffer[start + i];
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use fsio::FsIo;
    use log::info;

    use crate::legacy::legacy_fft_v2::gpu_fft;
    use crate::measure_time;
    use crate::utils::computing_device::ComputingDevice;

    use super::*;

    #[test]
    fn test_gpu_analyzer() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let batch_size = 1024 * 8;
        let overlap_size = 512;
        let fsio = FsIo::new();

        let mut analyzer =
            Analyzer::new(ComputingDevice::Cpu, window_size, overlap_size, None, None);
        let gpu_result = measure_time!("GPU FFT", analyzer.process(&fsio, file_path).unwrap());

        let gpu_result1 = measure_time!(
            "GPU FFT1",
            gpu_fft(
                &fsio,
                file_path,
                window_size,
                batch_size,
                overlap_size,
                None
            )
        )
        .unwrap();

        info!("GPU result: {gpu_result:?}");
        info!("GPU result: {gpu_result:?}");

        assert!(
            (gpu_result.rms - gpu_result1.rms).abs() < 0.01,
            "GPU FFT result mismatch"
        );
        assert!(
            (gpu_result.energy - gpu_result1.energy).abs() < 5.0,
            "GPU FFT result mismatch"
        );
        assert!(
            gpu_result.zcr.abs_diff(gpu_result1.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            gpu_result.zcr,
            gpu_result1.zcr
        );
        for (i, (gpu_result, gpu_value1)) in gpu_result
            .spectrum
            .iter()
            .zip(gpu_result.spectrum.iter())
            .enumerate()
        {
            assert!(
                (gpu_result.norm() - gpu_value1.norm()).abs() < 0.001,
                "Spectrum difference too large at index {}: {} vs {}",
                i,
                gpu_result.norm(),
                gpu_value1.norm()
            );
        }
    }
}
