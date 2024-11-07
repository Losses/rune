use log::{debug, info};

use rubato::Resampler;
use rubato::SincFixedIn;
use rustfft::{num_complex::Complex, FftPlanner};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::audio::Signal;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error;

use crate::features::energy;
use crate::features::rms;
use crate::features::zcr;

use crate::fft_utils::*;
use crate::measure_time;
use crate::wgpu_fft::wgpu_radix4;

pub struct FFTProcessor {
    window_size: usize,
    batch_size: usize,
    overlap_size: usize,
    sample_rate: u32,
    fft: wgpu_radix4::FFTCompute,
    buffer: Vec<Complex<f32>>,
    avg_spectrum: Vec<Complex<f32>>,
    hanning_window: Vec<f32>,
    sample_buffer: Vec<f32>,
    count: usize,
    total_samples: usize,
    total_rms: f32,
    total_zcr: usize,
    total_energy: f32,
}

pub fn fft(
    file_path: &str,
    window_size: usize,
    batch_size: usize,
    overlap_size: usize,
) -> AudioDescription {
    // Get the audio track.
    let mut format = get_format(file_path).expect("no supported audio tracks");
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

    let fft = pollster::block_on(wgpu_radix4::FFTCompute::new(window_size * batch_size));
    // let mut planner = FftPlanner::new();
    // let fft = planner.plan_fft_forward(window_size);
    let mut buffer = vec![Complex::new(0.0, 0.0); window_size * batch_size];
    let mut avg_spectrum = vec![Complex::new(0.0, 0.0); window_size * batch_size];
    let mut count = 0;
    let mut total_samples = 0;
    let mut total_rms = 0 as f32;
    let mut total_zcr = 0;
    let mut total_energy = 0 as f32;

    // Precompute Hanning window
    let hanning_window: Vec<f32> = build_hanning_window(window_size * batch_size);

    // Buffer to hold audio samples until we have enough for one window
    let mut sample_buffer: Vec<f32> = Vec::with_capacity(window_size * batch_size);

    let resample_ratio = 11025_f64 / sample_rate as f64;

    let actural_data_size = (window_size as f64 / resample_ratio).ceil() as usize;
    let actual_batch_size = actural_data_size * batch_size;

    // Macro to handle different AudioBufferRef types
    macro_rules! process_audio_chunk {
        ($chunk:expr, $resampler:expr, $buffer:expr, $avg_spectrum:expr, $hanning_window:expr, $fft:expr) => {{
            let chunk_size = $chunk.len();
            let mut resampler = if chunk_size != actural_data_size {
                // Create a new resampler with the current chunk size
                SincFixedIn::<f32>::new(resample_ratio, 2.0, RESAMPLER_PARAMETER, chunk_size, 1)
                    .unwrap()
            } else {
                SincFixedIn::<f32>::new(
                    resample_ratio,
                    2.0,
                    RESAMPLER_PARAMETER,
                    actural_data_size,
                    1,
                )
                .unwrap()
            };

            let resampled_chunk = &resampler.process(&[$chunk], None).unwrap()[0];

            total_rms += rms(&resampled_chunk);
            total_zcr += zcr(&resampled_chunk);
            total_energy += energy(&resampled_chunk);

            for (i, &sample) in resampled_chunk.iter().enumerate() {
                if i >= window_size * batch_size {
                    break;
                }
                let windowed_sample = sample * $hanning_window[i];
                $buffer[i] = Complex::new(windowed_sample, 0.0);
            }

            pollster::block_on($fft.compute_fft(&mut $buffer));
            debug!("FFT processed");

            for (i, value) in $buffer.iter().enumerate() {
                $avg_spectrum[i] += value;
            }

            count += 1;
        }};
    }

    // Decode loop.
    loop {
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

        macro_rules! process_audio_buffer {
            ($buf:expr) => {
                for plane in $buf.planes().planes() {
                    debug!("Processing plane with len: {}", plane.len());
                    for &sample in plane.iter() {
                        let sample: f32 = IntoSample::<f32>::into_sample(sample);
                        sample_buffer.push(sample);
                        total_samples += 1;

                        while sample_buffer.len() >= actural_data_size {
                            let chunk = &sample_buffer[..actural_data_size];
                            process_audio_chunk!(
                                chunk,
                                resampler,
                                buffer,
                                avg_spectrum,
                                hanning_window,
                                fft
                            );
                            println!("sample_buffer len 1: {}", sample_buffer.len());
                            sample_buffer.drain(..(window_size * batch_size - overlap_size));
                            println!("sample_buffer len 2: {}", sample_buffer.len());
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

    if !sample_buffer.is_empty() {
        // Pad to the nearest multiple of 1024
        let target_size = ((total_samples + 1023) / 1024) * 1024;
        while sample_buffer.len() < target_size {
            sample_buffer.push(0.0);
        }

        // Only process up to target_size
        let chunk = &sample_buffer[..target_size.min(actural_data_size)];
        println!("Chunk length: {}", chunk.len());
        process_audio_chunk!(chunk, resampler, buffer, avg_spectrum, hanning_window, fft);
    }

    info!("Total samples: {}", total_samples);

    if count == 0 {
        panic!("No audio data processed");
    }

    // Calculate the final average spectrum.
    for value in avg_spectrum.iter_mut() {
        *value /= count as f32;
    }
    debug!("Final average spectrum calculated");

    AudioDescription {
        sample_rate,
        duration: duration_in_seconds,
        total_samples,
        spectrum: avg_spectrum,
        rms: total_rms / count as f32,
        zcr: total_zcr / count,
        energy: total_energy / count as f32,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::fft;

    use super::*;

    #[test]
    fn test_fft_startup_sound() {
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let batch_size = 4;
        let overlap_size = 512;

        let result = fft(file_path, window_size, batch_size, overlap_size);

        // let result = result.expect("Result should not be none");
        assert!(result.duration > 0.0, "Duration should be positive");
        assert!(result.sample_rate > 0, "Sample rate should be positive");
        assert!(
            result.total_samples > 0,
            "Should have processed some samples"
        );

        assert_eq!(
            result.spectrum.len(),
            window_size,
            "Spectrum length should match window size"
        );

        assert!(result.rms > 0.0, "RMS should be positive");
        assert!(result.energy > 0.0, "Energy should be positive");
        assert!(result.zcr >= 0, "ZCR should be non-negative");

        println!("Audio Analysis Results:");
        println!("{:?}", result.duration);
    }
}
