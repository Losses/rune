use rustfft::{num_complex::Complex, FftPlanner};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::conv::IntoSample;

pub fn process_audio_with_fft(
    file_path: &str,
    fft_size: usize,
    overlap_size: usize,
) -> Vec<Vec<Complex<f32>>> {
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
    let fft = planner.plan_fft_forward(fft_size);
    let mut buffer = vec![Complex::new(0.0, 0.0); fft_size];
    let mut fft_results = Vec::new();

    // Decode loop.
    loop {
        // Get the next packet from the media format.
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::ResetRequired) => unimplemented!(),
            Err(Error::IoError(_)) => break,
            Err(err) => panic!("{}", err),
        };

        // If the packet does not belong to the selected track, skip over it.
        if packet.track_id() != track_id {
            continue;
        }

        // Decode the packet into audio samples.
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::IoError(_)) => continue,
            Err(Error::DecodeError(_)) => continue,
            Err(err) => panic!("{}", err),
        };

        // Macro to handle different AudioBufferRef types
        macro_rules! process_audio_buffer {
            ($buf:expr) => {
                for plane in $buf.planes().planes() {
                    // Process each block using Overlap-Save method.
                    for chunk in plane.windows(fft_size).step_by(fft_size - overlap_size) {
                        // Fill the buffer with the chunk.
                        for (i, &sample) in chunk.iter().enumerate() {
                            buffer[i] = Complex::new(sample.into_sample(), 0.0);
                        }

                        // Perform FFT.
                        fft.process(&mut buffer);

                        // Store the FFT result.
                        fft_results.push(buffer.clone());
                    }
                }
            };
        }

        // Process the decoded audio samples.
        match decoded {
            AudioBufferRef::U8(buf) => process_audio_buffer!(buf),
            AudioBufferRef::U16(buf) => process_audio_buffer!(buf),
            AudioBufferRef::U24(buf) => process_audio_buffer!(buf),
            AudioBufferRef::U32(buf) => process_audio_buffer!(buf),
            AudioBufferRef::S8(buf) => process_audio_buffer!(buf),
            AudioBufferRef::S16(buf) => process_audio_buffer!(buf),
            AudioBufferRef::S24(buf) => process_audio_buffer!(buf),
            AudioBufferRef::S32(buf) => process_audio_buffer!(buf),
            AudioBufferRef::F32(buf) => process_audio_buffer!(buf),
            AudioBufferRef::F64(buf) => process_audio_buffer!(buf),
        }
    }

    fft_results
}
