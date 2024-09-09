use crate::features::*;
use crate::fft::*;

#[derive(Debug)]
pub struct AudioStat {
    pub sample_rate: u32,
    pub duration: f64,
    pub total_samples: usize,
}

#[derive(Debug)]
pub struct AnalysisParameter {
    pub window_size: usize,
    pub overlap_size: usize,
}

#[derive(Debug)]
pub struct AnalysisResult {
    pub stat: AudioStat,
    pub parameters: AnalysisParameter,
    pub rms: f32,
    pub zcr: usize,
    pub energy: f32,
    pub spectral_centroid: f32,
    pub spectral_flatness: f32,
    pub spectral_flux: f32,
    pub spectral_slope: f32,
    pub spectral_rolloff: f32,
    pub spectral_spread: f32,
    pub spectral_skewness: f32,
    pub spectral_kurtosis: f32,
    pub chromagram: Vec<f32>,
}

pub fn analyze_audio(file_path: &str, window_size: usize, overlap_size: usize) -> AnalysisResult {
    // Perform FFT on the audio file to get the spectrum
    let audio_desc = fft(file_path, window_size, overlap_size);

    let amp_spectrum = amp_spectrum(&audio_desc.spectrum, window_size);

    // Calculate spectral features
    let spectral_centroid = spectral_centroid(&amp_spectrum);
    let spectral_flatness = spectral_flatness(&amp_spectrum);
    let spectral_flux = spectral_flux(&amp_spectrum, &vec![0.0; amp_spectrum.len()], window_size); // Assuming previous signal is zero
    let spectral_slope = spectral_slope(&amp_spectrum, audio_desc.sample_rate as f32, window_size);
    let spectral_rolloff = spectral_rolloff(&amp_spectrum, audio_desc.sample_rate as f32);
    let spectral_spread = spectral_spread(&amp_spectrum);
    let spectral_skewness = spectral_skewness(&amp_spectrum);
    let spectral_kurtosis = spectral_kurtosis(&amp_spectrum);

    // Generate chroma filter bank and calculate chromagram
    let chroma_filter_bank = create_chroma_filter_bank(
        12, // Number of chroma bins
        audio_desc.sample_rate as f32,
        window_size,
        5.0,   // Center octave
        2.0,   // Octave width
        true,  // Base C
        440.0, // A440
    );
    let chromagram = chroma(&amp_spectrum, &chroma_filter_bank);

    // Create and return the analysis result
    AnalysisResult {
        stat: AudioStat {
            sample_rate: audio_desc.sample_rate,
            duration: audio_desc.duration,
            total_samples: audio_desc.total_samples,
        },
        parameters: AnalysisParameter {
            window_size,
            overlap_size,
        },
        rms: audio_desc.rms,
        zcr: audio_desc.zcr,
        energy: audio_desc.energy,
        spectral_centroid,
        spectral_flatness,
        spectral_flux,
        spectral_slope,
        spectral_rolloff,
        spectral_spread,
        spectral_skewness,
        spectral_kurtosis,
        chromagram,
    }
}

#[derive(Debug)]
pub struct NormalizedAnalysisResult {
    pub stat: AudioStat,
    pub parameters: AnalysisParameter,
    pub spectral_centroid: f32,
    pub spectral_flatness: f32,
    pub spectral_flux: f32,
    pub spectral_slope: f32,
    pub spectral_rolloff: f32,
    pub spectral_spread: f32,
    pub spectral_skewness: f32,
    pub spectral_kurtosis: f32,
    pub chromagram: Vec<f32>,
}

pub fn normalize_analysis_result(result: AnalysisResult) -> NormalizedAnalysisResult {
    // Define the ranges for each feature
    let max_spectral_centroid = (result.parameters.window_size / 2) as f32;
    let max_spectral_flatness = 1.0;
    let max_spectral_flux = 1.0; // Assuming a reasonable upper bound for normalization purposes
    let max_spectral_slope = 1.0;
    let max_spectral_rolloff = (result.stat.sample_rate / 2) as f32;
    let max_spectral_spread = (result.parameters.window_size / 2) as f32;
    let max_spectral_skewness = 1.0; // Assuming a reasonable upper bound for normalization purposes
    let max_spectral_kurtosis = 1.0;
    let max_chroma = 1.0;

    // Normalize each feature
    let normalized_spectral_centroid = result.spectral_centroid / max_spectral_centroid;
    let normalized_spectral_flatness = result.spectral_flatness / max_spectral_flatness;
    let normalized_spectral_flux = result.spectral_flux / max_spectral_flux;
    let normalized_spectral_slope = result.spectral_slope / max_spectral_slope;
    let normalized_spectral_rolloff = result.spectral_rolloff / max_spectral_rolloff;
    let normalized_spectral_spread = result.spectral_spread / max_spectral_spread;
    let normalized_spectral_skewness = result.spectral_skewness / max_spectral_skewness;
    let normalized_spectral_kurtosis = result.spectral_kurtosis / max_spectral_kurtosis;

    // Normalize chromagram
    let normalized_chromagram: Vec<f32> =
        result.chromagram.iter().map(|&x| x / max_chroma).collect();

    // Create and return the normalized analysis result
    NormalizedAnalysisResult {
        stat: result.stat,
        parameters: result.parameters,
        spectral_centroid: normalized_spectral_centroid,
        spectral_flatness: normalized_spectral_flatness,
        spectral_flux: normalized_spectral_flux,
        spectral_slope: normalized_spectral_slope,
        spectral_rolloff: normalized_spectral_rolloff,
        spectral_spread: normalized_spectral_spread,
        spectral_skewness: normalized_spectral_skewness,
        spectral_kurtosis: normalized_spectral_kurtosis,
        chromagram: normalized_chromagram,
    }
}
