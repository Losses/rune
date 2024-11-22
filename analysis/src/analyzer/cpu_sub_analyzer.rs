use crate::analyzer::core_analyzer::Analyzer;
use crate::analyzer::sub_analyzer::SubAnalyzer;
use crate::utils::hanning_window::build_hanning_window;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rubato::Resampler;
use rustfft::num_complex::Complex;

use crate::utils::features::energy;
use crate::utils::features::rms;
use crate::utils::features::zcr;

pub struct CpuSubAnalyzer {
    cpu_fft: Arc<dyn RealToComplex<f32>>,
    fft_output_buffer: Vec<Complex<f32>>,
    fft_input_buffer: Vec<f32>,
    batch_cache_buffer_count: usize,
    hanning_window: Vec<f32>,
}

impl CpuSubAnalyzer {
    pub fn new(window_size: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let cpu_fft = planner.plan_fft_forward(window_size);

        let fft_output_buffer = cpu_fft.make_output_vec();
        let fft_input_buffer = cpu_fft.make_input_vec();

        CpuSubAnalyzer {
            cpu_fft,
            fft_output_buffer,
            fft_input_buffer,
            batch_cache_buffer_count: 0,
            hanning_window: build_hanning_window(window_size),
        }
    }
}

impl SubAnalyzer for CpuSubAnalyzer {
    fn process_audio_chunk(&mut self, core_analyzer: &mut Analyzer, chunk: &[f32], force: bool) {
        let _ = &core_analyzer
            .resampler
            .as_mut()
            .unwrap()
            .process_into_buffer(&[chunk], &mut core_analyzer.resampler_output_buffer, None)
            .unwrap();

        let resampled_chunk = &core_analyzer.resampler_output_buffer[0];

        core_analyzer.total_rms += rms(resampled_chunk);
        core_analyzer.total_zcr += zcr(resampled_chunk);
        core_analyzer.total_energy += energy(resampled_chunk);

        // TODO: Already find one way to optimize this which can be 0.5ms faster in startup_0.ogg
        let start_idx = self.batch_cache_buffer_count * core_analyzer.window_size;
        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= core_analyzer.window_size {
                break;
            }
            let windowed_sample = sample * self.hanning_window[i];
            self.fft_input_buffer[start_idx + i] = windowed_sample;
        }

        self.batch_cache_buffer_count += 1;
        core_analyzer.count += 1;

        let cpu_fft = self.cpu_fft.as_ref();

        if force || self.batch_cache_buffer_count >= core_analyzer.batch_size {
            cpu_fft
                .process(&mut self.fft_input_buffer, &mut self.fft_output_buffer)
                .expect("Real FFT processing failed");

            let half_window = core_analyzer.window_size / 2;
            let batch_count = if force {
                self.batch_cache_buffer_count
            } else {
                core_analyzer.batch_size
            };

            for batch_idx in 0..batch_count {
                let _start = batch_idx * core_analyzer.window_size;
                // Copy the real FFT output directly
                for i in 0..half_window + 1 {
                    core_analyzer.avg_spectrum[i] += self.fft_output_buffer[i];
                }
                // Reconstruct the conjugate symmetric part
                for i in 1..half_window {
                    core_analyzer.avg_spectrum[core_analyzer.window_size - i] +=
                        self.fft_output_buffer[i].conj();
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}
