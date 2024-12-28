use super::filter::LowPassFilter;
use crate::sampler::SampleEvent;
use anyhow::Result;
use rustfft::{num_complex::Complex, FftPlanner};
use std::{f64::consts::PI, fmt};

const FREQ_BIN_SIZE: usize = 1024;
const HOP_SIZE: usize = FREQ_BIN_SIZE / 32;

#[derive(Debug)]
pub struct Peak {
    pub time: f64,
    pub freq: Complex<f64>,
}

#[derive(Debug)]
pub struct PeakList {
    peaks: Vec<Peak>,
}

impl PeakList {
    pub fn new(peaks: Vec<Peak>) -> Self {
        Self { peaks }
    }
}

impl fmt::Display for PeakList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Peaks:")?;
        for peak in &self.peaks {
            writeln!(
                f,
                "t: {:.2}s, Freq: {:.2} + {:.2}i",
                peak.time, peak.freq.re, peak.freq.im
            )?;
        }
        Ok(())
    }
}

pub struct SpectrogramProgessor {
    current_spectrogram: Vec<Vec<Complex<f64>>>,
    window: Vec<f64>,
    fft_planner: FftPlanner<f64>,
    cutoff_freq: f64,
    accumulated_samples: Vec<f64>,
}

impl Default for SpectrogramProgessor {
    fn default() -> Self {
        Self::new(5000.0)
    }
}

const BANDS: &[(usize, usize)] = &[(0, 10), (10, 20), (20, 40), (40, 80), (80, 160), (160, 512)];

impl SpectrogramProgessor {
    pub fn new(cutoff_freq: f64) -> Self {
        let window = Self::generate_hamming_window();

        Self {
            current_spectrogram: Vec::new(),
            window,
            fft_planner: FftPlanner::new(),
            cutoff_freq,
            accumulated_samples: Vec::new(),
        }
    }

    fn generate_hamming_window() -> Vec<f64> {
        (0..FREQ_BIN_SIZE)
            .map(|i| 0.54 - 0.46 * (2.0 * PI * i as f64 / (FREQ_BIN_SIZE as f64 - 1.0)).cos())
            .collect()
    }

    pub fn pipe_sample_event(&mut self, event: SampleEvent) -> Result<()> {
        let mut low_pass_filter = LowPassFilter::new(self.cutoff_freq, event.sample_rate.into());
        let filtered_samples = low_pass_filter.filter_samples(&event.data);

        // Convert samples to f64
        let samples: Vec<f64> = filtered_samples.iter().map(|&x| x as f64).collect();

        // Accumulate samples
        self.accumulated_samples.extend(samples);

        // Process accumulated samples in overlapping windows
        if self.accumulated_samples.len() >= FREQ_BIN_SIZE {
            let num_windows = (self.accumulated_samples.len() - FREQ_BIN_SIZE) / HOP_SIZE + 1;

            for i in 0..num_windows {
                let start = i * HOP_SIZE;
                let mut buffer: Vec<Complex<f64>> = self.accumulated_samples
                    [start..start + FREQ_BIN_SIZE]
                    .iter()
                    .zip(self.window.iter())
                    .map(|(&sample, &window)| Complex::new(sample * window, 0.0))
                    .collect();

                // Perform FFT
                let fft = self.fft_planner.plan_fft_forward(FREQ_BIN_SIZE);
                fft.process(&mut buffer);

                // Add to spectrogram
                self.current_spectrogram.push(buffer);
            }

            // Keep remaining samples
            let keep_from = (num_windows - 1) * HOP_SIZE + FREQ_BIN_SIZE;
            self.accumulated_samples = self.accumulated_samples[keep_from..].to_vec();
        }

        Ok(())
    }

    pub fn extract_peaks(&self, audio_duration: f64) -> PeakList {
        if self.current_spectrogram.is_empty() {
            return PeakList::new(Vec::new());
        }

        let bin_duration = audio_duration / self.current_spectrogram.len() as f64;
        let mut peaks = Vec::new();

        for (bin_idx, bin) in self.current_spectrogram.iter().enumerate() {
            let mut bin_peaks = Vec::new();

            for &(min, max) in BANDS {
                let mut max_mag = 0.0;
                let mut max_freq = Complex::new(0.0, 0.0);
                let mut max_freq_idx = min;

                for (idx, &freq) in bin[min..max].iter().enumerate() {
                    let magnitude = freq.norm();
                    if magnitude > max_mag {
                        max_mag = magnitude;
                        max_freq = freq;
                        max_freq_idx = min + idx;
                    }
                }

                bin_peaks.push((max_mag, max_freq, max_freq_idx));
            }

            let avg_magnitude =
                bin_peaks.iter().map(|&(mag, _, _)| mag).sum::<f64>() / bin_peaks.len() as f64;

            for (mag, freq, freq_idx) in bin_peaks {
                if mag > avg_magnitude {
                    let peak_time_in_bin = freq_idx as f64 * bin_duration / bin.len() as f64;
                    let peak_time = bin_idx as f64 * bin_duration + peak_time_in_bin;

                    peaks.push(Peak {
                        time: peak_time,
                        freq,
                    });
                }
            }
        }

        PeakList::new(peaks)
    }
}
