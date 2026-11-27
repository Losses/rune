use std::cell::Cell;
use std::num::NonZero;
use std::sync::{Arc, Weak};

use rodio::cpal::Sample;
#[cfg(not(any(feature = "test-output", test)))]
use rodio::cpal::traits::HostTrait;
use rodio::cpal::traits::StreamTrait;
use rodio::mixer::{self, Mixer};
use rodio::source::Source;
use rodio::{DeviceSinkError, PlayError};
use rodio::{DeviceTrait, SupportedStreamConfig, cpal};
use std::any;
#[cfg(any(feature = "test-output", test))]
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

pub struct RuneOutputStream {
    mixer: Arc<Mixer>,
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
    mixer: Weak<Mixer>,
}

impl RuneOutputStream {
    pub fn try_from_device_with_callback<E>(
        device: &cpal::Device,
        error_callback: E,
    ) -> Result<(Self, RuneOutputStreamHandle), DeviceSinkError>
    where
        E: FnMut(cpal::Error) + Send + 'static + Clone,
    {
        let default_config = device
            .default_output_config()
            .map_err(DeviceSinkError::DefaultSinkConfigError)?;
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
    ) -> Result<(Self, RuneOutputStreamHandle), DeviceSinkError>
    where
        E: FnMut(cpal::Error) + Send + 'static + Clone,
    {
        let (mixer, _stream) = device
            .new_output_stream_with_format_and_callback(config, error_callback)
            .map_err(DeviceSinkError::BuildError)?;
        _stream.play().map_err(DeviceSinkError::PlayError)?;
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
    pub fn suspend(&self) -> Result<(), cpal::Error> {
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
    pub fn resume(&self) -> Result<(), cpal::Error> {
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

    /// Opens the default output device or, if that cannot play, the first other output device that
    /// can. Each device is tried with its default config only, and passed over if that fails,
    /// rather than with every other config it supports: a device behind ALSA's PulseAudio plugin
    /// supports hundreds, each try opens the device again, and while the server has no sink none
    /// of them can play, so trying them all takes about 0.4 s per device. ALSA's `null` device,
    /// which plays nothing, is not tried.
    #[cfg(not(any(feature = "test-output", test)))]
    pub fn try_default_with_callback<E>(
        error_callback: E,
    ) -> Result<(Self, RuneOutputStreamHandle), DeviceSinkError>
    where
        E: FnMut(cpal::Error) + Send + 'static + Clone,
    {
        let default_device = cpal::default_host()
            .default_output_device()
            .ok_or(DeviceSinkError::NoDevice)?;

        let default_stream =
            Self::try_from_device_with_callback(&default_device, error_callback.clone());

        default_stream.or_else(|original_err| {
            let devices = match cpal::default_host().output_devices() {
                Ok(d) => d,
                Err(_) => return Err(original_err),
            };

            devices
                .filter(|d| !discards_samples(d))
                .find_map(|d| Self::try_from_device_with_callback(&d, error_callback.clone()).ok())
                .ok_or(original_err)
        })
    }
}

/// Whether `device` is ALSA's `null` device, which takes any stream and throws its samples away as
/// fast as they come: a track that plays there runs to its end at full speed without a sound.
/// cpal 0.15 left it out of the output devices it listed; cpal 0.18 lists it.
#[cfg(any(test, not(feature = "test-output")))]
fn discards_samples(device: &cpal::Device) -> bool {
    device.id().is_ok_and(|id| id.id() == "null")
}

#[cfg(any(feature = "test-output", test))]
impl RuneOutputStream {
    pub fn try_default_with_callback<E>(
        _: E,
    ) -> Result<(Self, RuneOutputStreamHandle), DeviceSinkError>
    where
        E: FnMut(cpal::Error) + Send + 'static + Clone,
    {
        Self::fake(FakeSuspend::RunsOn)
    }

    /// A fake output that pulls samples on a thread, and does `suspend` as told.
    pub fn fake(suspend: FakeSuspend) -> Result<(Self, RuneOutputStreamHandle), DeviceSinkError> {
        let (mixer, mut source) = mixer::mixer(rodio::nz!(2), rodio::nz!(48_000));
        let mixer = Arc::new(mixer);
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
        S: Source + Send + 'static,
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
    ) -> Result<(Arc<Mixer>, cpal::Stream), cpal::Error>
    where
        E: FnMut(cpal::Error) + Send + 'static + Clone;
}

impl CpalDeviceExt for cpal::Device {
    fn new_output_stream_with_format_and_callback<E>(
        &self,
        format: cpal::SupportedStreamConfig,
        error_callback: E,
    ) -> Result<(Arc<Mixer>, cpal::Stream), cpal::Error>
    where
        E: FnMut(cpal::Error) + Send + 'static,
    {
        let (Some(channels), Some(sample_rate)) = (
            NonZero::new(format.channels()),
            NonZero::new(format.sample_rate()),
        ) else {
            return Err(cpal::ErrorKind::InvalidInput.into());
        };
        let (mixer_tx, mut mixer_rx) = mixer::mixer(channels, sample_rate);
        let config = stream_config(&format);

        match format.sample_format() {
            cpal::SampleFormat::F32 => self.build_output_stream::<f32, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().unwrap_or(0f32))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::F64 => self.build_output_stream::<f64, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0f64))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I8 => self.build_output_stream::<i8, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i8))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I16 => self.build_output_stream::<i16, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i16))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I24 => self.build_output_stream::<cpal::I24, _, _>(
                config,
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(cpal::I24::EQUILIBRIUM)
                    })
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I32 => self.build_output_stream::<i32, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i32))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::I64 => self.build_output_stream::<i64, _, _>(
                config,
                move |data, _| {
                    data.iter_mut()
                        .for_each(|d| *d = mixer_rx.next().map(Sample::from_sample).unwrap_or(0i64))
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U8 => self.build_output_stream::<u8, _, _>(
                config,
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
                config,
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
            cpal::SampleFormat::U24 => self.build_output_stream::<cpal::U24, _, _>(
                config,
                move |data, _| {
                    data.iter_mut().for_each(|d| {
                        *d = mixer_rx
                            .next()
                            .map(Sample::from_sample)
                            .unwrap_or(cpal::U24::EQUILIBRIUM)
                    })
                },
                error_callback,
                None,
            ),
            cpal::SampleFormat::U32 => self.build_output_stream::<u32, _, _>(
                config,
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
                config,
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
            _ => return Err(cpal::ErrorKind::UnsupportedConfig.into()),
        }
        .map(|stream| (Arc::new(mixer_tx), stream))
    }
}

/// The config an output stream is built with: `format`, with a buffer of about 100 ms on Linux.
///
/// cpal 0.15 asked ALSA for 100 ms. cpal 0.18 leaves the size to the device and makes the buffer
/// two of its periods: 32 ms through the PulseAudio plugin, where a stall of the stream's worker
/// of 50 ms, such as a slow read of the track, was heard as a gap. A fixed size is the size of a
/// period, and the buffer two of them, so this asks for periods of 50 ms, within the range the
/// device reports. rodio 0.23 asks for the same periods for its own outputs. On the other
/// platforms cpal's default did not change this way.
fn stream_config(format: &SupportedStreamConfig) -> cpal::StreamConfig {
    let buffer_size = match *format.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } if cfg!(target_os = "linux") => {
            cpal::BufferSize::Fixed((format.sample_rate() / 20).clamp(min, max))
        }
        _ => cpal::BufferSize::Default,
    };
    cpal::StreamConfig {
        buffer_size,
        ..format.config()
    }
}

/// How the player opens, stops and starts again its audio output. The player opens outputs this
/// way so that its tests can count them and make them fail.
pub(crate) trait Outputs: any::Any {
    fn open(
        &self,
        on_error: Arc<dyn Fn(cpal::Error) + Send + Sync>,
    ) -> Result<(RuneOutputStream, RuneOutputStreamHandle), DeviceSinkError>;

    fn suspend(&self, stream: &RuneOutputStream) -> Result<(), cpal::Error>;

    fn resume(&self, stream: &RuneOutputStream) -> anyhow::Result<()>;
}

/// The device's output (a fake one in tests and `test-output` builds).
pub(crate) struct DeviceOutputs;

impl Outputs for DeviceOutputs {
    fn open(
        &self,
        on_error: Arc<dyn Fn(cpal::Error) + Send + Sync>,
    ) -> Result<(RuneOutputStream, RuneOutputStreamHandle), DeviceSinkError> {
        RuneOutputStream::try_default_with_callback(move |error| on_error(error))
    }

    fn suspend(&self, stream: &RuneOutputStream) -> Result<(), cpal::Error> {
        stream.suspend()
    }

    fn resume(&self, stream: &RuneOutputStream) -> anyhow::Result<()> {
        Ok(stream.resume()?)
    }
}

/// Whether an error that an output stream reports ends the output for the player: the stream
/// plays no more, as when its device went away, or it plays through another device.
///
/// Two errors do not. After an xrun the stream recovers by itself, and only a gap was heard. And
/// through the ALSA pulse plugin, `snd_pcm_avail_delay` fails with an I/O error each time cpal
/// prepares the PCM, as a stream starts and after an underrun, until PulseAudio has timing data
/// for the plugin's new stream: alsa-plugins reports that wait as `EIO`. The stream then plays
/// on. A stream that lost its sink fails in other calls too.
pub(crate) fn ends_the_output(error: &cpal::Error) -> bool {
    match error.kind() {
        cpal::ErrorKind::Xrun => false,
        cpal::ErrorKind::BackendError => error.message() != Some(PULSE_TIMING_WAIT),
        _ => true,
    }
}

/// How cpal's ALSA backend reports `snd_pcm_avail_delay` failing with `EIO`.
const PULSE_TIMING_WAIT: &str =
    "ALSA function 'snd_pcm_avail_delay' failed with error 'I/O error (5)'";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_of_stream_error_but_an_xrun_ends_the_output() {
        assert!(!ends_the_output(&cpal::ErrorKind::Xrun.into()));
        // What a stream reports on each platform when its device goes away, when it plays through
        // another device or must be built again, and when the backend fails.
        for kind in [
            cpal::ErrorKind::DeviceNotAvailable,
            cpal::ErrorKind::DeviceChanged,
            cpal::ErrorKind::StreamInvalidated,
            cpal::ErrorKind::BackendError,
            cpal::ErrorKind::DeviceBusy,
            cpal::ErrorKind::Other,
        ] {
            assert!(ends_the_output(&kind.into()), "{kind:?}");
        }
    }

    /// The errors of cpal's ALSA backend, made from the alsa-lib call that failed as cpal makes
    /// them, so that a new wording in cpal or the alsa crate fails here.
    #[cfg(target_os = "linux")]
    #[test]
    fn of_the_alsa_errors_only_an_underrun_and_the_pulse_timing_wait_leave_the_output_playing() {
        const EIO: i32 = 5;
        const EPIPE: i32 = 32;
        const ENODEV: i32 = 19;
        let alsa = |function: &'static str, errno: i32| {
            cpal::Error::from(alsa::Error::new(function, errno))
        };

        // A write that underran, and the plugin's new stream waiting for timing data.
        assert!(!ends_the_output(&alsa("snd_pcm_writei", EPIPE)));
        assert!(!ends_the_output(&alsa("snd_pcm_avail_delay", EIO)));
        // The sink went away while the worker polled (#557; the alsa crate passes this call's
        // negative return on), the stream could not be prepared again after an underrun (#945),
        // and a hardware device was unplugged.
        assert!(ends_the_output(&alsa(
            "snd_pcm_poll_descriptors_revents",
            -EIO
        )));
        assert!(ends_the_output(&alsa("snd_pcm_prepare", EIO)));
        assert!(ends_the_output(&alsa("snd_pcm_writei", ENODEV)));
        // Only that wait: the same call failing otherwise, or another call failing with EIO, ends
        // the output.
        assert!(ends_the_output(&alsa("snd_pcm_avail_delay", ENODEV)));
        assert!(ends_the_output(&alsa("snd_pcm_avail", EIO)));
    }

    /// The output devices that ALSA lists here, as the fallback of `try_default_with_callback`
    /// gets them. alsa-lib's own configuration defines the `null` device.
    #[cfg(target_os = "linux")]
    #[test]
    fn of_the_alsa_output_devices_only_null_is_passed_over() {
        use rodio::cpal::traits::HostTrait;

        let passed_over: Vec<String> = cpal::default_host()
            .output_devices()
            .expect("ALSA lists its output devices")
            .filter(discards_samples)
            .map(|device| {
                device
                    .id()
                    .expect("an ALSA device has an id")
                    .id()
                    .to_owned()
            })
            .collect();
        assert_eq!(passed_over, ["null"]);
    }

    #[test]
    fn on_linux_the_output_asks_for_periods_of_50_ms_as_far_as_the_device_takes_them() {
        let range = |min, max| cpal::SupportedBufferSize::Range { min, max };
        let config = |rate, buffer_size| {
            stream_config(&SupportedStreamConfig::new(
                2,
                rate,
                buffer_size,
                cpal::SampleFormat::F32,
            ))
        };
        let periods_of = |frames| {
            if cfg!(target_os = "linux") {
                cpal::BufferSize::Fixed(frames)
            } else {
                cpal::BufferSize::Default
            }
        };

        let asked = |rate, buffer_size| config(rate, buffer_size).buffer_size;

        assert_eq!(asked(48_000, range(1, 1 << 20)), periods_of(2400));
        assert_eq!(asked(44_100, range(1, 1 << 20)), periods_of(2205));
        // A device whose periods cannot be that long, or that short.
        assert_eq!(asked(48_000, range(64, 1024)), periods_of(1024));
        assert_eq!(asked(48_000, range(4096, 8192)), periods_of(4096));
        // A device that does not say gets cpal's default.
        let unknown = cpal::SupportedBufferSize::Unknown;
        assert_eq!(asked(48_000, unknown), cpal::BufferSize::Default);
        // The rest is the device's default config.
        let chosen = config(48_000, range(1, 1 << 20));
        assert_eq!((chosen.channels, chosen.sample_rate), (2, 48_000));
    }
}
