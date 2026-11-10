use crate::utils::mixing::append_mixed;
use std::{
    io::ErrorKind,
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result, anyhow};
use log::debug;

use fsio_media_source::get_codecs;
use rubato::{FftFixedInOut, Resampler};
use rustfft::num_complex::Complex;
use symphonia::core::{
    audio::{AudioBuffer, AudioBufferRef, Signal},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    conv::IntoSample,
    errors::Error,
    formats::FormatReader,
    sample::Sample,
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
            return Ok(());
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
    target_buffer: Vec<f32>,
    delay_remaining: usize,
    emitted_samples: usize,

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
        assert!(window_size >= 2 && window_size.is_multiple_of(2));
        assert!(overlap_size < window_size);
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
            target_buffer: Vec::new(),
            delay_remaining: 0,
            emitted_samples: 0,

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
            resampler: None,
            resampler_output_buffer: vec![],

            sub_analyzer: if computing_device == ComputingDevice::Gpu {
                Arc::new(Mutex::new(GpuSubAnalyzer::new(window_size, batch_size)))
            } else {
                Arc::new(Mutex::new(CpuSubAnalyzer::new(window_size)))
            },
        }
    }

    pub fn process(&mut self, fsio: &FsIo, file_path: &str) -> Result<Option<AudioDescription>> {
        let mut format = get_format(fsio, file_path)?;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("no supported audio tracks"))?;

        let (sample_rate, duration_in_seconds) =
            get_codec_information(track).context("read audio codec information")?;
        self.sample_rate = sample_rate;
        self.duration_in_seconds = duration_in_seconds;

        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = get_codecs()
            .make(&track.codec_params, &dec_opts)
            .context("create audio decoder")?;

        let track_id = track.id;

        if self.is_cancelled || (self.fn_is_cancelled)() {
            return Ok(None);
        }

        self.process_audio_stream(&mut format, &mut decoder, track_id)?;

        if self.is_cancelled || self.count == 0 {
            return Ok(None);
        }

        Ok(Some(AudioDescription {
            sample_rate: self.sample_rate,
            duration: self.duration_in_seconds,
            total_samples: self.total_samples,
            spectrum: self.avg_spectrum.clone(),
            rms: self.total_rms / self.count as f32,
            zcr: self.total_zcr / self.count,
            energy: self.total_energy / self.count as f32,
        }))
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
        let mut samples = std::mem::take(&mut self.sample_buffer);
        append_mixed(buf, &mut samples);
        self.consume_mixed_samples(samples, frames);
    }

    fn consume_mixed_samples(&mut self, mut samples: Vec<f32>, frames: usize) {
        self.total_samples += frames;

        let mut consumed = 0;
        while samples.len() - consumed >= self.actual_data_size {
            check_cancellation!(self);
            self.resample_source_block(&samples[consumed..consumed + self.actual_data_size], None);
            consumed += self.actual_data_size;
        }
        // Consume each source sample once; overlap is applied after resampling.
        samples.drain(..consumed);
        self.sample_buffer = samples;
    }

    fn resample_source_block(&mut self, input: &[f32], final_length: Option<usize>) {
        let (_, written) = self
            .resampler
            .as_mut()
            .unwrap()
            .process_into_buffer(&[input], &mut self.resampler_output_buffer, None)
            .unwrap();
        let skip = self.delay_remaining.min(written);
        self.delay_remaining -= skip;
        let available = written - skip;
        let keep = final_length.map_or(available, |n| {
            available.min(n.saturating_sub(self.emitted_samples))
        });
        let mut target = std::mem::take(&mut self.target_buffer);
        target.extend_from_slice(&self.resampler_output_buffer[0][skip..skip + keep]);
        self.emitted_samples += keep;
        let mut consumed = 0;
        while target.len() - consumed >= self.window_size {
            self.process_audio_chunk(&target[consumed..consumed + self.window_size], false);
            consumed += self.window_size - self.overlap_size;
        }
        target.drain(..consumed);
        self.target_buffer = target;
    }

    fn initialize_resampler(&mut self) {
        let resample_ratio = crate::ANALYSIS_SAMPLE_RATE as f64 / self.sample_rate as f64;
        self.actual_data_size = ((self.window_size) as f64 / resample_ratio).ceil() as usize;

        self.resampler = Some(
            FftFixedInOut::<f32>::new(
                self.sample_rate as usize,
                crate::ANALYSIS_SAMPLE_RATE as usize,
                self.actual_data_size,
                1,
            )
            .unwrap(),
        );
        self.resampler_output_buffer = self
            .resampler
            .as_mut()
            .unwrap()
            .output_buffer_allocate(true);

        self.actual_data_size = self.resampler.as_mut().unwrap().input_frames_max();
        self.delay_remaining = self.resampler.as_ref().unwrap().output_delay();
    }

    fn finish_audio_stream(&mut self) {
        let expected = (self.total_samples as u64 * crate::ANALYSIS_SAMPLE_RATE as u64)
            .div_ceil(self.sample_rate as u64) as usize;
        if !self.sample_buffer.is_empty() {
            self.sample_buffer.resize(self.actual_data_size, 0.0);
            let input = std::mem::take(&mut self.sample_buffer);
            self.resample_source_block(&input, Some(expected));
        }
        let padding = vec![0.0; self.actual_data_size];
        while self.emitted_samples < expected {
            check_cancellation!(self);
            self.resample_source_block(&padding, Some(expected));
        }
        assert_eq!(self.emitted_samples, expected);
        if !self.target_buffer.is_empty() {
            // Center an entire short clip so Hann's zero endpoint cannot erase it.
            if self.count == 0 {
                let leading = (self.window_size - self.target_buffer.len()) / 2;
                self.target_buffer
                    .splice(0..0, std::iter::repeat_n(0.0, leading));
            }
            self.target_buffer.resize(self.window_size, 0.0);
            let chunk = std::mem::take(&mut self.target_buffer);
            self.process_audio_chunk(&chunk, true);
        }
    }

    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn FormatReader>,
        decoder: &mut Box<dyn Decoder>,
        track_id: u32,
    ) -> Result<()> {
        self.initialize_resampler();

        // Decode loop.
        loop {
            // Check for cancellation
            if self.is_cancelled || (self.fn_is_cancelled)() {
                self.is_cancelled = true;
                return Ok(());
            }

            // Get the next packet from the media format.
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    debug!("End of stream");
                    break;
                }
                Err(err) => return Err(anyhow!(err).context("read audio packet")),
            };
            debug!("Packet received: track_id = {}", packet.track_id());

            // If the packet does not belong to the selected track, skip over it.
            if packet.track_id() != track_id {
                continue;
            }

            // Decode the packet into audio samples.
            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(Error::IoError(error)) if error.kind() == ErrorKind::UnexpectedEof => {
                    debug!("IO Error while decoding");
                    continue;
                }
                Err(Error::DecodeError(_)) => {
                    debug!("Decode Error");
                    continue;
                }
                Err(err) => return Err(anyhow!(err).context("decode audio packet")),
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

        self.finish_audio_stream();

        debug!("Total samples: {}", self.total_samples);

        if self.count == 0 {
            return Ok(());
        }

        // Calculate the final average spectrum.
        for value in self.avg_spectrum.iter_mut() {
            // Store RMS magnitude, not the phase-sensitive mean complex FFT.
            *value = Complex::new((value.re / self.count as f32).sqrt(), 0.0);
        }
        debug!("Final average spectrum calculated");
        Ok(())
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn final_window_does_not_retain_track_sized_buffer() {
        let mut analyzer = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");
        let result = analyzer.process(&FsIo::new(), path).unwrap().unwrap();
        assert!(result.rms.is_finite());
        assert!(analyzer.total_samples > analyzer.actual_data_size);
        assert!(analyzer.sample_buffer.capacity() <= analyzer.actual_data_size * 2);
    }

    #[test]
    fn cancelled_analysis_does_not_return_partial_results() {
        let mut analyzer = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        // Cancel after process() enters the decode loop, before its first packet.
        let calls = Cell::new(0);
        analyzer.fn_is_cancelled = Box::new(move || {
            calls.set(calls.get() + 1);
            calls.get() > 1
        });
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/startup_0.ogg");
        assert!(analyzer.process(&FsIo::new(), path).unwrap().is_none());
    }
}

#[cfg(test)]
mod streaming_tests {

    use super::*;
    use std::f32::consts::PI;

    fn run(samples: &[f32], rate: u32, packet: usize) -> Analyzer {
        let mut a = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        a.sample_rate = rate;
        a.initialize_resampler();
        for chunk in samples.chunks(packet) {
            let mut input = std::mem::take(&mut a.sample_buffer);
            input.extend_from_slice(chunk);
            a.consume_mixed_samples(input, chunk.len());
        }
        a.finish_audio_stream();
        a
    }

    #[test]
    fn streaming_is_independent_of_decoder_packet_boundaries() {
        for rate in [8000, 11025, 44100, 48000, 96000] {
            let samples: Vec<f32> = (0..20003).map(|i| (i as f32 * 0.137).sin()).collect();
            let a = run(&samples, rate, 17);
            let b = run(&samples, rate, 4096);
            assert_eq!(
                a.emitted_samples,
                (samples.len() as u64 * 11025).div_ceil(rate as u64) as usize
            );
            assert_eq!(a.avg_spectrum, b.avg_spectrum);
            assert_eq!(a.count, b.count);
            assert_eq!(a.total_energy, b.total_energy);
            assert_eq!(a.total_samples, samples.len());
        }
    }

    #[test]
    fn one_sample_clip_is_not_erased_by_hann_endpoint() {
        for rate in [44100, 48000] {
            let a = run(&[1.0], rate, 1);
            assert_eq!(a.count, 1);
            assert!(a.avg_spectrum.iter().any(|x| x.re > 0.0));
        }
    }

    #[test]
    fn opposite_phase_frames_do_not_cancel_power() {
        let mut a = Analyzer::new(ComputingDevice::Cpu, 1024, 512, None, None);
        let frame: Vec<f32> = (0..1024)
            .map(|i| (2.0 * PI * 21.0 * i as f32 / 1024.0).sin())
            .collect();
        a.process_audio_chunk(&frame, false);
        let power = a.avg_spectrum[21].re;
        a.process_audio_chunk(&frame.iter().map(|x| -x).collect::<Vec<_>>(), true);
        assert!(power > 65000.0);
        assert!((a.avg_spectrum[21].re - 2.0 * power).abs() < 0.01);
    }
}
