use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rodio::Source;
use rodio::source::{self, SeekError};
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer, SignalSpec},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
    units::{self, Time},
};

const MAX_DECODE_RETRIES: usize = 3;

struct SharedSourceMedia<R> {
    inner: Arc<Mutex<R>>,
    len: Option<u64>,
}

impl<R: Read + Seek> SharedSourceMedia<R> {
    fn new(inner: Arc<Mutex<R>>) -> Self {
        let len = {
            let mut guard = inner.lock().unwrap();
            let current = guard.stream_position().ok();
            let end = guard.seek(SeekFrom::End(0)).ok();
            if let Some(pos) = current {
                let _ = guard.seek(SeekFrom::Start(pos));
            }
            end
        };
        Self { inner, len }
    }
}

impl<R: Read + Seek + Send + Sync> MediaSource for SharedSourceMedia<R> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

impl<R: Read + Seek + Send + Sync> Read for SharedSourceMedia<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.lock().unwrap().read(buf)
    }
}

impl<R: Read + Seek + Send + Sync> Seek for SharedSourceMedia<R> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.lock().unwrap().seek(pos)
    }
}

pub struct SymphoniaDecoder {
    decoder: Box<dyn Decoder>,
    current_frame_offset: usize,
    format: Box<dyn FormatReader>,
    total_duration: Option<Time>,
    buffer: SampleBuffer<i16>,
    spec: SignalSpec,
    track_id: u32,
}

impl SymphoniaDecoder {
    pub fn new(mss: MediaSourceStream, extension: Option<&str>) -> Result<Self> {
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }

        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &format_opts, &metadata_opts)
            .map_err(|e| anyhow!("Failed to probe audio stream: {e}"))?;

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow!("No supported audio track found"))?;

        let track_id = track.id;
        let track_params = track.codec_params.clone();

        let mut decoder = fsio_media_source::get_codecs()
            .make(&track_params, &DecoderOptions::default())
            .map_err(|e| anyhow!("Failed to create decoder for codec: {e}"))?;

        let total_duration = track_params
            .time_base
            .zip(track_params.n_frames)
            .map(|(base, frames)| base.calc_time(frames));

        let mut decode_errors = 0;
        let decoded = loop {
            let current_frame = match probed.format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(_)) => break decoder.last_decoded(),
                Err(e) => bail!("Failed to read initial packet: {e}"),
            };

            if current_frame.track_id() != track_id {
                continue;
            }

            match decoder.decode(&current_frame) {
                Ok(decoded) => break decoded,
                Err(e) => match e {
                    Error::DecodeError(_) => {
                        decode_errors += 1;
                        if decode_errors > MAX_DECODE_RETRIES {
                            bail!("Exceeded max decode retries: {e}");
                        } else {
                            continue;
                        }
                    }
                    _ => bail!("Decode error: {e}"),
                },
            }
        };

        let spec = decoded.spec().to_owned();
        let buffer = Self::get_buffer(decoded, &spec);

        Ok(Self {
            decoder,
            current_frame_offset: 0,
            format: probed.format,
            total_duration,
            buffer,
            spec,
            track_id,
        })
    }

    #[inline]
    fn get_buffer(decoded: AudioBufferRef, spec: &SignalSpec) -> SampleBuffer<i16> {
        let duration = units::Duration::from(decoded.capacity() as u64);
        let mut buffer = SampleBuffer::<i16>::new(duration, *spec);
        buffer.copy_interleaved_ref(decoded);
        buffer
    }

    fn refine_position(&mut self, seek_res: SeekedTo) -> Result<(), source::SeekError> {
        let mut samples_to_pass = seek_res.required_ts.saturating_sub(seek_res.actual_ts);
        let packet = loop {
            let candidate = loop {
                let pkt = self
                    .format
                    .next_packet()
                    .map_err(|e| source::SeekError::Other(Box::new(e)))?;
                if pkt.track_id() == self.track_id {
                    break pkt;
                }
            };
            if candidate.dur() > samples_to_pass {
                break candidate;
            } else {
                samples_to_pass -= candidate.dur();
            }
        };

        let mut decoded = self.decoder.decode(&packet);
        for _ in 0..MAX_DECODE_RETRIES {
            if decoded.is_err() {
                let packet = loop {
                    let pkt = self
                        .format
                        .next_packet()
                        .map_err(|e| source::SeekError::Other(Box::new(e)))?;
                    if pkt.track_id() == self.track_id {
                        break pkt;
                    }
                };
                decoded = self.decoder.decode(&packet);
            }
        }

        let decoded = decoded.map_err(|e| source::SeekError::Other(Box::new(e)))?;
        decoded.spec().clone_into(&mut self.spec);
        self.buffer = Self::get_buffer(decoded, &self.spec);
        self.current_frame_offset = samples_to_pass as usize * self.channels() as usize;
        Ok(())
    }
}

fn skip_back_a_tiny_bit(mut time: Time) -> Time {
    time.frac -= 0.0001;
    if time.frac < 0.0 {
        time.seconds = time.seconds.saturating_sub(1);
        time.frac = 1.0 - time.frac;
    }
    time
}

impl Source for SymphoniaDecoder {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        Some(
            self.buffer
                .samples()
                .len()
                .saturating_sub(self.current_frame_offset),
        )
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.spec.channels.count() as u16
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.spec.rate
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.total_duration
            .map(|Time { seconds, frac }| Duration::new(seconds, (frac * 1_000_000_000.0) as u32))
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), source::SeekError> {
        let seek_beyond_end = self
            .total_duration()
            .is_some_and(|dur| dur.saturating_sub(pos).as_millis() < 1);

        let time = if seek_beyond_end {
            let time = self.total_duration.expect("if guarantees this is Some");
            skip_back_a_tiny_bit(time)
        } else {
            pos.as_secs_f64().into()
        };

        let channels = self.channels() as usize;
        let to_skip = if channels > 0 {
            self.current_frame_offset % channels
        } else {
            0
        };

        let seek_res = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: None,
                },
            )
            .map_err(|e| source::SeekError::Other(Box::new(e)))?;

        self.refine_position(seek_res)?;
        self.current_frame_offset += to_skip;

        Ok(())
    }
}

impl Iterator for SymphoniaDecoder {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.current_frame_offset >= self.buffer.len() {
            let packet = loop {
                let pkt = self.format.next_packet().ok()?;
                if pkt.track_id() == self.track_id {
                    break pkt;
                }
            };
            let mut decoded = self.decoder.decode(&packet);
            for _ in 0..MAX_DECODE_RETRIES {
                if decoded.is_err() {
                    let packet = loop {
                        let pkt = self.format.next_packet().ok()?;
                        if pkt.track_id() == self.track_id {
                            break pkt;
                        }
                    };
                    decoded = self.decoder.decode(&packet);
                }
            }
            let decoded = decoded.ok()?;
            decoded.spec().clone_into(&mut self.spec);
            self.buffer = Self::get_buffer(decoded, &self.spec);
            self.current_frame_offset = 0;
        }

        let sample = *self.buffer.samples().get(self.current_frame_offset)?;
        self.current_frame_offset += 1;
        Some(sample)
    }
}

pub struct RuneDecoder<R: Read + Seek + Send + Sync + 'static> {
    inner: DecoderInner<R>,
}

enum DecoderInner<R: Read + Seek + Send + Sync + 'static> {
    Symphonia(SymphoniaDecoder),
    Rodio(Box<rodio::Decoder<R>>),
}

impl<R: Read + Seek + Send + Sync + 'static> RuneDecoder<R> {
    pub fn new(reader: R, extension: Option<&str>) -> Result<Self> {
        let shared = Arc::new(Mutex::new(reader));
        let media_source = SharedSourceMedia::new(shared.clone());
        let mss = MediaSourceStream::new(Box::new(media_source), Default::default());

        match SymphoniaDecoder::new(mss, extension) {
            Ok(decoder) => Ok(Self {
                inner: DecoderInner::Symphonia(decoder),
            }),
            Err(e) => {
                log::debug!("SymphoniaDecoder failed: {e:?}, falling back to rodio::Decoder");
                // Reset seek position
                let mut guard = shared.lock().unwrap();
                let _ = guard.seek(SeekFrom::Start(0));
                drop(guard);

                let unwrapped = Arc::try_unwrap(shared)
                    .map(|mutex| mutex.into_inner().unwrap())
                    .unwrap_or_else(|_| panic!("Failed to unwrap shared reader"));
                let decoder = rodio::Decoder::new(unwrapped)
                    .map_err(|err| anyhow!("All decoders failed: {err}"))?;
                Ok(Self {
                    inner: DecoderInner::Rodio(Box::new(decoder)),
                })
            }
        }
    }
}

impl<R: Read + Seek + Send + Sync + 'static> Iterator for RuneDecoder<R> {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.inner {
            DecoderInner::Symphonia(s) => s.next(),
            DecoderInner::Rodio(r) => r.next(),
        }
    }
}

impl<R: Read + Seek + Send + Sync + 'static> Source for RuneDecoder<R> {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        match &self.inner {
            DecoderInner::Symphonia(s) => s.current_frame_len(),
            DecoderInner::Rodio(r) => r.current_frame_len(),
        }
    }

    #[inline]
    fn channels(&self) -> u16 {
        match &self.inner {
            DecoderInner::Symphonia(s) => s.channels(),
            DecoderInner::Rodio(r) => r.channels(),
        }
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        match &self.inner {
            DecoderInner::Symphonia(s) => s.sample_rate(),
            DecoderInner::Rodio(r) => r.sample_rate(),
        }
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        match &self.inner {
            DecoderInner::Symphonia(s) => s.total_duration(),
            DecoderInner::Rodio(r) => r.total_duration(),
        }
    }

    #[inline]
    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        match &mut self.inner {
            DecoderInner::Symphonia(s) => s.try_seek(pos),
            DecoderInner::Rodio(r) => r.try_seek(pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffered::rune_buffered;
    use std::fs::File;
    use std::io::BufReader;

    #[test]
    fn test_decode_and_seek_opus() {
        let path = "/tmp/test_startup.opus";
        if !std::path::Path::new(path).exists() {
            return;
        }

        let file = File::open(path).expect("open opus file");
        let decoder = RuneDecoder::new(BufReader::new(file), Some("opus"))
            .expect("create rune decoder for opus");

        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 48000);
        assert!(decoder.total_duration().is_some());

        let mut buffered = rune_buffered(decoder);

        // Read initial samples and check current_samples
        for _ in 0..10 {
            assert!(buffered.next().is_some());
        }
        let samples = buffered.current_samples();
        assert!(samples.is_some(), "Current samples should be available");
        assert_eq!(samples.unwrap().len(), 2);

        // Read more samples
        let mut count = 10;
        for _ in 0..990 {
            if buffered.next().is_some() {
                count += 1;
            }
        }
        assert_eq!(count, 1000);

        // Test seek
        let seek_res = buffered.try_seek(Duration::from_secs(2));
        assert!(seek_res.is_ok(), "Opus seek failed: {:?}", seek_res.err());

        // Read after seek
        let mut count_after = 0;
        for _ in 0..1000 {
            if buffered.next().is_some() {
                count_after += 1;
            }
        }
        assert_eq!(count_after, 1000);
    }

    #[test]
    fn test_decode_and_seek_aac() {
        let path = "/tmp/test_startup.m4a";
        if !std::path::Path::new(path).exists() {
            return;
        }

        let file = File::open(path).expect("open m4a file");
        let decoder = RuneDecoder::new(BufReader::new(file), Some("m4a"))
            .expect("create rune decoder for aac");

        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 44100);
        assert!(decoder.total_duration().is_some());

        let mut buffered = rune_buffered(decoder);

        // Read initial samples and check current_samples
        for _ in 0..10 {
            assert!(buffered.next().is_some());
        }
        let samples = buffered.current_samples();
        assert!(samples.is_some(), "Current samples should be available");
        assert_eq!(samples.unwrap().len(), 2);

        // Read more samples
        let mut count = 10;
        for _ in 0..990 {
            if buffered.next().is_some() {
                count += 1;
            }
        }
        assert_eq!(count, 1000);

        // Test seek
        let seek_res = buffered.try_seek(Duration::from_secs(2));
        assert!(seek_res.is_ok(), "AAC seek failed: {:?}", seek_res.err());

        // Read after seek
        let mut count_after = 0;
        for _ in 0..1000 {
            if buffered.next().is_some() {
                count_after += 1;
            }
        }
        assert_eq!(count_after, 1000);
    }

    #[test]
    fn test_decode_vorbis() {
        let path = "../assets/startup_0.ogg";
        if !std::path::Path::new(path).exists() {
            return;
        }

        let file = File::open(path).expect("open ogg file");
        let mut decoder = RuneDecoder::new(BufReader::new(file), Some("ogg"))
            .expect("create rune decoder for ogg");

        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 44100);
        assert!(decoder.next().is_some());
    }
}
