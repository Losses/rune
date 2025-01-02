use std::sync::mpsc::{channel, Receiver, Sender};

use anyhow::{bail, Result};
use rubato::{FftFixedInOut, Resampler};
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::formats::FormatReader;
use symphonia::core::sample::Sample;
use tokio_util::sync::CancellationToken;

use analysis::utils::audio_metadata_reader::{get_codec_information, get_format};

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
    sender: Sender<SampleEvent>,
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
                    .map_or(false, |token| token.is_cancelled())
            }),
            is_cancelled: false,

            sender,
            receiver,
        }
    }

    // Main processing function to read and process audi
    pub fn process(&mut self) -> Result<()> {
        // Get the audio format from the file path
        let mut format = get_format(&self.file_path)?;

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
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &dec_opts)?;

        // Create a resampler for converting audio to the target sample rate
        let resampler = FftFixedInOut::<f64>::new(
            sample_rate as usize,
            self.target_sample_rate as usize,
            self.samples_per_chunk,
            1,
        )?;

        // Process the audio stream using the format reader, decoder, and resampler
        self.process_audio_stream(
            &mut format,
            &mut decoder,
            track_id,
            sample_rate,
            duration,
            resampler,
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
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        let frames = buf.frames();
        let num_channels = buf.spec().channels.count();
        let frame_duration = 1.0 / sample_rate as f64;

        for frame_idx in 0..frames {
            // Mix audio channels into one sample
            let mut frame_sum = 0.0;
            for channel in 0..num_channels {
                frame_sum += IntoSample::<f64>::into_sample(buf.chan(channel)[frame_idx]);
            }
            let mixed_sample = frame_sum / num_channels as f64;

            // Calculate the next frame time
            let next_time = self.current_time + frame_duration;

            // Check if we're within the current sample window
            if self.current_time >= self.next_sample_time
                && self.current_time < (self.next_sample_time + self.sample_duration)
            {
                // Add the sample to the current buffer
                self.current_sample_buffer.push(mixed_sample);

                // Check if we've collected enough samples for this chunk
                if self.current_sample_buffer.len() >= self.samples_per_chunk {
                    // Process and send the current chunk
                    let input_frames = vec![self.current_sample_buffer.clone()];
                    resampler.process_into_buffer(&input_frames, output_buffer, None)?;

                    self.sender.send(SampleEvent {
                        sample_index: *sample_index,
                        total_samples: (self.interval / self.sample_duration) as usize,
                        data: output_buffer[0].clone(),
                        sample_rate: self.target_sample_rate,
                        duration: self.sample_duration,
                        start_time: self.next_sample_time,
                        end_time: self.next_sample_time + self.sample_duration,
                    })?;

                    // Reset buffer and update counters
                    self.current_sample_buffer.clear();
                    self.next_sample_time += self.interval;
                    *sample_index += 1;
                }
            } else if self.current_time >= (self.next_sample_time + self.sample_duration) {
                // We've moved past the current sample window without filling the buffer
                // Fill the remaining space with silence
                while self.current_sample_buffer.len() < self.samples_per_chunk {
                    self.current_sample_buffer.push(0.0);
                }

                // Process and send the current chunk
                let input_frames = vec![self.current_sample_buffer.clone()];
                resampler.process_into_buffer(&input_frames, output_buffer, None)?;

                self.sender.send(SampleEvent {
                    sample_index: *sample_index,
                    total_samples: (self.interval / self.sample_duration) as usize,
                    data: output_buffer[0].clone(),
                    sample_rate: self.target_sample_rate,
                    duration: self.sample_duration,
                    start_time: self.next_sample_time,
                    end_time: self.next_sample_time + self.sample_duration,
                })?;

                // Reset buffer and update counters
                self.current_sample_buffer.clear();
                self.next_sample_time += self.interval;
                *sample_index += 1;
            }

            self.current_time = next_time;
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
        mut resampler: FftFixedInOut<f64>,
    ) -> Result<()> {
        let mut sample_index = 0;
        let mut output_buffer = resampler.output_buffer_allocate(true);

        // Loop through the audio stream until the end
        while self.current_time < duration {
            // Check for cancellation
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return Ok(());
            }

            // Get the next packet from the format reader
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(_) => break, // Exit loop on error
            };

            // Skip packets that do not match the track ID
            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(_) => continue, // Skip on decode error
            };

            // Process the decoded audio buffer based on its type
            match decoded {
                AudioBufferRef::U8(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S8(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::F32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::F64(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        &mut output_buffer,
                        &mut sample_index,
                    )?;
                }
            }
        }

        Ok(())
    }
}
