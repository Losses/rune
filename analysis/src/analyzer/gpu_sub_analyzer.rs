use crate::analyzer::sub_analyzer::SubAnalyzer;
use crate::analyzer::analyzer::{self, Analyzer};
use crate::utils::analyzer_utils::build_hanning_window;
use std::string;
use std::sync::Arc;

use log::{debug, info};

use realfft::{RealFftPlanner, RealToComplex};
use rubato::{FftFixedInOut, Resampler};
use rustfft::num_complex::Complex;
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error;
use symphonia::core::sample::Sample;
use tokio_util::sync::CancellationToken;

use crate::shared_utils::computing_device_type::ComputingDevice;
use crate::utils::features::energy;
use crate::utils::features::rms;
use crate::utils::features::zcr;

use crate::shared_utils::analyzer_shared_utils::*;
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
    fn process_audio_chunk(
        &mut self,
        base_analyzer: &mut Analyzer,
        chunk: &[f32],
        force: bool,
    ) {
        let resampled_chunk = &base_analyzer
            .resampler
            .as_mut()
            .unwrap()
            .process(&[chunk], None)
            .unwrap()[0];

        base_analyzer.total_rms += rms(resampled_chunk);
        base_analyzer.total_zcr += zcr(resampled_chunk);
        base_analyzer.total_energy += energy(resampled_chunk);

        let start_idx = self.batch_cache_buffer_count * base_analyzer.window_size;
        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= base_analyzer.window_size {
                break;
            }
            let windowed_sample = sample * self.hanning_window[i];
            self.batch_fft_buffer[start_idx + i] = Complex::new(windowed_sample, 0.0);
        }

        self.batch_cache_buffer_count += 1;
        base_analyzer.count += 1;

        if force {
            // Only process the valid portion of the batch buffer
            pollster::block_on(self.gpu_fft.compute_fft(&mut self.batch_fft_buffer));

            // Accumulate spectrums for the valid batches
            for batch_idx in 0..self.batch_cache_buffer_count {
                let start = batch_idx * base_analyzer.window_size;
                for i in 0..base_analyzer.window_size {
                    base_analyzer.avg_spectrum[i] += self.batch_fft_buffer[start + i];
                }
            }

            self.batch_cache_buffer_count = 0;
        } else if self.batch_cache_buffer_count >= base_analyzer.batch_size {
            pollster::block_on(self.gpu_fft.compute_fft(&mut self.batch_fft_buffer));

            // Split batch_fft_buffer into batches and accumulate into avg_spectrum
            for batch_idx in 0..base_analyzer.batch_size {
                let start = batch_idx * base_analyzer.window_size;

                for i in 0..base_analyzer.window_size {
                    base_analyzer.avg_spectrum[i] += self.batch_fft_buffer[start + i];
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::legacy::{legacy_fft_processor::gpu_fft};
    use crate::measure_time;

    use super::*;

    #[test]
    fn test_gpu_analyzer() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let batch_size = 1024 * 8;
        let overlap_size = 512;

        let mut analyzer = Analyzer::new(ComputingDevice::Cpu,window_size, overlap_size, 1, None);
        let gpu_result = measure_time!("GPU FFT", analyzer.process(file_path).unwrap());

        let gpu_result1 = measure_time!(
            "GPU FFT1",
            gpu_fft(file_path, window_size, batch_size, overlap_size, None)
        )
        .unwrap();

        info!("GPU result: {:?}", gpu_result);
        info!("GPU result: {:?}", gpu_result);

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
