use std::sync::mpsc::Sender;

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
    pub data: Vec<f64>,       // Sample data
    pub sample_rate: u32,     // Sample rate
    pub duration: f64,        // Sample duration
    pub start_time: f64,      // Start time of this sample in the audio (in seconds)
    pub end_time: f64,        // End time of this sample in the audio (in seconds)
}

pub struct IntervalSampler {
    sample_duration: f64,    // Duration of each sample (in seconds)
    interval: f64,           // Time interval between samples (in seconds)
    target_sample_rate: u32, // Target sample rate

    // Processing state
    current_time: f64,               // Current time position in the audio
    current_sample_buffer: Vec<f64>, // Buffer for storing current samples
    samples_per_chunk: usize,        // Number of samples per chunk

    // Cancellation flag
    fn_is_cancelled: Box<dyn Fn() -> bool + Send>,
    is_cancelled: bool,
}

impl IntervalSampler {
    pub fn new(
        sample_duration: f64,
        interval: f64,
        target_sample_rate: u32,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        IntervalSampler {
            sample_duration,
            interval,
            target_sample_rate,
            current_time: 0.0,
            current_sample_buffer: Vec::new(),
            samples_per_chunk: 0,
            fn_is_cancelled: Box::new(move || {
                cancel_token
                    .as_ref()
                    .map_or(false, |token| token.is_cancelled())
            }),
            is_cancelled: false,
        }
    }

    pub fn process(&mut self, file_path: &str, sender: Sender<SampleEvent>) -> Result<()> {
        let mut format = get_format(file_path)?;
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
        self.samples_per_chunk = (self.sample_duration * sample_rate as f64) as usize;

        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &dec_opts)?;

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
            sample_rate,
            duration,
            resampler,
            &mut output_buffer,
            &sender,
        )?;

        Ok(())
    }

    fn should_take_sample(&self, current_time: f64) -> bool {
        (current_time / self.interval).floor() > (self.current_time / self.interval).floor()
    }

    fn process_audio_buffer<T>(
        &mut self,
        buf: &AudioBuffer<T>,
        sample_rate: u32,
        resampler: &mut FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
        sample_index: &mut usize,
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        let frames = buf.frames();
        let num_channels = buf.spec().channels.count();
        let frame_duration = 1.0 / sample_rate as f64;
        let mut sample_start_time: Option<f64> = None;

        for frame_idx in 0..frames {
            let next_time = self.current_time + frame_duration;

            if self.should_take_sample(next_time) {
                // Clear buffer if we're starting a new sample
                self.current_sample_buffer.clear();
                sample_start_time = Some(self.current_time);
            }

            // Mix channels
            let mut frame_sum = 0.0;
            for channel in 0..num_channels {
                frame_sum += IntoSample::<f64>::into_sample(buf.chan(channel)[frame_idx]);
            }
            let mixed_sample = frame_sum / num_channels as f64;

            // Add sample to buffer if we're collecting
            if self.should_take_sample(next_time) || !self.current_sample_buffer.is_empty() {
                self.current_sample_buffer.push(mixed_sample);
            }

            // Process if we have enough samples
            if self.current_sample_buffer.len() >= self.samples_per_chunk {
                let input_frames = vec![self.current_sample_buffer.clone()];
                resampler.process_into_buffer(&input_frames, output_buffer, None)?;

                sender.send(SampleEvent {
                    sample_index: *sample_index,
                    total_samples: (self.interval / self.sample_duration) as usize,
                    data: output_buffer[0].clone(),
                    sample_rate: self.target_sample_rate,
                    duration: self.sample_duration,
                    start_time: sample_start_time.unwrap_or(self.current_time),
                    end_time: sample_start_time.unwrap_or(self.current_time) + self.sample_duration,
                })?;

                *sample_index += 1;
                self.current_sample_buffer.clear();
                sample_start_time = None;
            }

            self.current_time = next_time;
        }
        Ok(())
    }

    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn FormatReader>,
        decoder: &mut Box<dyn Decoder>,
        track_id: u32,
        sample_rate: u32,
        duration: f64,
        mut resampler: FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
    ) -> Result<()> {
        let mut sample_index = 0;

        while self.current_time < duration {
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return Ok(());
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
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::U32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S8(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S16(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S24(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::S32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::F32(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
                AudioBufferRef::F64(buf) => {
                    self.process_audio_buffer(
                        buf.as_ref(),
                        sample_rate,
                        &mut resampler,
                        output_buffer,
                        sender,
                        &mut sample_index,
                    )?;
                }
            }
        }

        // Process any remaining samples in the buffer
        if !self.current_sample_buffer.is_empty() {
            while self.current_sample_buffer.len() < self.samples_per_chunk {
                self.current_sample_buffer.push(0.0);
            }
            let input_frames = vec![self.current_sample_buffer.clone()];
            resampler.process_into_buffer(&input_frames, output_buffer, None)?;

            let end_time =
                self.current_time + (self.current_sample_buffer.len() as f64 / sample_rate as f64);

            sender.send(SampleEvent {
                sample_index,
                total_samples: (duration / self.interval) as usize,
                data: output_buffer[0].clone(),
                sample_rate: self.target_sample_rate,
                duration: self.sample_duration,
                start_time: self.current_time,
                end_time,
            })?;
        }

        Ok(())
    }
}
