use std::sync::{Arc, Mutex};

use log::debug;

use rubato::{FftFixedInOut, Resampler};
use rustfft::num_complex::Complex;
use symphonia::{
    core::{
        audio::{AudioBuffer, AudioBufferRef, Signal},
        codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
        conv::IntoSample,
        errors::Error,
        formats::FormatReader,
        sample::Sample,
    },
    default::get_codecs,
};
use tokio_util::sync::CancellationToken;

use fsio::FsIo;

use crate::{
    analyzer::{
        cpu_sub_analyzer::CpuSubAnalyzer, gpu_sub_analyzer::GpuSubAnalyzer,
        sub_analyzer::SubAnalyzer,
    },
    utils::{
        audio_description::AudioDescription, audio_metadata_reader::*,
        computing_device::ComputingDevice,
    },
};

macro_rules! check_cancellation {
    ($self:expr) => {
        if $self.is_cancelled || ($self.fn_is_cancelled)() {
            $self.is_cancelled = true;
            return;
        }
    };

    ($self:expr,$func:expr) => {{
        if $self.is_cancelled || ($self.fn_is_cancelled)() {
            $self.is_cancelled = true;
            return;
        }
        $func;
    }};
}

pub struct Analyzer {
    pub batch_size: usize,
    pub window_size: usize,
    overlap_size: usize,
    pub avg_spectrum: Vec<Complex<f32>>,
    sample_buffer: Vec<f32>,

    fn_is_cancelled: Box<dyn Fn() -> bool>,
    is_cancelled: bool,

    // Processing state
    pub count: usize,
    total_samples: usize,
    sample_rate: u32,
    duration_in_seconds: f64,
    pub total_rms: f32,
    pub total_zcr: usize,
    pub total_energy: f32,
    pub actual_data_size: usize,
    resample_ratio: f64,
    pub resampler: Option<FftFixedInOut<f32>>,
    pub resampler_output_buffer: Vec<Vec<f32>>,
    sub_analyzer: Arc<Mutex<dyn SubAnalyzer>>,
}

impl Analyzer {
    pub fn new(
        computing_device: ComputingDevice,
        window_size: usize,
        overlap_size: usize,
        batch_size: Option<usize>,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        let batch_size = if let Some(batch_size) = batch_size {
            batch_size
        } else if computing_device == ComputingDevice::Gpu {
            1024 * 8
        } else {
            1
        };

        Analyzer {
            batch_size,
            window_size,
            overlap_size,
            avg_spectrum: vec![Complex::new(0.0, 0.0); window_size],
            sample_buffer: Vec::with_capacity(window_size),

            fn_is_cancelled: Box::new(move || {
                cancel_token
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled())
            }),
            is_cancelled: false,

            count: 0,
            total_samples: 0,
            sample_rate: 0,
            duration_in_seconds: 0.0,
            total_rms: 0.0,
            total_zcr: 0,
            total_energy: 0.0,
            actual_data_size: 0,
            resample_ratio: 0.0,
            resampler: None,
            resampler_output_buffer: vec![],

            sub_analyzer: if computing_device == ComputingDevice::Gpu {
                Arc::new(Mutex::new(GpuSubAnalyzer::new(window_size, batch_size)))
            } else {
                Arc::new(Mutex::new(CpuSubAnalyzer::new(window_size)))
            },
        }
    }

    pub fn process(&mut self, fsio: &FsIo, file_path: &str) -> Option<AudioDescription> {
        let mut format = get_format(fsio, file_path).expect("no supported audio tracks");
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("No supported audio tracks");

        let (sample_rate, duration_in_seconds) = get_codec_information(track).unwrap();
        self.sample_rate = sample_rate;
        self.duration_in_seconds = duration_in_seconds;

        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = get_codecs()
            .make(&track.codec_params, &dec_opts)
            .expect("unsupported codec");

        let track_id = track.id;

        if self.is_cancelled || (self.fn_is_cancelled)() {
            return None;
        }

        self.process_audio_stream(&mut format, &mut decoder, track_id);

        Some(AudioDescription {
            sample_rate: self.sample_rate,
            duration: self.duration_in_seconds,
            total_samples: self.total_samples,
            spectrum: self.avg_spectrum.clone(),
            rms: self.total_rms / self.count as f32,
            zcr: self.total_zcr / self.count,
            energy: self.total_energy / self.count as f32,
        })
    }

    fn process_audio_chunk(&mut self, chunk: &[f32], force: bool) {
        Arc::clone(&self.sub_analyzer)
            .lock()
            .unwrap()
            .process_audio_chunk(self, chunk, force);
    }

    fn process_audio_buffer<T>(&mut self, buf: &AudioBuffer<T>)
    where
        T: Sample + IntoSample<f32>,
    {
        let frames = buf.frames();
        let num_channels = buf.spec().channels.count();

        for frame_idx in 0..frames {
            let mixed_sample: f32 = (0..num_channels)
                .map(|ch| IntoSample::<f32>::into_sample(buf.chan(ch)[frame_idx]))
                .sum::<f32>()
                / num_channels as f32;

            self.sample_buffer.push(mixed_sample);
            self.total_samples += 1;

            while self.sample_buffer.len() >= self.actual_data_size {
                let chunk: Vec<f32> = self.sample_buffer[..self.actual_data_size].to_vec();
                self.process_audio_chunk(&chunk, false);
                self.sample_buffer
                    .drain(..(self.window_size - self.overlap_size));
            }
        }
    }

    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn FormatReader>,
        decoder: &mut Box<dyn Decoder>,
        track_id: u32,
    ) {
        self.resample_ratio = 11025_f64 / self.sample_rate as f64;
        self.actual_data_size = ((self.window_size) as f64 / self.resample_ratio).ceil() as usize;

        self.resampler = Some(
            FftFixedInOut::<f32>::new(self.sample_rate as usize, 11025, self.actual_data_size, 1)
                .unwrap(),
        );
        self.resampler_output_buffer = self
            .resampler
            .as_mut()
            .unwrap()
            .output_buffer_allocate(true);

        self.actual_data_size = self.resampler.as_mut().unwrap().input_frames_max();

        // Decode loop.
        loop {
            // Check for cancellation
            check_cancellation!(self);

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

            match decoded {
                AudioBufferRef::U8(buf) => {
                    debug!("Decoded buffer type: U8, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::U16(buf) => {
                    debug!("Decoded buffer type: U16, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::U24(buf) => {
                    debug!("Decoded buffer type: U24, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::U32(buf) => {
                    debug!("Decoded buffer type: U32, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::S8(buf) => {
                    debug!("Decoded buffer type: S8, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::S16(buf) => {
                    debug!("Decoded buffer type: S16, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::S24(buf) => {
                    debug!("Decoded buffer type: S24, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::S32(buf) => {
                    debug!("Decoded buffer type: S32, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::F32(buf) => {
                    debug!("Decoded buffer type: F32, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
                AudioBufferRef::F64(buf) => {
                    debug!("Decoded buffer type: F64, length: {}", buf.frames());
                    check_cancellation!(self, self.process_audio_buffer(buf.as_ref()));
                }
            }
        }

        if !self.sample_buffer.is_empty() {
            // Pad to the nearest multiple of 1024
            let target_size = self.total_samples.div_ceil(1024) * 1024;
            while self.sample_buffer.len() < target_size {
                self.sample_buffer.push(0.0);
            }

            // Only process up to target_size
            let chunk: Vec<f32> =
                self.sample_buffer[..target_size.min(self.actual_data_size)].to_vec();
            check_cancellation!(self, self.process_audio_chunk(&chunk, true));
        }

        debug!("Total samples: {}", self.total_samples);

        if self.count == 0 {
            panic!("No audio data processed");
        }

        // Calculate the final average spectrum.
        for value in self.avg_spectrum.iter_mut() {
            *value /= self.count as f32;
        }
        debug!("Final average spectrum calculated");
    }
}
