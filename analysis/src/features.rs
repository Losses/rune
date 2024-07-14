use rustfft::num_complex::Complex;
use std::f32;

pub fn amp_spectrum(complex_spectrum: &Vec<Complex<f32>>, buffer_size: usize) -> Vec<f32> {
    let mut amp_spectrum = vec![0.0; buffer_size / 2];
    for i in 0..buffer_size / 2 {
        amp_spectrum[i] =
            (complex_spectrum[i].re.powi(2) + complex_spectrum[i].im.powi(2)).sqrt();
    }
    amp_spectrum
}

pub fn mu(i: usize, amplitude_spect: &[f32]) -> f32 {
    let (mut numerator, mut denominator) = (0.0, 0.0);
    for (k, &amp) in amplitude_spect.iter().enumerate() {
        numerator += (k.pow(i.try_into().unwrap()) as f32) * amp.abs();
        denominator += amp;
    }
    numerator / denominator
}

pub fn spectral_centroid(amp_spectrum: &[f32]) -> f32 {
    mu(1, amp_spectrum)
}

pub fn spectral_flatness(amp_spectrum: &[f32]) -> f32 {
    let (mut numerator, mut denominator) = (0.0, 0.0);
    for &amp in amp_spectrum {
        numerator += amp.ln();
        denominator += amp;
    }
    (numerator / amp_spectrum.len() as f32).exp() * amp_spectrum.len() as f32 / denominator
}

pub fn spectral_flux(signal: &[f32], previous_signal: &[f32], buffer_size: usize) -> f32 {
    let mut sf = 0.0;
    for i in 0..buffer_size / 2 {
        let x = (signal[i] - previous_signal[i]).abs();
        sf += (x + x.abs()) / 2.0;
    }
    sf
}

pub fn spectral_slope(amp_spectrum: &[f32], sample_rate: f32, buffer_size: usize) -> f32 {
    let (mut amp_sum, mut freq_sum, mut pow_freq_sum, mut amp_freq_sum) = (0.0, 0.0, 0.0, 0.0);
    let mut freqs = vec![0.0; amp_spectrum.len()];

    for (i, &amp) in amp_spectrum.iter().enumerate() {
        amp_sum += amp;
        let cur_freq = (i as f32 * sample_rate) / buffer_size as f32;
        freqs[i] = cur_freq;
        pow_freq_sum += cur_freq * cur_freq;
        freq_sum += cur_freq;
        amp_freq_sum += cur_freq * amp;
    }

    (amp_spectrum.len() as f32 * amp_freq_sum - freq_sum * amp_sum)
        / (amp_sum * (pow_freq_sum - freq_sum.powi(2)))
}

pub fn spectral_rolloff(amp_spectrum: &[f32], sample_rate: f32) -> f32 {
    let nyq_bin = sample_rate / (2.0 * (amp_spectrum.len() - 1) as f32);
    let mut ec = amp_spectrum.iter().sum::<f32>();
    let threshold = 0.99 * ec;
    let mut n = amp_spectrum.len() - 1;

    while ec > threshold && n > 0 {
        ec -= amp_spectrum[n];
        n -= 1;
    }

    (n + 1) as f32 * nyq_bin
}

pub fn spectral_spread(amp_spectrum: &[f32]) -> f32 {
    (mu(2, amp_spectrum) - mu(1, amp_spectrum).powi(2)).sqrt()
}

pub fn spectral_skewness(amp_spectrum: &[f32]) -> f32 {
    let mu1 = mu(1, amp_spectrum);
    let mu2 = mu(2, amp_spectrum);
    let mu3 = mu(3, amp_spectrum);
    let numerator = 2.0 * mu1.powi(3) - 3.0 * mu1 * mu2 + mu3;
    let denominator = (mu2 - mu1.powi(2)).sqrt().powi(3);
    numerator / denominator
}

pub fn spectral_kurtosis(amp_spectrum: &[f32]) -> f32 {
    let mu1 = mu(1, amp_spectrum);
    let mu2 = mu(2, amp_spectrum);
    let mu3 = mu(3, amp_spectrum);
    let mu4 = mu(4, amp_spectrum);
    let numerator = -3.0 * mu1.powi(4) + 6.0 * mu1 * mu2 - 4.0 * mu1 * mu3 + mu4;
    let denominator = (mu2 - mu1.powi(2)).powi(2);
    numerator / denominator
}

pub fn chroma(amp_spectrum: &[f32], chroma_filter_bank: &[Vec<f32>]) -> Vec<f32> {
    let mut chromagram: Vec<f32> = chroma_filter_bank
        .iter()
        .map(|row| row.iter().zip(amp_spectrum).map(|(&r, &a)| r * a).sum())
        .collect();

    let max_val = chromagram.iter().cloned().fold(0. / 0., f32::max);
    if max_val != 0.0 {
        chromagram.iter_mut().for_each(|v| *v /= max_val);
    }

    chromagram
}

pub fn hz_to_octaves(freq: f32, a440: f32) -> f32 {
    (freq / a440).log2() + 4.0
}

pub fn normalize_by_column(matrix: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    let num_cols = matrix[0].len();
    let mut col_sums = vec![0.0; num_cols];

    for row in &matrix {
        for (j, &val) in row.iter().enumerate() {
            col_sums[j] += val;
        }
    }

    matrix
        .into_iter()
        .map(|row| {
            row.into_iter()
                .enumerate()
                .map(|(j, val)| val / col_sums[j])
                .collect()
        })
        .collect()
}

pub fn create_chroma_filter_bank(
    num_filters: usize,
    sample_rate: f32,
    buffer_size: usize,
    center_octave: f32,
    octave_width: f32,
    base_c: bool,
    a440: f32,
) -> Vec<Vec<f32>> {
    let num_output_bins = buffer_size / 2 + 1;
    let mut frequency_bins: Vec<f32> = (0..buffer_size)
        .map(|i| {
            num_filters as f32 * hz_to_octaves((sample_rate * i as f32) / buffer_size as f32, a440)
        })
        .collect();

    frequency_bins[0] = frequency_bins[1] - 1.5 * num_filters as f32;

    let bin_width_bins: Vec<f32> = frequency_bins
        .windows(2)
        .map(|w| (w[1] - w[0]).max(1.0))
        .chain(std::iter::once(1.0))
        .collect();

    let half_num_filters = (num_filters / 2) as isize;
    let filter_peaks: Vec<Vec<f32>> = (0..num_filters)
        .map(|i| {
            frequency_bins
                .iter()
                .map(|&frq| {
                    ((10.0 * num_filters as f32 + half_num_filters as f32 + frq - i as f32)
                        % num_filters as f32)
                        - half_num_filters as f32
                })
                .collect()
        })
        .collect();

    let mut weights: Vec<Vec<f32>> = filter_peaks
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(j, &val)| (-0.5 * (2.0 * val / bin_width_bins[j]).powi(2)).exp())
                .collect()
        })
        .collect();

    weights = normalize_by_column(weights);

    if octave_width > 0.0 {
        let octave_weights: Vec<f32> = frequency_bins
            .iter()
            .map(|&v| {
                (-0.5 * ((v / num_filters as f32 - center_octave) / octave_width).powi(2)).exp()
            })
            .collect();

        weights = weights
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(j, &cell)| cell * octave_weights[j])
                    .collect()
            })
            .collect();
    }

    if base_c {
        weights.rotate_left(3);
    }

    weights
        .into_iter()
        .map(|row| row.into_iter().take(num_output_bins).collect())
        .collect()
}
