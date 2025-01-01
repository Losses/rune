use anyhow::Result;
use rubato::{FftFixedInOut, Resampler};
use std::sync::mpsc::Sender;
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::Decoder;
use symphonia::core::conv::IntoSample;
use symphonia::core::formats::FormatReader;
use symphonia::core::sample::Sample;
use tokio_util::sync::CancellationToken;

pub struct SampleEvent {
    pub sample_index: usize, // Index of the sample event
    pub data: Vec<f64>,      // Resampled audio data
    pub sample_rate: u32,    // Sample rate of the output data
    pub duration: f64,       // Duration of the sample in seconds
}

pub struct IntervalSampler {
    sample_duration: f64,    // Duration of each sample in seconds
    interval: f64,           // Interval between samples in seconds
    target_sample_rate: u32, // Target sample rate for resampling

    // Processing state
    current_position: f64,    // Current processing position in seconds
    current_buffer: Vec<f64>, // Buffer for the current audio samples
    samples_per_chunk: usize, // Number of samples per chunk

    // Cancellation flag
    fn_is_cancelled: Box<dyn Fn() -> bool + Send>, // Function to check if processing is cancelled
    is_cancelled: bool,                            // Flag indicating if processing is cancelled
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
            current_position: 0.0,
            current_buffer: Vec::new(),
            samples_per_chunk: 0,
            fn_is_cancelled: Box::new(move || {
                cancel_token
                    .as_ref()
                    .map_or(false, |token| token.is_cancelled())
            }),
            is_cancelled: false,
        }
    }

    pub fn process(
        &mut self,
        mut format: Box<dyn FormatReader>,
        mut decoder: Box<dyn Decoder>,
        track_id: u32,
        sender: Sender<SampleEvent>,
    ) -> impl Iterator<Item = Result<()>> + '_ {
        // Determine the sample rate from the decoder; default to 44100 if not available
        let sample_rate = decoder.codec_params().sample_rate.unwrap_or(44100);
        self.samples_per_chunk = (self.sample_duration * sample_rate as f64) as usize;

        // Initialize the resampler
        let mut resampler = FftFixedInOut::<f64>::new(
            sample_rate as usize,
            self.target_sample_rate as usize,
            self.samples_per_chunk,
            1, // Number of channels
        )
        .unwrap();

        let mut output_buffer = resampler.output_buffer_allocate(true);

        std::iter::from_fn(move || {
            // Check if the process is cancelled
            if self.is_cancelled || (self.fn_is_cancelled)() {
                return None;
            }

            // Process packets until the next interval point is reached
            while self.current_position < self.interval {
                let packet = match format.next_packet() {
                    Ok(packet) => packet,
                    Err(_) => return None, // End of stream or error
                };

                // Skip packets not matching the track ID
                if packet.track_id() != track_id {
                    continue;
                }

                let decoded = match decoder.decode(&packet) {
                    Ok(decoded) => decoded,
                    Err(_) => continue, // Skip errors in decoding
                };

                let frames = decoded.frames() as f64;

                // Process the audio buffer
                let result = match decoded {
                    // Handle different audio formats by converting them to f64
                    AudioBufferRef::U8(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::U16(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::U24(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::U32(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::S8(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::S16(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::S24(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::S32(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::F32(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                    AudioBufferRef::F64(buf) => self.process_buffer(
                        buf.as_ref(),
                        &mut resampler,
                        &mut output_buffer,
                        &sender,
                    ),
                };

                if let Err(e) = result {
                    return Some(Err(e)); // Return error if processing fails
                }

                // Update the current position
                self.current_position += frames / sample_rate as f64;
            }

            // Reset position counter by subtracting the interval
            self.current_position -= self.interval;

            // Clear the buffer to prepare for the next interval
            self.current_buffer.clear();

            Some(Ok(()))
        })
    }

    fn process_buffer<T>(
        &mut self,
        buf: &AudioBuffer<T>,
        resampler: &mut FftFixedInOut<f64>,
        output_buffer: &mut [Vec<f64>],
        sender: &Sender<SampleEvent>,
    ) -> Result<()>
    where
        T: Sample + IntoSample<f64>,
    {
        let num_channels = buf.spec().channels.count();
        let frames = buf.frames();

        // Iterate over each frame in the buffer
        for frame_idx in 0..frames {
            let mut frame_sum = 0.0;
            // Sum samples from all channels to mix them into a single sample
            for channel in 0..num_channels {
                frame_sum += IntoSample::<f64>::into_sample(buf.chan(channel)[frame_idx]);
            }

            // Average the mixed sample
            let mixed_sample = frame_sum / num_channels as f64;
            self.current_buffer.push(mixed_sample);

            // Check if the buffer has enough samples for processing
            if self.current_buffer.len() >= self.samples_per_chunk {
                let input_frames = vec![self.current_buffer[..self.samples_per_chunk].to_vec()];
                resampler.process_into_buffer(&input_frames, output_buffer, None)?;

                // Send the processed sample event to the sender
                sender.send(SampleEvent {
                    sample_index: (self.current_position / self.interval) as usize,
                    data: output_buffer[0].clone(),
                    sample_rate: self.target_sample_rate,
                    duration: self.sample_duration,
                })?;

                // Clear the buffer after processing
                self.current_buffer.clear();
            }
        }

        Ok(())
    }
}
