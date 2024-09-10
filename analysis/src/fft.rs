use log::debug;

use rubato::Resampler;
use rubato::SincFixedIn;
use rubato::SincInterpolationParameters;
use rubato::SincInterpolationType;
use rubato::WindowFunction;
use rustfft::{num_complex::Complex, FftPlanner};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::audio::Signal;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::FormatReader;
use symphonia::core::formats::Track;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::features::energy;
use crate::features::rms;
use crate::features::zcr;

const RESAMPLER_PARAMETER: rubato::SincInterpolationParameters = SincInterpolationParameters {
    sinc_len: 256,
    f_cutoff: 0.95,
    interpolation: SincInterpolationType::Linear,
    oversampling_factor: 256,
    window: WindowFunction::BlackmanHarris2,
};

pub struct AudioDescription {
    pub sample_rate: u32,
    pub duration: f64,
    pub total_samples: usize,
    pub spectrum: Vec<Complex<f32>>,
    pub rms: f32,
    pub zcr: usize,
    pub energy: f32,
}

pub fn build_hanning_window(window_size: usize) -> Vec<f32> {
    (0..window_size)
        .map(|n| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (window_size as f32 - 1.0)).cos())
        })
        .collect()
}

pub fn get_format(file_path: &str) -> Result<Box<dyn FormatReader>, Error> {
    // Open the media source.
    let src = std::fs::File::open(file_path).expect("failed to open media");

    // Create the media source stream.
    let mss = MediaSourceStream::new(Box::new(src), Default::default());

    // Create a probe hint using the file's extension.
    let mut hint = Hint::new();
    let ext = file_path.split('.').last().unwrap_or_default();
    hint.with_extension(ext);

    // Use the default options for metadata and format readers.
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    // Probe the media source.
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .expect("unsupported format");

    // Get the instantiated format reader.
    let format = probed.format;

    Ok(format)
}

pub fn get_codec_information(track: &Track) -> Result<(u32, f64), symphonia::core::errors::Error> {
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| symphonia::core::errors::Error::Unsupported("No sample rate found"))?;
    let duration = track
        .codec_params
        .n_frames
        .ok_or_else(|| symphonia::core::errors::Error::Unsupported("No duration found"))?;

    let time_base = track
        .codec_params
        .time_base
        .unwrap_or_else(|| symphonia::core::units::TimeBase::new(1, sample_rate));
    let duration_in_seconds =
        time_base.calc_time(duration).seconds as f64 + time_base.calc_time(duration).frac;

    Ok((sample_rate, duration_in_seconds))
}

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
            ($buf:expr) => {
                for plane in $buf.planes().planes() {
                    debug!("Processing plane with len: {}", plane.len());
                    for &sample in plane.iter() {
                        let sample: f32 = IntoSample::<f32>::into_sample(sample);
                        sample_buffer.push(sample);
                        total_samples += 1;

                        // Process the buffer when it reaches the window size
                        while sample_buffer.len() >= actural_data_size {
                            let chunk = &sample_buffer[..actural_data_size];
                            let resampled_chunk = &resampler.process(&[chunk], None).unwrap()[0];

                            total_rms += rms(resampled_chunk);
                            total_zcr += zcr(resampled_chunk);
                            total_energy += energy(resampled_chunk);

                            for (i, &sample) in resampled_chunk.iter().enumerate() {
                                if i >= window_size {
                                    break;
                                }

                                let windowed_sample = sample * hanning_window[i];
                                buffer[i] = Complex::new(windowed_sample, 0.0);
                            }

                            fft.process(&mut buffer);
                            debug!("FFT processed");

                            for (i, value) in buffer.iter().enumerate() {
                                avg_spectrum[i] += value;
                            }

                            count += 1;

                            // Remove the processed samples, keeping the overlap
                            sample_buffer.drain(..(window_size - overlap_size));
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
