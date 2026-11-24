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
use std::iter;

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
    current_frame: u64,              // Index of the next decoded audio frame
    current_sample_buffer: Vec<f64>, // Buffer to store current samples
    sample_index: usize,             // Index of the next sampling window

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

            current_frame: 0,
            current_sample_buffer: Vec::new(),
            sample_index: 0,

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

    fn window_at(&self, index: usize, sample_rate: u32) -> SampleWindow {
        let start_time = index as f64 * self.interval;
        let end_time = if self.sample_duration == self.interval {
            (index + 1) as f64 * self.interval
        } else {
            start_time + self.sample_duration
        };
        let first_frame_at_or_after = |time: f64| {
            let frame = time * f64::from(sample_rate);
            let nearest = frame.round();
            // Keep an integer boundary stable through floating-point time arithmetic.
            if (frame - nearest).abs() <= 4.0 * f64::EPSILON * frame {
                nearest as u64
            } else {
                frame.ceil() as u64
            }
        };
        SampleWindow {
            start_time,
            end_time,
            start_frame: first_frame_at_or_after(start_time),
            end_frame: first_frame_at_or_after(end_time),
        }
    }

    fn process_audio_buffer<T>(
        &mut self,
        buf: &AudioBuffer<T>,
        sample_rate: u32,
        resample: &mut impl FnMut(&[f64]) -> Result<Vec<f64>>,
        sender: &Sender<SampleEvent>,
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        let num_channels = buf.spec().channels.count();
        let mut window = self.window_at(self.sample_index, sample_rate);

        for frame_idx in 0..buf.frames() {
            let source_frame = self.current_frame;
            self.current_frame += 1;
            if source_frame < window.start_frame {
                continue;
            }

            let mixed_sample = (0..num_channels)
                .map(|ch| IntoSample::<f64>::into_sample(buf.chan(ch)[frame_idx]))
                .sum::<f64>()
                / num_channels as f64;
            self.current_sample_buffer.push(mixed_sample);

            while self.current_frame >= window.end_frame {
                let len = (window.end_frame - window.start_frame) as usize;
                sender.send(SampleEvent {
                    sample_index: self.sample_index,
                    total_samples: (self.interval / self.sample_duration) as usize,
                    data: resample(&self.current_sample_buffer[..len])?,
                    sample_rate: self.target_sample_rate,
                    duration: self.sample_duration,
                    start_time: window.start_time,
                    end_time: window.end_time,
                })?;

                self.sample_index += 1;
                let next = self.window_at(self.sample_index, sample_rate);
                // Keep any source frames that also belong to the next window.
                let advance = (next.start_frame - window.start_frame) as usize;
                self.current_sample_buffer
                    .drain(..advance.min(self.current_sample_buffer.len()));
                window = next;
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
        let source_frames = self.sample_duration * f64::from(sample_rate);
        if !source_frames.is_finite() || source_frames < 1.0 {
            bail!("Sample duration must span at least one source frame");
        }
        if !self.interval.is_finite() || self.interval <= 0.0 {
            bail!("Sampling interval must be finite and positive");
        }

        let target_rate = self.target_sample_rate;
        let mut resampler = (sample_rate != target_rate)
            .then(|| {
                FftFixedInOut::<f64>::new(
                    sample_rate as usize,
                    target_rate as usize,
                    source_frames.ceil() as usize,
                    1,
                )
            })
            .transpose()?
            .map(|resampler| {
                let output = resampler.output_buffer_allocate(true);
                (resampler, output)
            });
        let mut resample = |samples: &[f64]| match &mut resampler {
            Some((resampler, output)) => {
                resample_window(samples, sample_rate, target_rate, resampler, output)
            }
            None => Ok(samples.to_vec()),
        };

        let duration_frames = duration * f64::from(sample_rate);
        while (self.current_frame as f64) < duration_frames {
            // Check for cancellation
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return Ok(());
            }

            // Get the next packet from the format reader
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(SymphoniaError::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    // The metadata may count codec pre-skip or estimate the duration.
                    break;
                }
                Err(error) => return Err(error).context("read audio packet"),
            };

            // Skip packets that do not match the track ID
            if packet.track_id() != track_id {
                continue;
            }

            let decoded = decoder.decode(&packet).context("decode audio packet")?;

            // Process the decoded audio buffer based on its type
            match decoded {
                AudioBufferRef::U8(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::U16(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::U24(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::U32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::S8(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::S16(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::S24(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::S32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::F32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
                AudioBufferRef::F64(buf) => {
                    self.process_audio_buffer(buf.as_ref(), sample_rate, &mut resample, sender)?;
                }
            }
        }

        Ok(())
    }
}

struct SampleWindow {
    start_time: f64,
    end_time: f64,
    start_frame: u64,
    end_frame: u64,
}

fn resample_window(
    samples: &[f64],
    source_rate: u32,
    target_rate: u32,
    resampler: &mut FftFixedInOut<f64>,
    output: &mut [Vec<f64>],
) -> Result<Vec<f64>> {
    resampler.reset();
    let block_len = resampler.input_frames_next();
    let expected =
        (samples.len() as u64 * u64::from(target_rate)).div_ceil(u64::from(source_rate)) as usize;
    let mut delay = resampler.output_delay();
    let mut data = Vec::with_capacity(expected);

    // Flush the FFT overlap with zero input so the window's final samples are available.
    for block in samples.chunks(block_len).chain(iter::once(&[][..])) {
        let (_, written) = if block.len() == block_len {
            resampler.process_into_buffer(&[block], output, None)?
        } else {
            let channel = [block];
            let input = (!block.is_empty()).then_some(channel.as_slice());
            resampler.process_partial_into_buffer(input, output, None)?
        };
        let start = delay.min(written);
        delay -= start;
        let end = (start + expected - data.len()).min(written);
        data.extend_from_slice(&output[0][start..end]);
        if data.len() == expected {
            return Ok(data);
        }
    }
    bail!(
        "Resampler produced {} samples, expected {expected}",
        data.len()
    );
}

#[cfg(test)]
mod tests {
    use std::io;

    use symphonia::core::{
        audio::{Channels, SignalSpec},
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
    fn gap_frames_are_discarded_before_buffering() -> Result<()> {
        let rate = 16;
        let mut sampler = IntervalSampler::new("", 1.0, 4.0, rate, None);
        let (sender, receiver) = channel();
        let mut audio = AudioBuffer::<f64>::new(
            2 * u64::from(rate),
            SignalSpec::new(rate, Channels::FRONT_LEFT),
        );
        audio.render_silence(None);

        sampler.process_audio_buffer(&audio, rate, &mut |samples| Ok(samples.to_vec()), &sender)?;

        assert_eq!(receiver.try_iter().count(), 1);
        assert_eq!(
            sampler.current_sample_buffer.len(),
            0,
            "gap frames were buffered"
        );
        Ok(())
    }

    #[test]
    fn packet_end_of_stream_completes_normally() -> Result<()> {
        process_first_packet(|_| end_of_stream_error())
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
