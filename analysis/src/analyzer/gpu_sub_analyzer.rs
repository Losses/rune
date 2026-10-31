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
        let resampled_chunk = chunk;

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
                    core_analyzer.avg_spectrum[i].re += self.batch_fft_buffer[start + i].norm_sqr();
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}
