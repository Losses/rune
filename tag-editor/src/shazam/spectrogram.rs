use anyhow::Result;
use rustfft::{num_complex::Complex, num_traits::Float, FftPlanner};
use std::{f64::consts::PI, fmt};

use super::signature::FrequencyPeak;

const FREQ_BIN_SIZE: usize = 2048;
const HOP_SIZE: usize = 128;
const SAMPLE_OVERLAP: usize = 45;

const TIME_OFFSETS: [i32; 14] = [
    -53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
];

const NEIGHBORS: [i32; 8] = [-10, -7, -4, -3, 1, 2, 5, 8];

const SPREAD_OFFSETS: [i32; 3] = [-2, -4, -7];

#[derive(Debug)]
pub struct SpectralPeaks {
    pub sample_rate: i32,
    pub num_samples: i32,
    pub peaks_by_band: [Vec<FrequencyPeak>; 5],
}

impl fmt::Display for SpectralPeaks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Spectral Peaks:")?;
        writeln!(f, "= Sample Rate: {} Hz", self.sample_rate)?;
        writeln!(f, "= Total Samples: {}", self.num_samples)?;

        for (band_index, peaks) in self.peaks_by_band.iter().enumerate() {
            writeln!(f, "= Band {}: {} peaks", band_index, peaks.len())?;
            // for peak in peaks {
            //     writeln!(
            //         f,
            //         "== Pass: {}, Magnitude: {}, Bin: {}",
            //         peak.pass, peak.magnitude, peak.bin
            //     )?;
            // }
        }

        Ok(())
    }
}

struct Ring<T> {
    buf: Vec<T>,
    index: usize,
}

impl<T: Clone + Default> Ring<T> {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![T::default(); size],
            index: 0,
        }
    }

    fn mod_index(&self, i: i32) -> usize {
        let size = self.buf.len() as i32;
        let mut idx = self.index as i32 + i;
        while idx < 0 {
            idx += size;
        }
        (idx % size) as usize
    }

    fn at(&self, offset: i32) -> &T {
        &self.buf[self.mod_index(offset)]
    }

    fn at_mut(&mut self, offset: i32) -> &mut T {
        let idx = self.mod_index(offset);
        &mut self.buf[idx]
    }

    fn append(&mut self, values: &[T]) {
        for value in values {
            self.buf[self.index] = value.clone();
            self.index = (self.index + 1) % self.buf.len();
        }
    }
}

pub struct SpectrogramProcessor {
    samples_ring: Ring<f64>,
    fft_outputs: Ring<Vec<f64>>,
    spread_outputs: Ring<Vec<f64>>,
    window: Vec<f64>,
    fft_planner: FftPlanner<f64>,
    sample_rate: f64,
    total_samples: usize,
}

impl Default for SpectrogramProcessor {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

impl SpectrogramProcessor {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            samples_ring: Ring::new(FREQ_BIN_SIZE),
            fft_outputs: Ring::new(256),
            spread_outputs: Ring::new(256),
            window: Self::generate_hanning_window(),
            fft_planner: FftPlanner::new(),
            sample_rate,
            total_samples: 0,
        }
    }

    fn generate_hanning_window() -> Vec<f64> {
        (0..FREQ_BIN_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / (FREQ_BIN_SIZE as f64 - 1.0)).cos())
            .collect()
    }

    fn normalize_peak(x: f64) -> f64 {
        x.max(1.0 / 64.0).ln() * 1477.3 + 6144.0
    }

    fn get_peak_band(bin: i32, sample_rate: f64) -> Option<usize> {
        let hz = (bin as f64 * sample_rate) / (2.0 * 1024.0 * 64.0);

        match hz {
            hz if (250.0..520.0).contains(&hz) => Some(0),
            hz if (520.0..1450.0).contains(&hz) => Some(1),
            hz if (1450.0..3500.0).contains(&hz) => Some(2),
            hz if (3500.0..=5500.0).contains(&hz) => Some(3),
            _ => None,
        }
    }

    fn find_max_neighbor(
        &self,
        spread_outputs: &Ring<Vec<f64>>,
        time_idx: usize,
        freq_idx: usize,
    ) -> f64 {
        let mut max_val = 0.0;

        // Check neighboring frequencies at t-49
        for &offset in &NEIGHBORS {
            let neighbor_idx = freq_idx as i32 + offset;
            if (0..1025).contains(&neighbor_idx) {
                max_val = max_val.max(spread_outputs.at(-49)[neighbor_idx as usize]);
            }
        }

        // Check neighboring time frames
        for &offset in &TIME_OFFSETS {
            let t_idx = time_idx as i32 + offset;
            if t_idx >= 0 && t_idx < spread_outputs.buf.len() as i32 && freq_idx > 0 {
                max_val = max_val.max(spread_outputs.at(offset)[freq_idx - 1]);
            }
        }

        max_val
    }

    pub fn process_samples(&mut self, samples: &[f64]) -> Result<()> {
        for chunk in samples.chunks(HOP_SIZE) {
            // Process each chunk of samples
            for (i, &sample) in chunk.iter().enumerate() {
                self.samples_ring.append(&[sample]);
                if i >= HOP_SIZE {
                    break;
                }
            }

            // Perform FFT when we have enough samples
            if self.total_samples >= FREQ_BIN_SIZE {
                // Apply scaling and window function
                let mut fft_input: Vec<Complex<f64>> = (0..FREQ_BIN_SIZE)
                    .map(|i| {
                        let sample = self.samples_ring.at(-(FREQ_BIN_SIZE as i32) + i as i32);
                        let scaled = (sample * 1024.0 * 64.0).round();
                        Complex::new(scaled * self.window[i], 0.0)
                    })
                    .collect();

                // Perform FFT
                let fft = self.fft_planner.plan_fft_forward(FREQ_BIN_SIZE);
                fft.process(&mut fft_input);

                // Calculate magnitudes with minimum value protection
                let magnitudes: Vec<f64> = fft_input[..1025]
                    .iter()
                    .map(|c| {
                        let mag = (c.re * c.re + c.im * c.im) / (1 << 17) as f64;
                        mag.max(0.0000000001)
                    })
                    .collect();

                self.fft_outputs.append(&[magnitudes.clone()]);

                // Frequency domain spreading
                let mut spread = magnitudes.clone();
                for i in 0..spread.len() - 2 {
                    spread[i] = spread[i..=i + 2].iter().copied().fold(0.0, f64::max);
                }
                self.spread_outputs.append(&[spread.clone()]);

                // Apply time domain spreading
                if self.spread_outputs.buf.len() > 7 {
                    for &offset in &SPREAD_OFFSETS {
                        let spread_output = self.spread_outputs.at_mut(offset);
                        for (i, &magnitude) in spread.iter().enumerate() {
                            if i < spread_output.len() {
                                spread_output[i] = spread_output[i].max(magnitude);
                            }
                        }
                    }
                }
            }

            self.total_samples += chunk.len();
        }

        Ok(())
    }

    pub fn extract_peaks(&self) -> SpectralPeaks {
        let mut peaks_by_band = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];

        if self.fft_outputs.buf.len() <= SAMPLE_OVERLAP {
            return SpectralPeaks {
                sample_rate: self.sample_rate as i32,
                num_samples: self.total_samples as i32,
                peaks_by_band,
            };
        }

        for i in SAMPLE_OVERLAP..self.fft_outputs.buf.len() {
            let fft_output = &self.fft_outputs.at(-(i as i32));

            for bin in 10..1015 {
                if fft_output[bin] <= self.find_max_neighbor(&self.spread_outputs, i, bin) {
                    continue;
                }

                let before = Self::normalize_peak(fft_output[bin - 1]);
                let peak = Self::normalize_peak(fft_output[bin]);
                let after = Self::normalize_peak(fft_output[bin + 1]);

                let variation = ((32.0 * (after - before)) / (2.0 * peak - after - before)) as i32;
                let peak_bin = bin as i32 * 64 + variation;

                if let Some(band) = Self::get_peak_band(peak_bin, self.sample_rate) {
                    peaks_by_band[band].push(FrequencyPeak {
                        pass: (i - SAMPLE_OVERLAP) as i32,
                        magnitude: peak as i32,
                        bin: peak_bin,
                    });
                }
            }
        }

        SpectralPeaks {
            sample_rate: self.sample_rate as i32,
            num_samples: self.total_samples as i32,
            peaks_by_band,
        }
    }
}
