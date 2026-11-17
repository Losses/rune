use std::{
    io::ErrorKind,
    sync::mpsc::{Receiver, Sender, channel},
};

use anyhow::{Context, Result, bail};
use rubato::{FftFixedInOut, Resampler};
use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, Signal},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    conv::IntoSample,
    errors::Error as SymphoniaError,
    formats::FormatReader,
    sample::Sample,
};
use tokio_util::sync::CancellationToken;

use analysis::utils::audio_metadata_reader::{get_codec_information, get_format};
use fsio::FsIo;

pub struct SampleEvent {
    pub sample_index: usize,  // The index of the current sample
    pub total_samples: usize, // Total number of samples
    pub data: Vec<f64>,       // Sample data in floating-point format
    pub sample_rate: u32,     // Sample rate of the audio
    pub duration: f64,        // Duration of the sample in seconds
    pub start_time: f64,      // Start time of the sample in the audio (in seconds)
    pub end_time: f64,        // End time of the sample in the audio (in seconds)
}

pub struct IntervalSampler {
    file_path: String,       // Path to the audio file
    sample_duration: f64,    // Duration of each sample (in seconds)
    interval: f64,           // Time interval between samples (in seconds)
    target_sample_rate: u32, // Target sample rate for resampling

    // Processing state variables
    current_time: f64,               // Current time position in the audio
    current_sample_buffer: Vec<f64>, // Buffer to store current samples
    samples_per_chunk: usize,        // Number of samples per chunk
    next_sample_time: f64,           // Time for the next sample

    // Cancellation flag and function
    fn_is_cancelled: Box<dyn Fn() -> bool + Send>,
    is_cancelled: bool,

    // Channels for sending and receiving sample events
    sender: Option<Sender<SampleEvent>>,
    pub receiver: Receiver<SampleEvent>,
}

impl IntervalSampler {
    pub fn new(
        file_path: &str,
        sample_duration: f64,
        interval: f64,
        target_sample_rate: u32,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        // Create a channel for sending sample events
        let (sender, receiver) = channel::<SampleEvent>();

        IntervalSampler {
            file_path: file_path.to_string(),
            sample_duration,
            interval,
            target_sample_rate,

            current_time: 0.0,
            current_sample_buffer: Vec::new(),
            samples_per_chunk: 0,
            next_sample_time: 0.0,

            fn_is_cancelled: Box::new(move || {
                cancel_token
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
            }),
            is_cancelled: false,

            sender: Some(sender),
            receiver,
        }
    }

    /// Processes the file once and closes the event channel on return.
    ///
    /// The receiver can still drain queued events. Create a new sampler to
    /// process another file or retry after an error or cancellation.
    pub fn process(&mut self, fsio: &FsIo) -> Result<()> {
        let sender = self
            .sender
            .take()
            .context("Sampler has already been used")?;

        // Get the audio format from the file path
        let mut format = get_format(fsio, &self.file_path)?;

        // Find a valid audio track
        let track = match format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        {
            Some(x) => x,
            None => bail!("Failed to find a valid audio track"),
        };

        let track_id = track.id;

        // Get codec information such as sample rate and duration
        let (sample_rate, duration) = get_codec_information(track)?;

        // Calculate the number of samples per chunk based on duration and sample rate
        self.samples_per_chunk = (self.sample_duration * sample_rate as f64) as usize;

        // Initialize decoder with default options
        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = fsio_media_source::get_codecs().make(&track.codec_params, &dec_opts)?;

        // Process the audio stream using the format reader, decoder, and resampler
        self.process_audio_stream(
            &mut format,
            &mut decoder,
            track_id,
            sample_rate,
            duration,
            &sender,
        )?;

        Ok(())
    }

    fn process_audio_buffer<T>(
        &mut self,
        buf: &AudioBuffer<T>,
        sample_rate: u32,
        resampler: &mut FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sample_index: &mut usize,
        sender: &Sender<SampleEvent>,
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        let frames = buf.frames();
        let num_channels = buf.spec().channels.count();
        let frame_duration = 1.0 / sample_rate as f64;

        for frame_idx in 0..frames {
            // Mix down to mono
            let mixed_sample = (0..num_channels)
                .map(|ch| IntoSample::<f64>::into_sample(buf.chan(ch)[frame_idx]))
                .sum::<f64>()
                / num_channels as f64;

            // Store the sample with its exact time
            self.current_sample_buffer.push(mixed_sample);

            // Update current time
            self.current_time += frame_duration;

            // Check if we have enough samples for the current window
            let samples_in_window = (self.sample_duration * sample_rate as f64) as usize;

            if self.current_time >= self.next_sample_time + self.sample_duration {
                // We've passed the current window, process what we have

                // Ensure we have exactly the right number of samples
                if self.current_sample_buffer.len() >= samples_in_window {
                    // Extract exactly samples_in_window samples for processing
                    let chunk: Vec<f64> = self
                        .current_sample_buffer
                        .drain(..samples_in_window)
                        .collect();

                    // Process and send the chunk
                    let input_frames = vec![chunk];
                    resampler.process_into_buffer(&input_frames, output_buffer, None)?;

                    sender.send(SampleEvent {
                        sample_index: *sample_index,
                        total_samples: (self.interval / self.sample_duration) as usize,
                        data: output_buffer[0].clone(),
                        sample_rate: self.target_sample_rate,
                        duration: self.sample_duration,
                        start_time: self.next_sample_time,
                        end_time: self.next_sample_time + self.sample_duration,
                    })?;

                    // Move to next window
                    *sample_index += 1;
                    self.next_sample_time += self.interval;

                    // Keep remaining samples for next window
                    // No need to clear buffer as we used drain()
                }
            }
        }
        Ok(())
    }

    // Process the entire audio stream
    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn FormatReader>,
        decoder: &mut Box<dyn Decoder>,
        track_id: u32,
        sample_rate: u32,
        duration: f64,
        sender: &Sender<SampleEvent>,
    ) -> Result<()> {
        // Create a resampler for converting audio to the target sample rate
        let mut resampler = FftFixedInOut::<f64>::new(
            sample_rate as usize,
            self.target_sample_rate as usize,
            self.samples_per_chunk,
            1,
        )?;

        let mut sample_index = 0;
        let mut output_buffer = resampler.output_buffer_allocate(true);
        let expected_frames = (duration * f64::from(sample_rate)).round() as u64;
        let mut decoded_frames = 0;

        while decoded_frames < expected_frames {
            // Check for cancellation
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return Ok(());
            }

            // Get the next packet from the format reader
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    return Err(error).with_context(|| {
                        format!("audio ended after {decoded_frames} of {expected_frames} declared frames")
                    });
                }
                Err(error) => return Err(error).context("read audio packet"),
            };

            // Skip packets that do not match the track ID
            if packet.track_id() != track_id {
                continue;
            }

            let decoded = decoder.decode(&packet).context("decode audio packet")?;
            decoded_frames += decoded.frames() as u64;

            // Process the decoded audio buffer based on its type
            match decoded {
                AudioBufferRef::U8(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::U16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::U24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::U32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::S8(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::S16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::S24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::S32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::F32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
                AudioBufferRef::F64(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                        sender,
                    )?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use symphonia::core::{
        errors::{Error, Result as SymphoniaResult, end_of_stream_error},
        formats::{Cue, FormatOptions, Packet, SeekMode, SeekTo, SeekedTo, Track},
        io::MediaSourceStream,
        meta::Metadata,
    };

    use super::*;

    struct FirstPacket {
        inner: Box<dyn FormatReader>,
        first: Option<SymphoniaResult<Packet>>,
    }

    impl FormatReader for FirstPacket {
        fn try_new(_: MediaSourceStream, _: &FormatOptions) -> SymphoniaResult<Self> {
            unreachable!("constructed around an existing reader")
        }

        fn cues(&self) -> &[Cue] {
            self.inner.cues()
        }

        fn metadata(&mut self) -> Metadata<'_> {
            self.inner.metadata()
        }

        fn seek(&mut self, mode: SeekMode, to: SeekTo) -> SymphoniaResult<SeekedTo> {
            self.inner.seek(mode, to)
        }

        fn tracks(&self) -> &[Track] {
            self.inner.tracks()
        }

        fn next_packet(&mut self) -> SymphoniaResult<Packet> {
            self.first
                .take()
                .unwrap_or_else(|| self.inner.next_packet())
        }

        fn into_inner(self: Box<Self>) -> MediaSourceStream {
            self.inner.into_inner()
        }
    }

    fn process_first_packet(first: impl FnOnce(u32) -> SymphoniaResult<Packet>) -> Result<()> {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");
        let inner = get_format(&FsIo::new(), file)?;
        let track = inner.default_track().context("fixture audio track")?;
        let track_id = track.id;
        let (sample_rate, duration) = get_codec_information(track)?;
        let mut decoder = fsio_media_source::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;
        let mut format: Box<dyn FormatReader> = Box::new(FirstPacket {
            inner,
            first: Some(first(track_id)),
        });

        let mut sampler = IntervalSampler::new(file, 1.0, 1.0, 16_000, None);
        sampler.samples_per_chunk = sample_rate as usize;
        let (sender, _receiver) = channel();
        sampler.process_audio_stream(
            &mut format,
            &mut decoder,
            track_id,
            sample_rate,
            duration,
            &sender,
        )
    }

    #[test]
    fn packet_eof_before_declared_duration_is_reported() {
        let error = process_first_packet(|_| end_of_stream_error()).unwrap_err();
        assert!(error.to_string().starts_with("audio ended after 0 of "));
    }

    #[test]
    fn packet_read_error_is_reported() {
        let result =
            process_first_packet(|_| Err(Error::IoError(io::ErrorKind::PermissionDenied.into())));
        assert_eq!(result.unwrap_err().to_string(), "read audio packet");
    }

    #[test]
    fn packet_decode_error_is_reported() {
        let result =
            process_first_packet(|track_id| Ok(Packet::new_from_slice(track_id, 0, 1, &[])));
        assert_eq!(result.unwrap_err().to_string(), "decode audio packet");
    }
}
