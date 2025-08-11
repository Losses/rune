use std::sync::mpsc::Sender;

use anyhow::{Result, bail};
use rubato::{FftFixedInOut, Resampler};
use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, Signal},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    conv::IntoSample,
    formats::FormatReader,
    sample::Sample,
};
use tokio_util::sync::CancellationToken;

use analysis::utils::audio_metadata_reader::{get_codec_information, get_format};
use fsio::FsIo;

pub struct SampleEvent {
    pub sample_index: usize,  // The index of the current sample
    pub total_samples: usize, // Total number of samples
    pub data: Vec<f64>,       // Sample data
    pub sample_rate: u32,     // Sample rate
    pub duration: f64,
}

pub struct UniformSampler {
    sample_duration: f64,    // Duration of each sample (in seconds)
    sample_count: usize,     // Number of samples needed
    target_sample_rate: u32, // Target sample rate

    // Processing state
    current_sample_index: usize, // Current index of the sample being processed
    current_sample_buffer: Vec<f64>, // Buffer for storing current samples
    samples_per_chunk: usize,    // Number of samples per chunk
    overlap: usize,              // Overlap between chunks

    // Cancellation flag
    fn_is_cancelled: Box<dyn Fn() -> bool + Send>, // Function to check if the process is cancelled
    is_cancelled: bool,                            // Cancellation status
}

impl UniformSampler {
    pub fn new(
        sample_duration: f64,
        sample_count: usize,
        target_sample_rate: u32,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        UniformSampler {
            sample_duration,
            sample_count,
            target_sample_rate,
            current_sample_index: 0,
            current_sample_buffer: Vec::new(),
            samples_per_chunk: 0,
            overlap: 0,
            fn_is_cancelled: Box::new(move || {
                cancel_token
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
            }),
            is_cancelled: false,
        }
    }

    pub fn process(
        &mut self,
        fsio: &FsIo,
        file_path: &str,
        sender: Sender<SampleEvent>,
    ) -> Result<()> {
        let mut format = get_format(fsio, file_path)?;
        let track = match format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        {
            Some(x) => x,
            None => bail!("Failed to find a valid audio track"),
        };

        let track_id = track.id;

        let (sample_rate, duration) = get_codec_information(track)?;

        // Calculate how many samples are needed for each chunk
        self.samples_per_chunk = (self.sample_duration * sample_rate as f64) as usize;

        // If the total desired duration exceeds the audio duration, calculate the overlap
        let total_desired_duration = self.sample_duration * self.sample_count as f64;
        if total_desired_duration > duration {
            let overlap_duration =
                (total_desired_duration - duration) / (self.sample_count - 1) as f64;
            self.overlap = (overlap_duration * sample_rate as f64) as usize;
        }

        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &dec_opts)?;

        // Set up the resampler
        let resampler = FftFixedInOut::<f64>::new(
            sample_rate as usize,
            self.target_sample_rate as usize,
            self.samples_per_chunk,
            1,
        )?;
        let mut output_buffer = resampler.output_buffer_allocate(true);

        self.process_audio_stream(
            &mut format,
            &mut decoder,
            track_id,
            resampler,
            &mut output_buffer,
            &sender,
        )?;

        Ok(())
    }

    fn process_audio_chunk(
        &mut self,
        chunk: &[f64],
        resampler: &mut FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
    ) -> Result<()> {
        let input_frames = vec![chunk.to_vec()];
        resampler.process_into_buffer(&input_frames, output_buffer, None)?;

        // Send a sample event with the processed data
        sender.send(SampleEvent {
            sample_index: self.current_sample_index,
            total_samples: self.sample_count,
            data: output_buffer[0].clone(),
            sample_rate: self.target_sample_rate,
            duration: self.sample_duration,
        })?;

        self.current_sample_index += 1;
        Ok(())
    }

    fn process_audio_buffer<T>(
        &mut self,
        buf: &AudioBuffer<T>,
        resampler: &mut FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        // Get number of frames and channels
        let num_channels = buf.spec().channels.count();
        let frames = buf.frames();

        // Process frame by frame, averaging all channels
        for frame_idx in 0..frames {
            let mut frame_sum = 0.0;

            // Sum all channels for this frame
            for channel in 0..num_channels {
                let sample = buf.chan(channel)[frame_idx];
                frame_sum += IntoSample::<f64>::into_sample(sample);
            }

            // Average the sum by number of channels
            let mixed_sample = frame_sum / num_channels as f64;
            self.current_sample_buffer.push(mixed_sample);

            // Process the chunk if it reaches the required size
            if self.current_sample_buffer.len() >= self.samples_per_chunk {
                let chunk = self.current_sample_buffer[..self.samples_per_chunk].to_vec();
                self.process_audio_chunk(&chunk, resampler, output_buffer, sender)?;

                // Remove processed samples, keeping the overlap
                self.current_sample_buffer
                    .drain(..(self.samples_per_chunk - self.overlap));
            }
        }
        Ok(())
    }

    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn FormatReader>,
        decoder: &mut Box<dyn Decoder>,
        track_id: u32,
        mut resampler: FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
    ) -> Result<()> {
        loop {
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return Ok(());
            }

            if self.current_sample_index >= self.sample_count {
                break;
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(_) => break,
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(_) => continue,
            };

            match decoded {
                AudioBufferRef::U8(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::U16(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::U24(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::U32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::S8(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::S16(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::S24(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::S32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::F32(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
                AudioBufferRef::F64(buf) => {
                    self.process_audio_buffer(buf.as_ref(), &mut resampler, output_buffer, sender)?;
                }
            }
        }

        // Process any remaining samples
        if !self.current_sample_buffer.is_empty() && self.current_sample_index < self.sample_count {
            while self.current_sample_buffer.len() < self.samples_per_chunk {
                self.current_sample_buffer.push(0.0);
            }
            let buffer_clone = self.current_sample_buffer.clone();
            self.process_audio_chunk(&buffer_clone, &mut resampler, output_buffer, sender)?;
        }

        Ok(())
    }
}
