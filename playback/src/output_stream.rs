use std::sync::{Arc, Weak};

use rodio::cpal::Sample;
use rodio::cpal::traits::{HostTrait, StreamTrait};
use rodio::dynamic_mixer::{self, DynamicMixerController};
use rodio::source::Source;
use rodio::{DeviceTrait, SupportedStreamConfig, cpal};
use rodio::{PlayError, StreamError};

pub struct RuneOutputStream {
    mixer: Arc<DynamicMixerController<f32>>,
    _stream: cpal::Stream,
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
        let out = Self { mixer, _stream };
        let handle = RuneOutputStreamHandle {
            mixer: Arc::downgrade(&out.mixer),
        };
        Ok((out, handle))
    }

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
