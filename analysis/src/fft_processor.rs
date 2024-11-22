use std::sync::Arc;

use log::{debug, info};

use realfft::{RealFftPlanner, RealToComplex};
use rubato::{FftFixedInOut, Resampler};
use rustfft::num_complex::Complex;
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::conv::IntoSample;
use symphonia::core::errors::Error;
use symphonia::core::sample::Sample;
use tokio_util::sync::CancellationToken;

use crate::computing_device::ComputingDevice;
use crate::features::energy;
use crate::features::rms;
use crate::features::zcr;

use crate::fft_utils::*;
use crate::wgpu_fft::wgpu_radix4;

pub struct FFTProcessor {
    computing_device: ComputingDevice,
    window_size: usize, // 
    batch_size: usize,
    overlap_size: usize,
    gpu_batch_fft: Option<wgpu_radix4::FFTCompute>,
    cpu_batch_fft: Option<Arc<dyn RealToComplex<f32>>>,
    cpu_batch_fft_output_buffer: Vec<Complex<f32>>,
    cpu_batch_fft_buffer: Vec<f32>,
    batch_fft_buffer: Vec<Complex<f32>>,
    avg_spectrum: Vec<Complex<f32>>,
    hanning_window: Vec<f32>,
    sample_buffer: Vec<f32>,
    batch_cache_buffer_count: usize,
    fn_is_cancelled: Box<dyn Fn() -> bool>,
    is_cancelled: bool,
    // Processing state
    count: usize,
    total_samples: usize,
    total_rms: f32,
    total_zcr: usize,
    total_energy: f32,
    actual_data_size: usize,
    resample_ratio: f64,
    sample_rate: u32,
    duration_in_seconds: f64,
    resampler: Option<FftFixedInOut<f32>>,
    resampler_output_buffer: Vec<Vec<f32>>,
}

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

impl FFTProcessor {
    pub fn new(
        computing_device: ComputingDevice,
        window_size: usize,
        // for cpu mode, batch_size must to be 1
        // for gpu mode, batch_size is recommended to be 1024 * 8
        batch_size: usize,
        overlap_size: usize,
        cancel_token: Option<CancellationToken>,
    ) -> Self {
        let gpu_batch_fft = if computing_device == ComputingDevice::Gpu {
            Some(pollster::block_on(wgpu_radix4::FFTCompute::new(
                window_size * batch_size,
            )))
        } else {
            None
        };
        let batch_fft_buffer = vec![Complex::new(0.0, 0.0); window_size * batch_size];
        let avg_spectrum = vec![Complex::new(0.0, 0.0); window_size];
        let hanning_window = build_hanning_window(window_size);
        let sample_buffer = Vec::with_capacity(window_size);

        let mut planner = RealFftPlanner::<f32>::new();
        let cpu_batch_fft = planner.plan_fft_forward(window_size);
        let cpu_batch_fft_output_buffer = cpu_batch_fft.make_output_vec();
        let cpu_batch_fft_buffer = cpu_batch_fft.make_input_vec();

        let fn_is_cancelled: Box<dyn Fn() -> bool> = Box::new(move || {
            cancel_token
                .as_ref()
                .map_or(false, |token| token.is_cancelled())
        });
        let is_cancelled = false;

        Self {
            computing_device,
            window_size,
            batch_size,
            overlap_size,
            gpu_batch_fft,
            cpu_batch_fft: Some(cpu_batch_fft),
            cpu_batch_fft_output_buffer,
            cpu_batch_fft_buffer,
            batch_fft_buffer,
            avg_spectrum,
            hanning_window,
            sample_buffer,
            batch_cache_buffer_count: 0,
            fn_is_cancelled,
            is_cancelled,
            // Processing state
            count: 0,
            total_samples: 0,
            total_rms: 0.0,
            total_zcr: 0,
            total_energy: 0.0,
            actual_data_size: 0,
            resample_ratio: 0.0,
            sample_rate: 0,
            duration_in_seconds: 0.0,
            resampler: None,
            resampler_output_buffer: Vec::new(),
        }
    }

    pub fn process_file(&mut self, file_path: &str) -> Option<AudioDescription> {
        let mut format = get_format(file_path).expect("no supported audio tracks");
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("No supported audio tracks");

        let (sample_rate, duration_in_seconds) = get_codec_information(track).unwrap();
        self.sample_rate = sample_rate;
        self.duration_in_seconds = duration_in_seconds;

        let dec_opts: DecoderOptions = Default::default();
        let mut decoder = symphonia::default::get_codecs()
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

    fn process_audio_buffer<T>(&mut self, buf: &AudioBuffer<T>)
    where
        T: Sample + IntoSample<f32>,
    {
        for plane in buf.planes().planes() {
            debug!("Processing plane with len: {}", plane.len());
            for &sample in plane.iter() {
                let sample: f32 = IntoSample::<f32>::into_sample(sample);
                self.sample_buffer.push(sample);
                self.total_samples += 1;

                while self.sample_buffer.len() >= self.actual_data_size {
                    let chunk: Vec<f32> = self.sample_buffer[..self.actual_data_size].to_vec();
                    if self.computing_device == ComputingDevice::Gpu {
                        self.gpu_process_audio_chunk(&chunk, false);
                    } else {
                        self.cpu_process_audio_chunk(&chunk, false);
                    }
                    self.sample_buffer
                        .drain(..(self.window_size - self.overlap_size));
                }
            }
        }
    }

    fn process_audio_stream(
        &mut self,
        format: &mut Box<dyn symphonia::core::formats::FormatReader>,
        decoder: &mut Box<dyn symphonia::core::codecs::Decoder>,
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
            let target_size = ((self.total_samples + 1023) / 1024) * 1024;
            while self.sample_buffer.len() < target_size {
                self.sample_buffer.push(0.0);
            }

            // Only process up to target_size
            let chunk: Vec<f32> =
                self.sample_buffer[..target_size.min(self.actual_data_size)].to_vec();
            if self.computing_device == ComputingDevice::Gpu {
                check_cancellation!(self, self.gpu_process_audio_chunk(&chunk, true));
            } else {
                check_cancellation!(self, self.cpu_process_audio_chunk(&chunk, true));
            }
        }

        info!("Total samples: {}", self.total_samples);

        if self.count == 0 {
            panic!("No audio data processed");
        }

        // Calculate the final average spectrum.
        for value in self.avg_spectrum.iter_mut() {
            *value /= self.count as f32;
        }
        debug!("Final average spectrum calculated");
    }

    fn gpu_process_audio_chunk(&mut self, chunk: &[f32], force: bool) {
        let resampled_chunk = &self
            .resampler
            .as_mut()
            .unwrap()
            .process(&[chunk], None)
            .unwrap()[0];

        self.total_rms += rms(resampled_chunk);
        self.total_zcr += zcr(resampled_chunk);
        self.total_energy += energy(resampled_chunk);

        let start_idx = self.batch_cache_buffer_count * self.window_size;
        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= self.window_size {
                break;
            }
            let windowed_sample = sample * self.hanning_window[i];
            self.batch_fft_buffer[start_idx + i] = Complex::new(windowed_sample, 0.0);
        }

        self.batch_cache_buffer_count += 1;
        self.count += 1;

        let gpu_fft = self
            .gpu_batch_fft
            .as_ref()
            .expect("GPU computing device not initialized");

        if force {
            // Only process the valid portion of the batch buffer
            pollster::block_on(gpu_fft.compute_fft(&mut self.batch_fft_buffer));

            // Accumulate spectrums for the valid batches
            for batch_idx in 0..self.batch_cache_buffer_count {
                let start = batch_idx * self.window_size;
                for i in 0..self.window_size {
                    self.avg_spectrum[i] += self.batch_fft_buffer[start + i];
                }
            }

            self.batch_cache_buffer_count = 0;
        } else if self.batch_cache_buffer_count >= self.batch_size {
            pollster::block_on(gpu_fft.compute_fft(&mut self.batch_fft_buffer));

            // Split batch_fft_buffer into batches and accumulate into avg_spectrum
            for batch_idx in 0..self.batch_size {
                let start = batch_idx * self.window_size;

                for i in 0..self.window_size {
                    self.avg_spectrum[i] += self.batch_fft_buffer[start + i];
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }

    fn cpu_process_audio_chunk(&mut self, chunk: &[f32], force: bool) {
        let _ = &self
            .resampler
            .as_mut()
            .unwrap()
            .process_into_buffer(&[chunk], &mut self.resampler_output_buffer, None)
            .unwrap();

        let resampled_chunk = &self.resampler_output_buffer[0];

        self.total_rms += rms(resampled_chunk);
        self.total_zcr += zcr(resampled_chunk);
        self.total_energy += energy(resampled_chunk);

        // TODO: Already find one way to optimize this which can be 0.5ms faster in startup_0.ogg
        let start_idx = self.batch_cache_buffer_count * self.window_size;
        for (i, &sample) in resampled_chunk.iter().enumerate() {
            if i >= self.window_size {
                break;
            }
            let windowed_sample = sample * self.hanning_window[i];
            self.cpu_batch_fft_buffer[start_idx + i] = windowed_sample;
        }

        self.batch_cache_buffer_count += 1;
        self.count += 1;

        let cpu_fft = self
            .cpu_batch_fft
            .as_ref()
            .expect("CPU computing device not initialized");

        if force {
            cpu_fft
                .process(
                    &mut self.cpu_batch_fft_buffer,
                    &mut self.cpu_batch_fft_output_buffer,
                )
                .expect("Real FFT processing failed");

            let half_window = self.window_size / 2;
            for batch_idx in 0..self.batch_cache_buffer_count {
                let _start: usize = batch_idx * self.window_size;
                // Copy the real FFT output directly
                for i in 0..half_window + 1 {
                    self.avg_spectrum[i] += self.cpu_batch_fft_output_buffer[i];
                }
                // Reconstruct the conjugate symmetric part
                for i in 1..half_window {
                    self.avg_spectrum[self.window_size - i] +=
                        self.cpu_batch_fft_output_buffer[i].conj();
                }
            }

            self.batch_cache_buffer_count = 0;
        } else if self.batch_cache_buffer_count >= self.batch_size {
            cpu_fft
                .process(
                    &mut self.cpu_batch_fft_buffer,
                    &mut self.cpu_batch_fft_output_buffer,
                )
                .expect("FFT processing failed");

            let half_window = self.window_size / 2;
            for batch_idx in 0..self.batch_size {
                let _start = batch_idx * self.window_size;
                for i in 0..half_window + 1 {
                    self.avg_spectrum[i] += self.cpu_batch_fft_output_buffer[i];
                }
                for i in 1..half_window {
                    self.avg_spectrum[self.window_size - i] +=
                        self.cpu_batch_fft_output_buffer[i].conj();
                }
            }

            self.batch_cache_buffer_count = 0;
        }
    }
}

pub fn gpu_fft(
    file_path: &str,
    window_size: usize,
    batch_size: usize,
    overlap_size: usize,
    cancel_token: Option<CancellationToken>,
) -> Option<AudioDescription> {
    FFTProcessor::new(
        ComputingDevice::Gpu,
        window_size,
        batch_size,
        overlap_size,
        cancel_token,
    )
    .process_file(file_path)
}

pub fn cpu_fft(
    file_path: &str,
    window_size: usize,
    overlap_size: usize,
    cancel_token: Option<CancellationToken>,
) -> Option<AudioDescription> {
    FFTProcessor::new(
        ComputingDevice::Cpu,
        window_size,
        1,
        overlap_size,
        cancel_token,
    )
    .process_file(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_fft;
    use crate::measure_time;
    use rustfft::FftPlanner;

    #[test]
    fn test_rust_fft() {
        let size = 1024;
        let original_data: Vec<Complex<f32>> = (0..size)
            .map(|i| Complex::new((i as f32).sin(), 0.0))
            .collect();
        let mut fft_data = original_data.clone();

        // Create FFT and IFFT planners
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let ifft = planner.plan_fft_inverse(size);

        // Perform FFT
        fft.process(&mut fft_data);

        // Perform inverse FFT
        ifft.process(&mut fft_data);

        for x in fft_data.iter_mut() {
            *x /= size as f32;
        }

        // Compare original and reconstructed data
        for (orig, reconstructed) in original_data.iter().zip(fft_data.iter()) {
            let diff = (orig - reconstructed).norm();
            assert!(
                diff < 1e-6,
                "Difference too large: {} vs {}, diff = {}",
                orig,
                reconstructed,
                diff
            );
        }
    }

    #[test]
    fn test_real_fft() {
        let size = 1024;
        // Create real input data
        let mut real_input: Vec<f32> = (0..size).map(|i| (i as f32).sin()).collect();
        let mut real_output = real_input.clone();

        // Create real FFT planner
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(size);
        let c2r = planner.plan_fft_inverse(size);

        // Create complex buffer for FFT output
        let mut spectrum = r2c.make_output_vec();

        // Perform forward FFT
        r2c.process(&mut real_input, &mut spectrum).unwrap();

        // Perform inverse FFT
        c2r.process(&mut spectrum, &mut real_output).unwrap();

        // Scale the output
        for x in real_output.iter_mut() {
            *x /= size as f32;
        }

        // Compare original and reconstructed data
        for (orig, reconstructed) in real_input.iter().zip(real_output.iter()) {
            let diff = (orig - reconstructed).abs();
            assert!(
                diff < 1e-6,
                "Difference too large: {} vs {}, diff = {}",
                orig,
                reconstructed,
                diff
            );
        }
    }

    #[test]
    fn test_fft_cpu_vs_legacy() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let overlap_size = 512;

        let cpu_result = measure_time!(
            "CPU FFT",
            cpu_fft(file_path, window_size, overlap_size, None)
        )
        .unwrap();

        let legacy_cpu_result = measure_time!(
            "LEGACY CPU FFT",
            legacy_fft::fft(file_path, window_size, overlap_size, None)
        )
        .unwrap();

        info!("CPU result: {:?}", cpu_result);
        info!("Legacy CPU result: {:?}", legacy_cpu_result);

        // Compare results with tolerance
        assert!(
            (cpu_result.rms - legacy_cpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            cpu_result.rms,
            legacy_cpu_result.rms
        );
        assert!(
            (cpu_result.energy - legacy_cpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            cpu_result.energy,
            legacy_cpu_result.energy
        );
        assert!(
            cpu_result.zcr.abs_diff(legacy_cpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            cpu_result.zcr,
            legacy_cpu_result.zcr
        );
    }

    #[test]
    fn test_fft_gpu_vs_legacy() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let batch_size = 1024 * 8;
        let overlap_size = 512;

        let gpu_result = measure_time!(
            "GPU FFT",
            gpu_fft(file_path, window_size, batch_size, overlap_size, None)
        )
        .unwrap();

        let legacy_cpu_result = measure_time!(
            "LEGACY CPU FFT",
            legacy_fft::fft(file_path, window_size, overlap_size, None)
        )
        .unwrap();

        info!("GPU result: {:?}", gpu_result);
        info!("Legacy CPU result: {:?}", legacy_cpu_result);

        // Compare results with tolerance
        assert!(
            (gpu_result.rms - legacy_cpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            gpu_result.rms,
            legacy_cpu_result.rms
        );
        assert!(
            (gpu_result.energy - legacy_cpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            gpu_result.energy,
            legacy_cpu_result.energy
        );
        assert!(
            gpu_result.zcr.abs_diff(legacy_cpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            gpu_result.zcr,
            legacy_cpu_result.zcr
        );
    }

    #[test]
    fn test_fft_cpu_vs_gpu() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let file_path = "../assets/startup_0.ogg";
        let window_size = 1024;
        let batch_size = 1024 * 8;
        let overlap_size = 512;

        let cpu_result = measure_time!(
            "CPU FFT",
            cpu_fft(file_path, window_size, overlap_size, None)
        )
        .unwrap();

        let gpu_result = measure_time!(
            "GPU FFT",
            gpu_fft(file_path, window_size, batch_size, overlap_size, None)
        )
        .unwrap();

        info!("CPU result: {:?}", cpu_result);
        info!("GPU result: {:?}", gpu_result);

        // Compare results with tolerance
        assert!(
            (cpu_result.rms - gpu_result.rms).abs() < 0.01,
            "RMS difference too large: {} vs {}",
            cpu_result.rms,
            gpu_result.rms
        );
        assert!(
            (cpu_result.energy - gpu_result.energy).abs() < 5.0,
            "Energy difference too large: {} vs {}",
            cpu_result.energy,
            gpu_result.energy
        );
        assert!(
            cpu_result.zcr.abs_diff(gpu_result.zcr) < 10,
            "ZCR values don't match: {} vs {}",
            cpu_result.zcr,
            gpu_result.zcr
        );

        // Compare spectrum values
        for (i, (cpu_value, gpu_value)) in cpu_result
            .spectrum
            .iter()
            .zip(gpu_result.spectrum.iter())
            .enumerate()
        {
            assert!(
                (cpu_value.norm() - gpu_value.norm()).abs() < 0.001,
                "Spectrum difference too large at index {}: {} vs {}",
                i,
                cpu_value.norm(),
                gpu_value.norm()
            );
        }
    }
}
