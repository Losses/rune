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
    /// A GPU sub-analyzer for `window_size`-sample windows, transformed `batch_size` at a time.
    ///
    /// # Panics
    ///
    /// Panics if `window_size` is not 1024, which is the length of each transform the shader
    /// computes.
    pub fn new(window_size: usize, batch_size: usize) -> Self {
        // The shader transforms each FFT_WINDOW_SIZE values on their own, so any other window
        // would be accumulated from transforms of the wrong length.
        assert_eq!(
            window_size,
            wgpu_radix4::FFT_WINDOW_SIZE,
            "GPU analysis only supports {}-sample windows",
            wgpu_radix4::FFT_WINDOW_SIZE
        );
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
            self.flush(core_analyzer);
        }
    }

    fn flush(&mut self, core_analyzer: &mut Analyzer) {
        if self.batch_cache_buffer_count == 0 {
            return;
        }

        let occupied = self.batch_cache_buffer_count * core_analyzer.window_size;
        let fft_len = occupied
            .next_multiple_of(wgpu_radix4::FFT_WINDOW_SIZE)
            .min(self.batch_fft_buffer.len());
        pollster::block_on(
            self.gpu_fft
                .compute_fft(&mut self.batch_fft_buffer[..fft_len]),
        );

        for batch_idx in 0..self.batch_cache_buffer_count {
            let start = batch_idx * core_analyzer.window_size;
            for i in 0..core_analyzer.window_size {
                core_analyzer.avg_spectrum[i].re += self.batch_fft_buffer[start + i].norm_sqr();
            }
        }

        self.batch_cache_buffer_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use crate::{analyzer::core_analyzer::Analyzer, utils::computing_device::ComputingDevice};

    #[test]
    #[should_panic(expected = "GPU analysis only supports 1024-sample windows")]
    fn a_window_the_gpu_cannot_transform_is_rejected() {
        Analyzer::new(ComputingDevice::Gpu, 2048, 1024, None, None);
    }
}
