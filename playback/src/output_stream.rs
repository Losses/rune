use std::cell::Cell;
use std::sync::{Arc, Weak};

use rodio::cpal::Sample;
#[cfg(not(any(feature = "test-output", test)))]
use rodio::cpal::traits::HostTrait;
use rodio::cpal::traits::StreamTrait;
use rodio::dynamic_mixer::{self, DynamicMixerController};
use rodio::source::Source;
use rodio::{DeviceTrait, SupportedStreamConfig, cpal};
use rodio::{PlayError, StreamError};
#[cfg(any(feature = "test-output", test))]
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

pub struct RuneOutputStream {
    mixer: Arc<DynamicMixerController<f32>>,
    _stream: Option<cpal::Stream>,
    // The stream is stopped while nothing plays. Only iOS stops it.
    suspended: Cell<bool>,
    #[cfg(any(feature = "test-output", test))]
    test_stop: Option<Arc<AtomicBool>>,
    // Whether the fake output pulls samples, as a device stream does while it runs. Only a fake
    // output made to stop with `suspend` has this (see [`FakeSuspend`]).
    #[cfg(any(feature = "test-output", test))]
    test_running: Option<Arc<AtomicBool>>,
}

/// What a fake output stream does when it is suspended.
#[cfg(any(feature = "test-output", test))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FakeSuspend {
    /// It runs on, as a device stream does everywhere but on iOS.
    RunsOn,
    /// It stops, as on iOS, so that tests see what iOS does.
    Stops,
}

#[derive(Clone)]
pub struct RuneOutputStreamHandle {
    mixer: Weak<DynamicMixerController<f32>>,
}

impl RuneOutputStream {
    pub fn try_from_device_with_callback<E>(
        device: &cpal::Device,
        error_callback: E,
    ) -> Result<(Self, RuneOutputStreamHandle), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone,
    {
        let default_config = device
            .default_output_config()
            .map_err(StreamError::DefaultStreamConfigError)?;
        RuneOutputStream::try_from_device_config_with_callback(
            device,
            default_config,
            error_callback,
        )
    }

    pub fn try_from_device_config_with_callback<E>(
        device: &cpal::Device,
        config: SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Self, RuneOutputStreamHandle), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone,
    {
        let (mixer, _stream) =
            device.try_new_output_stream_config_with_callback(config, error_callback)?;
        _stream.play().map_err(StreamError::PlayStreamError)?;
        let out = Self {
            mixer,
            _stream: Some(_stream),
            suspended: Cell::new(false),
            #[cfg(any(feature = "test-output", test))]
            test_stop: None,
            #[cfg(any(feature = "test-output", test))]
            test_running: None,
        };
        let handle = RuneOutputStreamHandle {
            mixer: Arc::downgrade(&out.mixer),
        };
        Ok((out, handle))
    }

    /// Stops the stream while nothing plays. iOS takes an app's play state, which the lock screen
    /// and Control Center show, from whether the app's audio output runs, so a stream that runs on
    /// with silence shows a paused player as playing. No other platform reads the play state from
    /// the output, so elsewhere the stream runs on as it always has. A fake output made with
    /// [`FakeSuspend::Stops`] stops too, so that tests see what iOS does.
    ///
    /// Nothing pulls samples from a stopped stream, so a sink on it takes no seek until it runs.
    pub fn suspend(&self) -> Result<(), cpal::PauseStreamError> {
        #[cfg(target_os = "ios")]
        if let Some(stream) = &self._stream {
            stream.pause()?;
            self.suspended.set(true);
        }
        #[cfg(any(feature = "test-output", test))]
        if let Some(running) = &self.test_running {
            running.store(false, Ordering::Relaxed);
            self.suspended.set(true);
        }
        Ok(())
    }

    /// Starts a stream that [`Self::suspend`] stopped. A stream that cannot start again, such as
    /// one whose audio session another app took, is of no more use.
    pub fn resume(&self) -> Result<(), cpal::PlayStreamError> {
        if !self.suspended.get() {
            return Ok(());
        }
        if let Some(stream) = &self._stream {
            stream.play()?;
        }
        #[cfg(any(feature = "test-output", test))]
        if let Some(running) = &self.test_running {
            running.store(true, Ordering::Relaxed);
        }
        self.suspended.set(false);
        Ok(())
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended.get()
    }

    #[cfg(not(any(feature = "test-output", test)))]
    pub fn try_default_with_callback<E>(
        error_callback: E,
    ) -> Result<(Self, RuneOutputStreamHandle), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone,
    {
        let default_device = cpal::default_host()
            .default_output_device()
            .ok_or(StreamError::NoDevice)?;

        let default_stream =
            Self::try_from_device_with_callback(&default_device, error_callback.clone());

        default_stream.or_else(|original_err| {
            let mut devices = match cpal::default_host().output_devices() {
                Ok(d) => d,
                Err(_) => return Err(original_err),
            };

            devices
                .find_map(|d| Self::try_from_device_with_callback(&d, error_callback.clone()).ok())
                .ok_or(original_err)
        })
    }
}

#[cfg(any(feature = "test-output", test))]
impl RuneOutputStream {
    pub fn try_default_with_callback<E>(_: E) -> Result<(Self, RuneOutputStreamHandle), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone,
    {
        Self::fake(FakeSuspend::RunsOn)
    }

    /// A fake output that pulls samples on a thread, and does `suspend` as told.
    pub fn fake(suspend: FakeSuspend) -> Result<(Self, RuneOutputStreamHandle), StreamError> {
        let (mixer, mut source) = dynamic_mixer::mixer::<f32>(2, 48_000);
        let stop = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));
        let test_running = (suspend == FakeSuspend::Stops).then(|| running.clone());
        let cancel = stop.clone();
        thread::spawn(move || {
            while !cancel.load(Ordering::Relaxed) {
                if running.load(Ordering::Relaxed) {
                    for _ in 0..960 {
                        let _ = source.next();
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let handle = RuneOutputStreamHandle {
            mixer: Arc::downgrade(&mixer),
        };
        Ok((
            Self {
                mixer,
                _stream: None,
                suspended: Cell::new(false),
                test_stop: Some(stop),
                test_running,
            },
            handle,
        ))
    }
}

#[cfg(any(feature = "test-output", test))]
impl Drop for RuneOutputStream {
    fn drop(&mut self) {
        if let Some(stop) = &self.test_stop {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

impl RuneOutputStreamHandle {
    pub fn play_raw<S>(&self, source: S) -> Result<(), PlayError>
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let mixer = self.mixer.upgrade().ok_or(PlayError::NoDevice)?;
        mixer.add(source);
        Ok(())
    }
}

pub(crate) trait CpalDeviceExt {
    fn new_output_stream_with_format_and_callback<E>(
        &self,
        format: cpal::SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Arc<DynamicMixerController<f32>>, cpal::Stream), cpal::BuildStreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone;

    fn try_new_output_stream_config_with_callback<E>(
        &self,
        config: cpal::SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Arc<DynamicMixerController<f32>>, cpal::Stream), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone;
}

impl CpalDeviceExt for cpal::Device {
    fn new_output_stream_with_format_and_callback<E>(
        &self,
        format: cpal::SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Arc<DynamicMixerController<f32>>, cpal::Stream), cpal::BuildStreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static,
    {
        let (mixer_tx, mut mixer_rx) =
            dynamic_mixer::mixer::<f32>(format.channels(), format.sample_rate().0);

        match format.sample_format() {
            cpal::SampleFormat::F32 => self.build_output_stream::<f32, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().unwrap_or(0f32))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::F64 => self.build_output_stream::<f64, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0f64))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I8 => self.build_output_stream::<i8, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i8))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I16 => self.build_output_stream::<i16, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i16))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I32 => self.build_output_stream::<i32, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i32))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I64 => self.build_output_stream::<i64, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i64))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U8 => self.build_output_stream::<u8, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(u8::MAX / 2)
                    })
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U16 => self.build_output_stream::<u16, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(u16::MAX / 2)
                    })
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U32 => self.build_output_stream::<u32, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(u32::MAX / 2)
                    })
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U64 => self.build_output_stream::<u64, _, _>(
                &format.config(),
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(u64::MAX / 2)
                    })
                },
                error_callback,
                None,
            ),
            _ => return Err(cpal::BuildStreamError::StreamConfigNotSupported),
        }
        .map(|stream| (mixer_tx, stream))
    }

    fn try_new_output_stream_config_with_callback<E>(
        &self,
        config: SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Arc<DynamicMixerController<f32>>, cpal::Stream), StreamError>
    where
        E: FnMut(cpal::StreamError) + Send + 'static + Clone,
    {
        self.new_output_stream_with_format_and_callback(config, error_callback.clone())
            .or_else(|err| {
                supported_output_formats(self)?
                    .find_map(|format| {
                        self.new_output_stream_with_format_and_callback(
                            format,
                            error_callback.clone(),
                        )
                        .ok()
                    })
                    .ok_or(StreamError::BuildStreamError(err))
            })
    }
}

fn supported_output_formats(
    device: &cpal::Device,
) -> Result<impl Iterator<Item = cpal::SupportedStreamConfig>, StreamError> {
    const HZ_44100: cpal::SampleRate = cpal::SampleRate(44_100);

    let mut supported: Vec<_> = device
        .supported_output_configs()
        .map_err(StreamError::SupportedStreamConfigsError)?
        .collect();
    supported.sort_by(|a, b| b.cmp_default_heuristics(a));

    Ok(supported.into_iter().flat_map(|sf| {
        let max_rate = sf.max_sample_rate();
        let min_rate = sf.min_sample_rate();
        let mut formats = vec![sf.with_max_sample_rate()];
        if HZ_44100 < max_rate && HZ_44100 > min_rate {
            formats.push(sf.with_sample_rate(HZ_44100))
        }
        formats.push(sf.with_sample_rate(min_rate));
        formats
    }))
}

/// How the player opens, stops and starts again its audio output. The player opens outputs this
/// way so that its tests can count them and make them fail.
pub(crate) trait Outputs: std::any::Any {
    fn open(
        &self,
        on_error: Arc<dyn Fn(cpal::StreamError) + Send + Sync>,
    ) -> Result<(RuneOutputStream, RuneOutputStreamHandle), StreamError>;

    fn suspend(&self, stream: &RuneOutputStream) -> Result<(), cpal::PauseStreamError>;

    fn resume(&self, stream: &RuneOutputStream) -> anyhow::Result<()>;
}

/// The device's output (a fake one in tests and `test-output` builds).
pub(crate) struct DeviceOutputs;

impl Outputs for DeviceOutputs {
    fn open(
        &self,
        on_error: Arc<dyn Fn(cpal::StreamError) + Send + Sync>,
    ) -> Result<(RuneOutputStream, RuneOutputStreamHandle), StreamError> {
        RuneOutputStream::try_default_with_callback(move |error| on_error(error))
    }

    fn suspend(&self, stream: &RuneOutputStream) -> Result<(), cpal::PauseStreamError> {
        stream.suspend()
    }

    fn resume(&self, stream: &RuneOutputStream) -> anyhow::Result<()> {
        Ok(stream.resume()?)
    }
}
