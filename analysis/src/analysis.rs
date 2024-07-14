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
