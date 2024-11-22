use crate::analyzer::SubAnalyzer;
use crate::base_analyzer::{self, BaseAnalyzer};
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

use crate::computing_device::ComputingDevice;
use crate::features::energy;
use crate::features::rms;
use crate::features::zcr;

use crate::fft_utils::*;
use crate::wgpu_fft::wgpu_radix4;

pub struct CpuSubAnalyzer {
    cpu_fft: Arc<dyn RealToComplex<f32>>,
    fft_output_buffer: Vec<Complex<f32>>,
    fft_input_buffer: Vec<f32>,
    batch_cache_buffer_count: usize,
    hanning_window: Vec<f32>,
}

impl CpuSubAnalyzer {
    pub fn new(window_size: usize, batch_size: usize) -> Self {
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
    fn process_audio_chunk(
        &mut self,
        base_analyzer: &mut BaseAnalyzer,
        chunk: &[f32],
        force: bool,
    ) {
        let _ = &base_analyzer
            .resampler
            .as_mut()
            .unwrap()
            .process_into_buffer(&[chunk], &mut base_analyzer.resampler_output_buffer, None)
            .unwrap();

        let resampled_chunk = &base_analyzer.resampler_output_buffer[0];

        base_analyzer.total_rms += rms(resampled_chunk);
        base_analyzer.total_zcr += zcr(resampled_chunk);
        base_analyzer.total_energy += energy(resampled_chunk);

        // TODO: Already find one way to optimize this which can be 0.5ms faster in startup_0.ogg
        let start_idx = self.batch_cache_buffer_count * base_analyzer.window_size;
        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= base_analyzer.window_size {
                break;
            }
            let windowed_sample = sample * self.hanning_window[i];
            self.fft_input_buffer[start_idx + i] = windowed_sample;
        }

        self.batch_cache_buffer_count += 1;
        base_analyzer.count += 1;

        let cpu_fft = self.cpu_fft.as_ref();

        if force {
            cpu_fft
                .process(&mut self.fft_input_buffer, &mut self.fft_output_buffer)
                .expect("Real FFT processing failed");

            let half_window = base_analyzer.window_size / 2;
            for batch_idx in 0..self.batch_cache_buffer_count {
                let _start: usize = batch_idx * base_analyzer.window_size;
                // Copy the real FFT output directly
                for i in 0..half_window + 1 {
                    base_analyzer.avg_spectrum[i] += self.fft_output_buffer[i];
                }
                // Reconstruct the conjugate symmetric part
                for i in 1..half_window {
                    base_analyzer.avg_spectrum[base_analyzer.window_size - i] +=
                        self.fft_output_buffer[i].conj();
                }
            }

            self.batch_cache_buffer_count = 0;
        } else if self.batch_cache_buffer_count >= base_analyzer.batch_size {
            cpu_fft
                .process(&mut self.fft_input_buffer, &mut self.fft_output_buffer)
                .expect("Real FFT processing failed");

            let half_window = base_analyzer.window_size / 2;

            for batch_idx in 0..base_analyzer.batch_size {
                let _start = batch_idx * base_analyzer.window_size;
                for i in 0..half_window + 1 {
                    base_analyzer.avg_spectrum[i] += self.fft_output_buffer[i];
                }
                for i in 1..half_window {
                    base_analyzer.avg_spectrum[base_analyzer.window_size - i] +=
                        self.fft_output_buffer[i].conj();
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}
