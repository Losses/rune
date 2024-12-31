use std::{fmt, fs::File, io::Write, sync::Mutex};

use lazy_static::lazy_static;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

use crate::shazam::ring::Ring;

use super::hanning::HANNING_MULTIPLIERS;

// Time offsets for peak detection
const TIME_OFFSETS: [i32; 14] = [
    -53, -45, 165, 172, 179, 186, 193, 200, 214, 221, 228, 235, 242, 249,
];

// Neighbor offsets for frequency peak detection
const NEIGHBORS: [i32; 8] = [-10, -7, -4, -3, 1, 2, 5, 8];

lazy_static! {
    static ref LOG_FILE: Mutex<File> =
        Mutex::new(File::create("rust_debug.log").expect("Unable to create log file"));
}

macro_rules! log {
    ($($arg:tt)*) => {
        {
            let mut file = LOG_FILE.lock().unwrap();
            writeln!(file, $($arg)*).expect("Failed to write to log file");
            file.flush().expect("Failed to flush log file");
        }
    };
}

#[derive(Debug)]
pub struct FrequencyPeak {
    pub pass: i32,
    pub magnitude: i32,
    pub bin: i32,
}

#[derive(Debug)]
pub struct Signature {
    pub sample_rate: i32,
    pub num_samples: i32,
    pub peaks_by_band: [Vec<FrequencyPeak>; 5],
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Signature:")?;
        writeln!(f, "= Sample Rate: {} Hz", self.sample_rate)?;
        writeln!(f, "= Total Samples: {}", self.num_samples)?;

        // Display the number of peaks in each frequency band
        for (band_index, peaks) in self.peaks_by_band.iter().enumerate() {
            writeln!(f, "= Band {}: {} peaks", band_index, peaks.len())?;
        }

        Ok(())
    }
}

// Define a new type for the array
#[derive(Clone)]
struct F64Array1025([f64; 1025]);

impl Default for F64Array1025 {
    fn default() -> Self {
        F64Array1025([0.0; 1025])
    }
}

pub fn compute_signature(sample_rate: i32, samples: &[f64]) -> Signature {
    log!("=== Starting Rust ComputeSignature ===");
    log!("Sample Rate: {}", sample_rate);
    log!("Number of Samples: {}", samples.len());

    let max_neighbor = |spread_outputs: &Ring<F64Array1025>, i: usize| {
        log!("  max_neighbor called with i: {}", i);
        let mut neighbor = 0.0f64;
        for &off in NEIGHBORS.iter() {
            let idx = i as isize + off as isize;
            if (0..1025).contains(&idx) {
                neighbor = neighbor.max(spread_outputs.at(-49).0[idx as usize]);
                log!(
                    "    off: {}, neighbor: {}, spread_outputs.at(-49).0[{}]: {}",
                    off,
                    neighbor,
                    idx,
                    spread_outputs.at(-49).0[idx as usize]
                );
            } else {
                log!("    off: {}, idx: {} out of bounds (0..1025)", off, idx);
            }
        }
        for &off in TIME_OFFSETS.iter() {
            let idx = i as isize - 1;
            if (0..1025).contains(&idx) {
                neighbor = neighbor.max(spread_outputs.at(off).0[idx as usize]);
                log!(
                    "    off: {}, neighbor: {}, spread_outputs.at({}).0[{}]: {}",
                    off,
                    neighbor,
                    off,
                    idx,
                    spread_outputs.at(off).0[idx as usize]
                );
            } else {
                log!("    off: {}, idx: {} out of bounds (0..1025)", off, idx);
            }
        }
        log!("  max_neighbor returning: {}", neighbor);
        neighbor
    };

    let normalize_peak = |x: f64| {
        let result = (x.max(1.0 / 64.0)).ln() * 1477.3 + 6144.0;
        log!(
            "  normalize_peak called with x: {}, returning: {}",
            x,
            result
        );
        result
    };

    let peak_band = |bin: i32| -> Option<usize> {
        let hz = (bin * sample_rate) / (2 * 1024 * 64);
        let result = match hz {
            hz if (250..520).contains(&hz) => Some(0),
            hz if (520..1450).contains(&hz) => Some(1),
            hz if (1450..3500).contains(&hz) => Some(2),
            hz if (3500..=5500).contains(&hz) => Some(3),
            _ => None,
        };
        log!(
            "  peak_band called with bin: {}, hz: {}, returning: {:?}",
            bin,
            hz,
            result
        );
        result
    };

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(2048);

    let mut samples_ring = Ring::<f64>::new(2048);
    let mut fft_outputs = Ring::<F64Array1025>::new(256);
    let mut spread_outputs = Ring::<F64Array1025>::new(256);
    let mut peaks_by_band: [Vec<FrequencyPeak>; 5] = Default::default();

    for i in 0..(samples.len() as i32 - 128) / 128 {
        log!("Iteration: {} (Rust)", i);
        let start = i * 128;
        // Corrected the end calculation to ensure we take exactly 128 samples
        let end = start + 128;
        if end <= samples.len() as i32 {
            samples_ring.append(&samples[start as usize..end as usize]);
            log!(
                "  samples_ring appended. New length: {}",
                samples_ring.len()
            );

            let mut reordered_samples = vec![Complex::new(0.0f64, 0.0f64); 2048];
            let mut temp = vec![0.0f64; 2048];
            samples_ring.slice(&mut temp, 0);
            log!("  temp sliced from samples_ring");

            // Apply Hanning window and prepare complex input
            for j in 0..temp.len() {
                let normalized = (temp[j] * 1024.0 * 64.0).round();
                reordered_samples[j] = Complex::new(normalized * HANNING_MULTIPLIERS[j], 0.0);
                // log!(
                //     "    reordered_samples[{}]: {} (after hanning)",
                //     j,
                //     reordered_samples[j]
                // );
            }

            // Perform FFT
            fft.process(&mut reordered_samples);

            let mut outputs = [0.0f64; 1025];
            for j in 0..1025 {
                outputs[j] = (reordered_samples[j].re * reordered_samples[j].re
                    + reordered_samples[j].im * reordered_samples[j].im)
                    / (1 << 17) as f64;
                outputs[j] = outputs[j].max(0.0000000001);
                // log!(
                //     "    FFT output[{}]: {} (real: {}, imag: {})",
                //     j,
                //     outputs[j],
                //     reordered_samples[j].re,
                //     reordered_samples[j].im
                // );
            }
            fft_outputs.append(&[F64Array1025(outputs)]);
            // log!("  fft_outputs appended. New length: {}", fft_outputs.len());

            // Spread peaks in frequency domain
            let mut spread = outputs;
            for j in 0..outputs.len() - 2 {
                spread[j] = outputs[j..=j + 2].iter().fold(0.0f64, |a, &b| a.max(b));
                // log!(
                //     "    spread_outputs.0[{}]: {} (after frequency spreading)",
                //     j,
                //     spread[j]
                // );
            }

            // Spread in time domain
            spread_outputs.append(&[F64Array1025(spread)]);
            for &off in &[-2, -4, -7] {
                let idx = spread_outputs.mod_index(spread_outputs.index as i32 + off);
                let mut prev = spread_outputs.buf[idx].0;
                log!("    Time spreading with off: {}", off);
                for j in 0..outputs.len() {
                    prev[j] = prev[j].max(spread[j]);
                    log!("      spreadOutputs.At({})[{}]: {:.6}", off, j, prev[j]);
                }
                spread_outputs.buf[idx] = F64Array1025(prev);
            }

            log!(
                "  spread_outputs appended. New length: {}",
                spread_outputs.len()
            );

            // Skip until we have enough samples
            if i < 45 {
                log!("  Skipping peak recognition (i < 45)");
                continue;
            }

            // Recognize peaks
            let fft_output = fft_outputs.at(-46);
            log!(
                "  Recognizing peaks (fft_output from {} iterations ago)",
                46
            );
            for bin in 10..1015 {
                let neighbor = max_neighbor(&spread_outputs, bin);
                if fft_output.0[bin] <= neighbor {
                    log!(
                        "    bin {}: fft_output.0[{}] ({}) <= max_neighbor ({}). Skipping.",
                        bin,
                        bin,
                        fft_output.0[bin],
                        neighbor
                    );
                    continue;
                }

                let before = normalize_peak(fft_output.0[bin - 1]);
                let peak = normalize_peak(fft_output.0[bin]);
                let after = normalize_peak(fft_output.0[bin + 1]);
                let variation = ((32.0 * (after - before)) / (2.0 * peak - after - before)) as i32;
                let peak_bin = bin as i32 * 64 + variation;
                log!(
                    "    bin {}: before: {}, peak: {}, after: {}, variation: {}, peak_bin: {}",
                    bin,
                    before,
                    peak,
                    after,
                    variation,
                    peak_bin
                );

                if let Some(band) = peak_band(peak_bin) {
                    peaks_by_band[band].push(FrequencyPeak {
                        pass: i - 45,
                        magnitude: peak as i32,
                        bin: peak_bin,
                    });
                    log!(
                        "    Peak added for band {}: Pass: {}, Magnitude: {}, Bin: {}",
                        band,
                        i - 45,
                        peak as i32,
                        peak_bin
                    );
                } else {
                    log!("    bin {}: peak_band returned None. Skipping.", bin);
                }
            }
        }
    }

    log!("=== Finished Rust ComputeSignature ===");
    Signature {
        sample_rate,
        num_samples: samples.len() as i32,
        peaks_by_band,
    }
}
