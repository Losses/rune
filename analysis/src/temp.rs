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

pub fn fft(file_path: &str, window_size: usize, overlap_size: usize) -> AudioDescription {
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
        1, // Assuming mono for simplicity
    )
    .unwrap();

    macro_rules! process_audio_chunk {
        ($chunk:expr, $resampler:expr, $buffer:expr, $avg_spectrum:expr, $hanning_window:expr) => {{
            let resampled_chunk = &$resampler.process(&[$chunk], None).unwrap()[0];

            total_rms += rms(resampled_chunk);
            total_zcr += zcr(resampled_chunk);
            total_energy += energy(resampled_chunk);

            for (i, &sample) in resampled_chunk.iter().enumerate() {
                if i >= window_size {
                    break;
                }
                let windowed_sample = sample * $hanning_window[i];
                $buffer[i] = Complex::new(windowed_sample, 0.0);
            }

            fft.process(&mut $buffer);
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

        // Macro to handle different AudioBufferRef types
        macro_rules! process_audio_buffer {
            ($buf:expr, $buffer:expr) => {
                for plane in $buf.planes().planes() {
                    debug!("Processing plane with len: {}", plane.len());
                    for &sample in plane.iter() {
                        let sample: f32 = IntoSample::<f32>::into_sample(sample);
                        sample_buffer.push(sample);
                        total_samples += 1;

                        // 在主循环中使用 macro
                        while sample_buffer.len() >= actural_data_size {
                            let chunk = &sample_buffer[..actural_data_size];
                            process_audio_chunk!(chunk, resampler, buffer, avg_spectrum, hanning_window);
                            sample_buffer.drain(..(window_size - overlap_size));
                        }

                        // 处理剩余样本
                        if !sample_buffer.is_empty() {
                            while sample_buffer.len() < actural_data_size {
                                sample_buffer.push(0.0);
                            }
                            
                            let chunk = &sample_buffer[..actural_data_size];
                            process_audio_chunk!(chunk, resampler, buffer, avg_spectrum, hanning_window);
                        }
                    }
                }
            };
        }

        match decoded {
            AudioBufferRef::U8(buf) => {
                debug!("Decoded buffer type: U8, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::U16(buf) => {
                debug!("Decoded buffer type: U16, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::U24(buf) => {
                debug!("Decoded buffer type: U24, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::U32(buf) => {
                debug!("Decoded buffer type: U32, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::S8(buf) => {
                debug!("Decoded buffer type: S8, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::S16(buf) => {
                debug!("Decoded buffer type: S16, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::S24(buf) => {
                debug!("Decoded buffer type: S24, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::S32(buf) => {
                debug!("Decoded buffer type: S32, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::F32(buf) => {
                debug!("Decoded buffer type: F32, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
            AudioBufferRef::F64(buf) => {
                debug!("Decoded buffer type: F64, length: {}", buf.frames());
                process_audio_buffer!(buf, buffer);
            }
        }
    }

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