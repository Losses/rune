use anyhow::Result;
use tokio_util::sync::CancellationToken;

use fsio::FsIo;

use crate::{
    analyzer::core_analyzer::Analyzer,
    measure_time,
    utils::{computing_device::ComputingDevice, features::*},
};

#[derive(Debug, Clone, Copy)]
pub struct AudioStat {
    pub sample_rate: u32,
    pub duration: f64,
    pub total_samples: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct AnalysisParameter {
    pub window_size: usize,
    pub overlap_size: usize,
}

#[derive(Debug, Clone, Copy)]
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
    pub chromagram: [f32; 12],
    pub perceptual_spread: f32,
    pub perceptual_sharpness: f32,
    pub perceptual_loudness: [f32; 24],
    pub mfcc: [f32; 13],
}

pub fn analyze_audio(
    fsio: &FsIo,
    file_path: &str,
    window_size: usize,
    overlap_size: usize,
    computing_device: ComputingDevice,
    cancel_token: Option<CancellationToken>,
) -> Result<Option<AnalysisResult>> {
    let mut analyzer = Analyzer::new(
        computing_device,
        window_size,
        overlap_size,
        None,
        cancel_token,
    );

    let audio_desc = measure_time!(
        &format!("[{computing_device:?}] Analyzer"),
        analyzer.process(fsio, file_path)
    );

    if audio_desc.is_none() {
        return Ok(None);
    };

    let audio_desc = audio_desc.expect("Audio desc should not be none");

    let amp_spectrum = amp_spectrum(&audio_desc.spectrum, window_size);

    // Calculate spectral features
    let spectral_centroid = spectral_centroid(&amp_spectrum);
    let spectral_flatness = spectral_flatness(&amp_spectrum);
    let spectral_flux = spectral_flux(&amp_spectrum, &vec![0.0; amp_spectrum.len()], window_size);
    let spectral_slope = spectral_slope(&amp_spectrum, audio_desc.sample_rate as f32, window_size);
    let spectral_rolloff = spectral_rolloff(&amp_spectrum, audio_desc.sample_rate as f32);
    let spectral_spread = spectral_spread(&amp_spectrum);
    let spectral_skewness = spectral_skewness(&amp_spectrum);
    let spectral_kurtosis = spectral_kurtosis(&amp_spectrum);

    // Generate chroma filter bank and calculate chromagram
    let chroma_filter_bank = create_chroma_filter_bank(
        12,      // Number of chroma bins
        11025.0, // Sample Rate
        window_size,
        5.0,   // Center octave
        2.0,   // Octave width
        true,  // Base C
        440.0, // A440
    );
    let chromagram: [f32; 12] = chroma(&amp_spectrum, &chroma_filter_bank)
        .try_into()
        .expect("Expected a Vec of length 12");

    let bark_scale = create_bark_scale(amp_spectrum.len(), 11025.0, amp_spectrum.len());
    let loudness = loudness(&amp_spectrum, &bark_scale, None)?;
    let perceptual_spread = perceptual_spread_from_loudness(&loudness)?;
    let perceptual_sharpness = perceptual_sharpness_from_loudness(&loudness)?;
    let perceptual_loudness: [f32; 24] = loudness
        .specific
        .try_into()
        .expect("Expected a Vec of length 24");

    let mel_filter_bank = create_mel_filter_bank(13, 11025.0, window_size);
    let mfcc: [f32; 13] = mfcc(&amp_spectrum, &mel_filter_bank, 13, window_size)?
        .try_into()
        .expect("Expected a Vec of length 13");

    // Create and return the analysis result
    Ok(Some(AnalysisResult {
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
        perceptual_loudness,
        perceptual_spread,
        perceptual_sharpness,
        mfcc,
    }))
}

#[derive(Debug, Clone, Copy)]
pub struct NormalizedAnalysisResult {
    pub stat: AudioStat,
    pub parameters: AnalysisParameter,
    pub raw: AnalysisResult,
    pub zcr: f32,
    pub energy: f32,
    pub spectral_centroid: f32,
    pub spectral_flatness: f32,
    pub spectral_flux: f32,
    pub spectral_slope: f32,
    pub spectral_rolloff: f32,
    pub spectral_spread: f32,
    pub spectral_skewness: f32,
    pub spectral_kurtosis: f32,
    pub chroma: [f32; 12],
}

pub fn normalize_analysis_result(result: &AnalysisResult) -> NormalizedAnalysisResult {
    // Define the ranges for each feature
    let max_spectral_centroid = (result.parameters.window_size / 2) as f32;
    let max_spectral_flatness = 1.0;
    let max_spectral_flux = 1.0;
    let max_spectral_slope = 1.0;
    let max_spectral_rolloff = (result.stat.sample_rate / 2) as f32;
    let max_spectral_spread = (result.parameters.window_size / 2) as f32;
    let max_spectral_skewness = 1.0;
    let max_spectral_kurtosis = 1.0;
    let max_chroma = 1.0;

    // Normalize each feature
    let normalized_zcr = result.zcr as f32 / ((result.stat.total_samples / 2) - 1) as f32;
    let normalized_energy = result.energy / result.stat.total_samples as f32;
    let normalized_spectral_centroid = result.spectral_centroid / max_spectral_centroid;
    let normalized_spectral_flatness = result.spectral_flatness / max_spectral_flatness;
    let normalized_spectral_flux = result.spectral_flux / max_spectral_flux;
    let normalized_spectral_slope = result.spectral_slope / max_spectral_slope;
    let normalized_spectral_rolloff = result.spectral_rolloff / max_spectral_rolloff;
    let normalized_spectral_spread = result.spectral_spread / max_spectral_spread;
    let normalized_spectral_skewness = result.spectral_skewness / max_spectral_skewness;
    let normalized_spectral_kurtosis = result.spectral_kurtosis / max_spectral_kurtosis;

    // Normalize chromagram
    let normalized_chroma: [f32; 12] = result
        .chromagram
        .iter()
        .map(|&x| x / max_chroma)
        .collect::<Vec<_>>()
        .try_into()
        .expect("Expected a Vec of length 12");

    // Create and return the normalized analysis result
    NormalizedAnalysisResult {
        stat: result.stat,
        parameters: result.parameters,
        raw: *result,
        zcr: normalized_zcr,
        energy: normalized_energy,
        spectral_centroid: normalized_spectral_centroid,
        spectral_flatness: normalized_spectral_flatness,
        spectral_flux: normalized_spectral_flux,
        spectral_slope: normalized_spectral_slope,
        spectral_rolloff: normalized_spectral_rolloff,
        spectral_spread: normalized_spectral_spread,
        spectral_skewness: normalized_spectral_skewness,
        spectral_kurtosis: normalized_spectral_kurtosis,
        chroma: normalized_chroma,
    }
}
