use fsio::FsIo;
use log::debug;

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use rustfft::{FftPlanner, num_complex::Complex};
use symphonia::core::{
    audio::{AudioBufferRef, Signal},
    codecs::{CODEC_TYPE_NULL, DecoderOptions},
    conv::IntoSample,
    errors::Error,
};
use tokio_util::sync::CancellationToken;

use crate::utils::{
    audio_description::AudioDescription,
    audio_metadata_reader::*,
    features::{energy, rms, zcr},
    hanning_window::build_hanning_window,
};

// Define the macro at the beginning of the file, after the imports
macro_rules! process_window {
    ($sample_buffer:expr, $actural_data_size:expr, $window_size:expr, $overlap_size:expr, $resampler:expr, $hanning_window:expr, $buffer:expr, $fft:expr, $avg_spectrum:expr, $total_rms:expr, $total_zcr:expr, $total_energy:expr, $count:expr, $is_cancelled:expr) => {
        // Check for cancellation before processing each window
        if $is_cancelled() {
            return None;
        }

        let chunk = &$sample_buffer[..$actural_data_size];
        debug!("Processing chunk of length: {}", chunk.len());
        let resampled_chunk = &$resampler.process(&[chunk], None).unwrap()[0];

        $total_rms += rms(resampled_chunk);
        $total_zcr += zcr(resampled_chunk);
        $total_energy += energy(resampled_chunk);

        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= $window_size {
                break;
            }

            let windowed_sample = sample * $hanning_window[i];
            $buffer[i] = Complex::new(windowed_sample, 0.0);
        }

        $fft.process(&mut $buffer);
        debug!("FFT processed");

        for (i, value) in $buffer.iter().enumerate() {
            $avg_spectrum[i] += value;
        }

        $count += 1;

        // Remove the processed samples, keeping the overlap
        $sample_buffer.drain(..($window_size - $overlap_size));
    };
}

#[allow(dead_code)]
pub const RESAMPLER_PARAMETER: rubato::SincInterpolationParameters = SincInterpolationParameters {
    sinc_len: 256,
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    oversampling_factor: 256,
    window: WindowFunction::BlackmanHarris2,
};

#[allow(dead_code)]
pub fn fft(
    fsio: &FsIo,
    file_path: &str,
    window_size: usize,
    overlap_size: usize,
    cancel_token: Option<CancellationToken>,
) -> Option<AudioDescription> {
    // Helper function to check cancellation
    let is_cancelled = || {
        cancel_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
    };

    // Get the audio track.
    let mut format = get_format(fsio, file_path).expect("no supported audio tracks");
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("No supported audio tracks");

    // Get codec information.
    let (sample_rate, duration_in_seconds) = get_codec_information(track).unwrap();

    // Use the default options for the decoder.
    let dec_opts: DecoderOptions = Default::default();

    // Create a decoder for the track.
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .expect("unsupported codec");

    // Store the track identifier, it will be used to filter packets.
    let track_id = track.id;

    // Prepare FFT planner and buffers.
    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(window_size);
    let mut buffer = vec![Complex::new(0.0, 0.0); window_size];
    let mut avg_spectrum = vec![Complex::new(0.0, 0.0); window_size];
    let mut count = 0;
    let mut total_samples = 0;
    let mut total_rms = 0 as f32;
    let mut total_zcr = 0;
    let mut total_energy = 0 as f32;

    // Precompute Hanning window
    let hanning_window: Vec<f32> = build_hanning_window(window_size);

    // Buffer to hold audio samples until we have enough for one window
    let mut sample_buffer: Vec<f32> = Vec::new();

    let resample_ratio = 11025_f64 / sample_rate as f64;

    let actural_data_size = (window_size as f64 / resample_ratio).ceil() as usize;

    // Initialize the resampler
    let mut resampler = SincFixedIn::<f32>::new(
        resample_ratio,
        2.0,
        RESAMPLER_PARAMETER,
        actural_data_size,
        1,
    )
    .unwrap();

    // Decode loop.
    loop {
        // Check for cancellation
        if is_cancelled() {
            return None;
        }

        // Get the next packet from the media format.
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::ResetRequired) => unimplemented!(),
            Err(Error::IoError(_)) => {
                debug!("End of stream");
                break;
            }
            Err(err) => panic!("{}", err),
        };
        debug!("Packet received: track_id = {}", packet.track_id());

        // If the packet does not belong to the selected track, skip over it.
        if packet.track_id() != track_id {
            continue;
        }

        // Decode the packet into audio samples.
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::IoError(_)) => {
                debug!("IO Error while decoding");
                continue;
            }
            Err(Error::DecodeError(_)) => {
                debug!("Decode Error");
                continue;
            }
            Err(err) => panic!("{}", err),
        };
        debug!("Packet decoded successfully");

        // Macro to handle different AudioBufferRef types
        macro_rules! process_audio_buffer {
            ($buf:expr) => {
                for plane in $buf.planes().planes() {
                    // Check for cancellation inside the buffer processing loop
                    if is_cancelled() {
                        return None;
                    }

                    debug!("Processing plane with len: {}", plane.len());
                    for &sample in plane.iter() {
                        let sample: f32 = IntoSample::<f32>::into_sample(sample);
                        sample_buffer.push(sample);
                        total_samples += 1;

                        // Process the buffer when it reaches the window size
                        while sample_buffer.len() >= actural_data_size {
                            process_window!(
                                sample_buffer,
                                actural_data_size,
                                window_size,
                                overlap_size,
                                resampler,
                                hanning_window,
                                buffer,
                                fft,
                                avg_spectrum,
                                total_rms,
                                total_zcr,
                                total_energy,
                                count,
                                is_cancelled
                            );
                        }
                    }
                }
            };
        }

        match decoded {
            AudioBufferRef::U8(buf) => {
                debug!("Decoded buffer type: U8, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::U16(buf) => {
                debug!("Decoded buffer type: U16, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::U24(buf) => {
                debug!("Decoded buffer type: U24, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::U32(buf) => {
                debug!("Decoded buffer type: U32, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::S8(buf) => {
                debug!("Decoded buffer type: S8, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::S16(buf) => {
                debug!("Decoded buffer type: S16, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::S24(buf) => {
                debug!("Decoded buffer type: S24, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::S32(buf) => {
                debug!("Decoded buffer type: S32, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::F32(buf) => {
                debug!("Decoded buffer type: F32, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
            AudioBufferRef::F64(buf) => {
                debug!("Decoded buffer type: F64, length: {}", buf.frames());
                process_audio_buffer!(buf);
            }
        }
    }

    if count == 0 {
        panic!("No audio data processed");
    }

    // Final cancellation check before returning results
    if is_cancelled() {
        return None;
    }

    // Calculate the final average spectrum.
    for value in avg_spectrum.iter_mut() {
        *value /= count as f32;
    }
    debug!("Final average spectrum calculated");

    debug!("Total samples: {total_samples}");

    Some(AudioDescription {
        sample_rate,
        duration: duration_in_seconds,
        total_samples,
        spectrum: avg_spectrum,
        rms: total_rms / count as f32,
        zcr: total_zcr / count,
        energy: total_energy / count as f32,
    })
}
