use anyhow::Result;
use rustfft::{num_complex::Complex, num_traits::Float, FftPlanner};
use std::{f64::consts::PI, fmt};

use super::signature::FrequencyPeak;

// Constants for FFT processing
const FREQ_BIN_SIZE: usize = 2048; // Size of the frequency bin for FFT
const HOP_SIZE: usize = 128;       // Number of samples to process per step
const SAMPLE_OVERLAP: usize = 45;  // Overlap between sample windows

// Time offsets for peak detection
const TIME_OFFSETS: [i32; 14] = [
    -53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
];

// Neighbor offsets for frequency peak detection
const NEIGHBORS: [i32; 8] = [-10, -7, -4, -3, 1, 2, 5, 8];

// Spread offsets for temporal smoothing
const SPREAD_OFFSETS: [i32; 3] = [-2, -4, -7];

#[derive(Debug)]
pub struct SpectralPeaks {
    pub sample_rate: i32,                       // Sample rate of the audio
    pub num_samples: i32,                       // Total number of samples processed
    pub peaks_by_band: [Vec<FrequencyPeak>; 5], // Detected peaks grouped by frequency band
}

impl fmt::Display for SpectralPeaks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Spectral Peaks:")?;
        writeln!(f, "= Sample Rate: {} Hz", self.sample_rate)?;
        writeln!(f, "= Total Samples: {}", self.num_samples)?;

        // Display the number of peaks in each frequency band
        for (band_index, peaks) in self.peaks_by_band.iter().enumerate() {
            writeln!(f, "= Band {}: {} peaks", band_index, peaks.len())?;
        }

        Ok(())
    }
}

// A circular buffer to store data with a fixed size
struct Ring<T> {
    buf: Vec<T>,  // Buffer to hold data
    index: usize, // Current index for insertion
}

impl<T: Clone + Default> Ring<T> {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![T::default(); size],
            index: 0,
        }
    }

    // Calculate the correct index in the circular buffer
    fn mod_index(&self, i: i32) -> usize {
        let size = self.buf.len() as i32;
        let mut idx = self.index as i32 + i;
        while idx < 0 {
            idx += size;
        }
        (idx % size) as usize
    }

    // Retrieve an item from the buffer at a given offset
    fn at(&self, offset: i32) -> &T {
        &self.buf[self.mod_index(offset)]
    }

    // Append values to the buffer, overwriting old data as needed
    fn append(&mut self, values: &[T]) {
        for value in values {
            self.buf[self.index] = value.clone();
            self.index = (self.index + 1) % self.buf.len();
        }
    }
}

pub struct SpectrogramProcessor {
    samples_ring: Ring<f64>,        // Circular buffer for storing samples
    fft_outputs: Ring<Vec<f64>>,    // Circular buffer for FFT outputs
    spread_outputs: Ring<Vec<f64>>, // Circular buffer for spread outputs
    window: Vec<f64>,               // Hanning window for FFT
    fft_planner: FftPlanner<f64>,   // FFT planner for executing FFT
    sample_rate: f64,               // Sample rate of the audio
    total_samples: usize,           // Total number of samples processed
}

impl Default for SpectrogramProcessor {
    fn default() -> Self {
        Self::new(44100.0) // Default sample rate
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

    // Generate a Hanning window for smoothing the FFT input
    fn generate_hanning_window() -> Vec<f64> {
        (0..FREQ_BIN_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / (FREQ_BIN_SIZE as f64 - 1.0)).cos())
            .collect()
    }

    // Normalize the peak value for better readability
    fn normalize_peak(x: f64) -> f64 {
        x.max(1.0 / 64.0).ln() * 1477.3 + 6144.0
    }

    // Determine the frequency band for a given frequency bin
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

    // Process incoming audio samples and perform FFT
    pub fn process_samples(&mut self, samples: &[f64]) -> Result<()> {
        for chunk in samples.chunks(HOP_SIZE) {
            // Add samples to the ring buffer
            for (i, &sample) in chunk.iter().enumerate() {
                self.samples_ring.append(&[sample]);
                if i >= HOP_SIZE {
                    break;
                }
            }

            // Perform FFT when enough samples are available
            if self.total_samples >= FREQ_BIN_SIZE {
                let mut fft_input: Vec<Complex<f64>> = (0..FREQ_BIN_SIZE)
                    .map(|i| {
                        let sample = self.samples_ring.at(-(FREQ_BIN_SIZE as i32) + i as i32);
                        // Scale, round, and apply window function
                        let scaled = (sample * 1024.0 * 64.0).round();
                        Complex::new(scaled * self.window[i], 0.0)
                    })
                    .collect();

                let fft = self.fft_planner.plan_fft_forward(FREQ_BIN_SIZE);
                fft.process(&mut fft_input);

                // Calculate magnitudes of FFT output
                let magnitudes: Vec<f64> = fft_input[..1025]
                    .iter()
                    .map(|c| {
                        let mag = (c.re * c.re + c.im * c.im) / (1 << 17) as f64;
                        mag.max(0.0000000001)
                    })
                    .collect();

                self.fft_outputs.append(&[magnitudes.clone()]);

                // Perform frequency domain spreading
                let mut spread = magnitudes.clone();
                for i in 0..spread.len() - 2 {
                    spread[i] = spread[i..=i + 2].iter().copied().fold(0.0, f64::max);
                }

                // Update spread outputs with temporal smoothing
                self.spread_outputs.append(&[spread.clone()]);
                if self.spread_outputs.buf.len() > 7 {
                    for &offset in &SPREAD_OFFSETS {
                        let prev_idx = self.spread_outputs.mod_index(offset);
                        if let Some(prev_spread) = self.spread_outputs.buf.get(prev_idx) {
                            if !prev_spread.is_empty() {
                                let mut accumulated = spread.clone();
                                for i in 0..accumulated.len().min(prev_spread.len()) {
                                    accumulated[i] = accumulated[i].max(prev_spread[i]);
                                }
                                self.spread_outputs.buf[prev_idx] = accumulated;
                            }
                        }
                    }
                }
            }

            self.total_samples += chunk.len();
        }

        Ok(())
    }

    // Find the maximum value among neighboring frequencies at a given time index
    fn find_max_neighbor(
        &self,
        spread_outputs: &Ring<Vec<f64>>,
        time_idx: usize,
        freq_idx: usize,
    ) -> f64 {
        let mut max_val = 0.0;

        // Check frequency neighbors at time t-49
        if let Some(spread_t49) = spread_outputs.at(-49).get(0..) {
            for &offset in &NEIGHBORS {
                let neighbor_idx = freq_idx as i32 + offset;
                if (0..1025).contains(&neighbor_idx) {
                    if let Some(&val) = spread_t49.get(neighbor_idx as usize) {
                        max_val = max_val.max(val);
                    }
                }
            }
        }

        // Check neighboring frequencies at different time offsets
        for &t_offset in &TIME_OFFSETS {
            let t = time_idx as i32 + t_offset;
            if t >= 0 && freq_idx > 0 {
                if let Some(spread) = spread_outputs.at(t_offset).get(0..) {
                    if let Some(&val) = spread.get(freq_idx - 1) {
                        max_val = max_val.max(val);
                    }
                }
            }
        }

        max_val
    }

    // Extract frequency peaks from the processed data
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
                // Skip if the current bin is not a local maximum
                if fft_output[bin] <= self.find_max_neighbor(&self.spread_outputs, i, bin) {
                    continue;
                }

                // Normalize peak values and calculate variation
                let before = Self::normalize_peak(fft_output[bin - 1]);
                let peak = Self::normalize_peak(fft_output[bin]);
                let after = Self::normalize_peak(fft_output[bin + 1]);

                let variation = ((32.0 * (after - before)) / (2.0 * peak - after - before)) as i32;
                let peak_bin = bin as i32 * 64 + variation;

                // Assign peak to the appropriate frequency band
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
