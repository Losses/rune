use log::debug;
use rustfft::{num_complex::Complex, FftPlanner};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::audio::Signal;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub fn fft(file_path: &str, window_size: usize, overlap_size: usize) -> Vec<Complex<f32>> {
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
    let mut format = probed.format;

    // Find the first audio track with a known (decodeable) codec.
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .expect("no supported audio tracks");

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

    // Precompute Hanning window
    let hanning_window: Vec<f32> = (0..window_size)
        .map(|n| {
            0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (window_size as f32 - 1.0)).cos())
        })
        .collect();

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
                    for chunk in plane
                        .windows(window_size)
                        .step_by(window_size - overlap_size)
                    {
                        debug!("Processing chunk with len: {}", chunk.len());
                        for (i, &sample) in chunk.iter().enumerate() {
                            let windowed_sample =
                                IntoSample::<f32>::into_sample(sample) * hanning_window[i];
                            buffer[i] = Complex::new(windowed_sample, 0.0);
                        }

                        fft.process(&mut buffer);
                        debug!("FFT processed");

                        for (i, value) in buffer.iter().enumerate() {
                            avg_spectrum[i] += value;
                        }

                        count += 1;
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

    avg_spectrum
}
