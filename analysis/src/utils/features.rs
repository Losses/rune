use std::{collections::HashMap, f32, f32::consts::PI, sync::Mutex};

use anyhow::{Result, bail};
use lazy_static::lazy_static;
use rustfft::num_complex::Complex;

// Code refactored from:
// * https://github.com/meyda/meyda/ (MIT)
// * https://github.com/vail-systems/node-dct/ (MIT)

// Time-domain Features

pub fn rms(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }

    let mut rms: f32 = 0.0;
    for &value in signal.iter() {
        rms += value.powi(2);
    }

    rms /= signal.len() as f32;
    rms = rms.sqrt();

    rms
}

pub fn zcr(signal: &[f32]) -> usize {
    if signal.is_empty() {
        return 0;
    }

    let mut zcr = 0;
    for i in 1..signal.len() {
        if (signal[i - 1] >= 0.0 && signal[i] < 0.0) || (signal[i - 1] < 0.0 && signal[i] >= 0.0) {
            zcr += 1;
        }
    }

    zcr
}

pub fn energy(signal: &[f32]) -> f32 {
    if signal.is_empty() {
        return 0.0;
    }

    let mut energy = 0.0;
    for &value in signal.iter() {
        energy += value.abs().powi(2);
    }

    energy
}

// Spectral Features

pub fn amp_spectrum(complex_spectrum: &[Complex<f32>], buffer_size: usize) -> Vec<f32> {
    let mut amp_spectrum = vec![0.0; buffer_size / 2];
    for i in 0..buffer_size / 2 {
        amp_spectrum[i] = (complex_spectrum[i].re.powi(2) + complex_spectrum[i].im.powi(2)).sqrt();
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

    let max_val = chromagram.iter().cloned().fold(0., f32::max);
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

// Perceptual Features

pub struct BarkLoudness {
    pub specific: Vec<f32>,
    pub total: f32,
}

pub fn loudness(
    amp_spectrum: &[f32],
    bark_scale: &[f32],
    number_of_bark_bands: Option<usize>,
) -> Result<BarkLoudness> {
    let number_of_bark_bands = number_of_bark_bands.unwrap_or(24);

    if amp_spectrum.len() != bark_scale.len() {
        bail!(
            "ampSpectrum({}) and barkScale({}) must have the same length",
            amp_spectrum.len(),
            bark_scale.len()
        );
    }

    let mut specific = vec![0.0; number_of_bark_bands];
    let normalised_spectrum = amp_spectrum;
    let mut bb_limits = vec![0; number_of_bark_bands + 1];

    bb_limits[0] = 0;
    let mut current_band_end =
        bark_scale[normalised_spectrum.len() - 1] / number_of_bark_bands as f32;
    let mut current_band = 1;

    for i in 0..normalised_spectrum.len() {
        while bark_scale[i] > current_band_end {
            bb_limits[current_band] = i as i32;
            current_band += 1;
            current_band_end = (current_band as f32 * bark_scale[normalised_spectrum.len() - 1])
                / number_of_bark_bands as f32;
        }
    }

    bb_limits[number_of_bark_bands] = normalised_spectrum.len() as i32 - 1;

    // Process
    for i in 0..number_of_bark_bands {
        let mut sum = 0.0;
        for j in bb_limits[i]..bb_limits[i + 1] {
            sum += normalised_spectrum[j as usize];
        }

        specific[i] = sum.powf(0.23);
    }

    // Get total loudness
    let total: f32 = specific.iter().sum();

    Ok(BarkLoudness { specific, total })
}

pub fn perceptual_spread_from_loudness(loudness_value: &BarkLoudness) -> Result<f32> {
    // Find the maximum specific loudness
    let max_specific = loudness_value
        .specific
        .iter()
        .cloned()
        .fold(0.0_f32, f32::max);

    // Calculate the spread
    let spread = ((loudness_value.total - max_specific) / loudness_value.total).powi(2);

    Ok(spread)
}

#[allow(dead_code)]
pub fn perceptual_spread(amp_spectrum: &[f32], bark_scale: &[f32]) -> Result<f32> {
    let loudness_value = loudness(amp_spectrum, bark_scale, None)?;

    perceptual_spread_from_loudness(&loudness_value)
}

pub fn perceptual_sharpness_from_loudness(loudness_value: &BarkLoudness) -> Result<f32> {
    let spec = &loudness_value.specific;
    let mut output = 0.0;

    for i in 0..spec.len() {
        if i < 15 {
            output += (i as f32 + 1.0) * spec[i + 1];
        } else {
            output += 0.066 * (0.171 * (i as f32 + 1.0)).exp();
        }
    }

    output *= 0.11 / loudness_value.total;

    Ok(output)
}

#[allow(dead_code)]
pub fn perceptual_sharpness(amp_spectrum: &[f32], bark_scale: &[f32]) -> Result<f32> {
    let loudness_value = loudness(amp_spectrum, bark_scale, None)?;

    perceptual_sharpness_from_loudness(&loudness_value)
}

pub fn power_spectrum(amp_spectrum: &[f32]) -> Vec<f32> {
    if amp_spectrum.is_empty() {
        return [].to_vec();
    }

    let power_spectrum: Vec<f32> = amp_spectrum.iter().map(|&x| x.powi(2)).collect();
    power_spectrum
}

pub fn mel_bands(
    amp_spectrum: &[f32],
    mel_filter_bank: &[Vec<f32>],
    buffer_size: usize,
) -> Result<Vec<f32>> {
    if amp_spectrum.is_empty() {
        bail!("Valid ampSpectrum is required to generate melBands");
    }
    if mel_filter_bank.is_empty() {
        bail!("Valid melFilterBank is required to generate melBands");
    }

    let pow_spec = power_spectrum(amp_spectrum);
    let num_filters = mel_filter_bank.len();
    let mut logged_mel_bands = vec![0.0f32; num_filters];

    for (i, filter) in mel_filter_bank.iter().enumerate() {
        for (j, &value) in filter.iter().enumerate().take(buffer_size / 2) {
            logged_mel_bands[i] += value * pow_spec[j];
        }
        logged_mel_bands[i] = (logged_mel_bands[i] + 1.0).ln();
    }

    Ok(logged_mel_bands)
}

lazy_static! {
    static ref COS_MAP: Mutex<HashMap<usize, Vec<f32>>> = Mutex::new(HashMap::new());
}

fn memoize_cosines(n: usize) {
    let mut cos_map = COS_MAP.lock().unwrap();
    cos_map.entry(n).or_insert_with(|| {
        let pi_n = PI / n as f32;
        let mut cosines = vec![0.0; n * n];

        for k in 0..n {
            for n in 0..n {
                cosines[n + k * n] = (pi_n * (n as f32 + 0.5) * k as f32).cos();
            }
        }

        cosines
    });
}

pub fn dct(signal: &[f32], scale: Option<f32>) -> Vec<f32> {
    let l = signal.len();
    let scale = scale.unwrap_or(2.0);

    {
        let cos_map = COS_MAP.lock().unwrap();
        if !cos_map.contains_key(&l) {
            drop(cos_map); // Release the lock before calling memoize_cosines
            memoize_cosines(l);
        }
    }

    let cos_map = COS_MAP.lock().unwrap();
    let cosines = cos_map.get(&l).unwrap();

    let mut coefficients = vec![0.0; l];

    for (ix, coeff) in coefficients.iter_mut().enumerate() {
        *coeff = scale
            * signal
                .iter()
                .enumerate()
                .fold(0.0, |prev, (ix_, &cur)| prev + cur * cosines[ix_ + ix * l]);
    }

    coefficients
}

pub fn mfcc(
    amp_spectrum: &[f32],
    mel_filter_bank: &[Vec<f32>],
    number_of_mfcc_coefficients: usize,
    buffer_size: usize,
) -> Result<Vec<f32>> {
    let number_of_mfcc_coefficients = number_of_mfcc_coefficients.clamp(1, 40).max(1);

    let num_filters = mel_filter_bank.len();
    if num_filters < number_of_mfcc_coefficients {
        bail!("Insufficient filter bank for requested number of coefficients");
    }

    let logged_mel_bands_array = mel_bands(amp_spectrum, mel_filter_bank, buffer_size)?;
    let mfccs = dct(&logged_mel_bands_array, None);

    Ok(mfccs
        .into_iter()
        .take(number_of_mfcc_coefficients)
        .collect())
}

pub fn create_bark_scale(length: usize, sample_rate: f32, buffer_size: usize) -> Vec<f32> {
    let mut bark_scale = vec![0.0; length];

    for (i, value) in bark_scale.iter_mut().enumerate() {
        let mut val = (i as f32 * sample_rate) / buffer_size as f32;
        val = 13.0 * (val / 1315.8).atan() + 3.5 * ((val / 7518.0).powi(2)).atan();
        *value = val;
    }

    bark_scale
}

pub fn create_mel_filter_bank(
    num_filters: usize,
    sample_rate: f32,
    buffer_size: usize,
) -> Vec<Vec<f32>> {
    let mut mel_values = vec![0.0; num_filters + 2];
    let mut mel_values_in_freq = vec![0.0; num_filters + 2];

    let lower_limit_freq = 0.0;
    let upper_limit_freq = sample_rate / 2.0;

    let lower_limit_mel = freq_to_mel(lower_limit_freq);
    let upper_limit_mel = freq_to_mel(upper_limit_freq);

    let range = upper_limit_mel - lower_limit_mel;
    let value_to_add = range / (num_filters + 1) as f32;

    let mut fft_bins_of_freq = vec![0; num_filters + 2];

    for i in 0..mel_values.len() {
        mel_values[i] = i as f32 * value_to_add;
        mel_values_in_freq[i] = mel_to_freq(mel_values[i]);
        fft_bins_of_freq[i] =
            ((buffer_size + 1) as f32 * mel_values_in_freq[i] / sample_rate).floor() as usize;
    }

    let mut filter_bank = vec![vec![0.0; buffer_size / 2 + 1]; num_filters];

    for j in 0..num_filters {
        for i in fft_bins_of_freq[j]..fft_bins_of_freq[j + 1] {
            filter_bank[j][i] = (i - fft_bins_of_freq[j]) as f32
                / (fft_bins_of_freq[j + 1] - fft_bins_of_freq[j]) as f32;
        }

        for i in fft_bins_of_freq[j + 1]..fft_bins_of_freq[j + 2] {
            filter_bank[j][i] = (fft_bins_of_freq[j + 2] - i) as f32
                / (fft_bins_of_freq[j + 2] - fft_bins_of_freq[j + 1]) as f32;
        }
    }

    filter_bank
}

fn freq_to_mel(freq: f32) -> f32 {
    2595.0 * (1.0 + freq / 700.0).log10()
}

fn mel_to_freq(mel: f32) -> f32 {
    700.0 * (10.0f32.powf(mel / 2595.0) - 1.0)
}
