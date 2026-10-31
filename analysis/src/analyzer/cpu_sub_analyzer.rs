use crate::analyzer::core_analyzer::Analyzer;
use crate::analyzer::sub_analyzer::SubAnalyzer;
use crate::utils::hanning_window::build_hanning_window;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use media_compute::audio::{AudioKernels, time_domain_stats};

pub struct CpuSubAnalyzer {
    cpu_fft: Arc<dyn RealToComplex<f32>>,
    fft_output_buffer: Vec<Complex<f32>>,
    fft_input_buffer: Vec<f32>,
    hanning_window: Vec<f32>,
    kernels: AudioKernels,
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
            hanning_window: build_hanning_window(window_size),
            kernels: AudioKernels::new(),
        }
    }
}

impl SubAnalyzer for CpuSubAnalyzer {
    fn process_audio_chunk(&mut self, core: &mut Analyzer, chunk: &[f32], _force: bool) {
        let (rms, zcr, energy) = time_domain_stats(chunk);
        core.total_rms += rms;
        core.total_zcr += zcr;
        core.total_energy += energy;
        self.kernels
            .apply_window(chunk, &self.hanning_window, &mut self.fft_input_buffer);
        self.cpu_fft
            .process(&mut self.fft_input_buffer, &mut self.fft_output_buffer)
            .expect("Real FFT processing failed");
        core.count += 1;
        for (sum, bin) in core.avg_spectrum.iter_mut().zip(&self.fft_output_buffer) {
            sum.re += bin.norm_sqr();
        }
    }
}
