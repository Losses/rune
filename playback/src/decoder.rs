use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rodio::Decoder as RodioDecoder;
use rodio::Source;
use rodio::source::{self, SeekError};
use std::io;
use symphonia::core::{
    audio::{AudioBufferRef, SampleBuffer, SignalSpec},
    codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, SeekedTo},
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::{Instantiate, QueryDescriptor},
    units::{self, Time},
};
use symphonia::default as symphonia_default;
use symphonia_default::formats::OggReader;

use crate::player::UnavailableReason;

const MAX_DECODE_RETRIES: usize = 3;

/// The result of refilling the decoder's sample buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FillOutcome {
    /// A non-empty packet was decoded.
    Filled,
    /// The container reported ordinary EOF during this refill.
    EndOfStream,
    /// The refill failed for another reason, such as a read error or exhausted decode retries.
    Failed,
}

/// Probes the container and reports whether Symphonia selected its Ogg reader. The extension hint
/// is deliberately not used: a file can have no extension, a misleading one, or `ogv`.
fn probe_format(
    mut mss: MediaSourceStream,
    format_opts: &FormatOptions,
    metadata_opts: &MetadataOptions,
) -> Result<(Box<dyn FormatReader>, bool)> {
    let probe = symphonia_default::get_probe();
    let ogg = OggReader::query()[0].inst;
    loop {
        match probe
            .next(&mut mss)
            .map_err(|e| anyhow::Error::new(e).context("Failed to probe audio stream"))?
        {
            Instantiate::Format(format) => {
                let is_ogg = match ogg {
                    Instantiate::Format(ogg) => std::ptr::fn_addr_eq(format, ogg),
                    _ => false,
                };
                let format = format(mss, format_opts).map_err(|e| {
                    anyhow::Error::new(e).context("Failed to instantiate audio format")
                })?;
                return Ok((format, is_ogg));
            }
            Instantiate::Metadata(metadata) => {
                let mut reader = metadata(metadata_opts);
                reader
                    .read_all(&mut mss)
                    .context("Failed to read audio metadata")?;
            }
        }
    }
}

/// Scans the Ogg page headers to tell an ordinary single-stream Ogg from a chained one.
/// Symphonia 0.5.4 does not expose the physical-stream count. A BOS page after the first non-BOS
/// page starts another physical stream; all BOS pages of a multiplexed single stream come first.
fn detect_ogg_layout(mss: &mut MediaSourceStream) -> Result<bool> {
    mss.seek(SeekFrom::Start(0))?;
    let mut is_ogg = false;
    let mut saw_bos = false;
    let mut saw_non_bos = false;
    let mut single_stream = true;
    loop {
        let mut header = [0u8; 27];
        match mss.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
            Err(_) => {
                single_stream = false;
                break;
            }
        }
        if &header[0..4] != b"OggS" {
            single_stream = false;
            break;
        }
        is_ogg = true;
        let header_type = header[5];
        let segments = header[26] as usize;
        let mut table = vec![0u8; segments];
        if mss.read_exact(&mut table).is_err() {
            single_stream = false;
            break;
        }
        let data_len: usize = table.iter().map(|&size| size as usize).sum();
        if header_type & 0x02 != 0 {
            if saw_non_bos {
                single_stream = false;
                break;
            }
            saw_bos = true;
        } else {
            saw_non_bos = true;
        }
        if mss.seek(SeekFrom::Current(data_len as i64)).is_err() {
            single_stream = false;
            break;
        }
    }
    mss.seek(SeekFrom::Start(0))?;
    Ok(is_ogg && saw_bos && single_stream)
}

struct SharedSourceMedia<R> {
    inner: Arc<Mutex<R>>,
    io_error: Arc<Mutex<Option<ErrorKind>>>,
    len: Option<u64>,
}

impl<R: Read + Seek> SharedSourceMedia<R> {
    fn new(
        inner: Arc<Mutex<R>>,
        io_error: Arc<Mutex<Option<ErrorKind>>>,
        known_len: Option<u64>,
    ) -> Self {
        let len = known_len.or_else(|| {
            let mut guard = inner.lock().unwrap();
            let current = match guard.stream_position() {
                Ok(position) => Some(position),
                Err(error) => {
                    *io_error.lock().unwrap() = Some(error.kind());
                    None
                }
            };
            let end = match guard.seek(SeekFrom::End(0)) {
                Ok(position) => Some(position),
                Err(error) => {
                    *io_error.lock().unwrap() = Some(error.kind());
                    None
                }
            };
            if let Some(pos) = current {
                let _ = guard.seek(SeekFrom::Start(pos));
            }
            end
        });
        Self {
            inner,
            io_error,
            len,
        }
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
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let result = self.inner.lock().unwrap().read(buf);
        if let Err(error) = &result {
            *self.io_error.lock().unwrap() = Some(error.kind());
        }
        result
    }
}

impl<R: Read + Seek + Send + Sync> Seek for SharedSourceMedia<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let result = self.inner.lock().unwrap().seek(pos);
        if let Err(error) = &result {
            *self.io_error.lock().unwrap() = Some(error.kind());
        }
        result
    }
}

pub struct SymphoniaDecoder {
    decoder: Box<dyn Decoder>,
    current_frame_offset: usize,
    format: Box<dyn FormatReader>,
    /// The duration of the physical streams selected so far, when all of their durations are
    /// known.
    total_duration: Option<Time>,
    /// Where the current physical stream starts in the chained file.
    stream_start: Option<Time>,
    /// The duration of the current physical stream, when known.
    stream_duration: Option<Time>,
    /// How many physical streams have been selected. The first stream is zero.
    stream_index: usize,
    /// Whether the container is Ogg, where a reset starts a new physical stream whose duration
    /// adds to the file's duration. Other containers may report a changed track list instead.
    is_ogg: bool,
    /// Whether the Ogg container has one physical stream. Its end is a confirmed file end even
    /// before the decoder reaches EOF, so an end-bound seek keeps the old clamping behavior.
    single_ogg: bool,
    /// Whether the container has reported ordinary EOF. For Ogg, this confirms that the current
    /// physical stream is the last one, so a seek at its end can be clamped instead of rejected.
    at_end: bool,
    /// Set when a reset could not be continued. The reader and decoder may then be on different
    /// streams, so the source must not be read or seeked again.
    broken: bool,
    /// Where a decoder failure is recorded for the player's progress reporting.
    failure: Option<Arc<Mutex<Option<UnavailableReason>>>>,
    buffer: SampleBuffer<i16>,
    spec: SignalSpec,
    track_id: u32,
}

impl SymphoniaDecoder {
    pub fn new(mss: MediaSourceStream, _extension: Option<&str>) -> Result<Self> {
        let format_opts = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let metadata_opts = MetadataOptions::default();

        let mut mss = mss;
        let single_stream = detect_ogg_layout(&mut mss)?;
        let (mut format, is_ogg) = probe_format(mss, &format_opts, &metadata_opts)?;
        let single_ogg = is_ogg && single_stream;

        let track = format
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
        let stream_duration = total_duration;

        let mut decode_errors = 0;
        let decoded = loop {
            let current_frame = match format.next_packet() {
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

        let mut this = Self {
            decoder,
            current_frame_offset: 0,
            format,
            total_duration,
            stream_start: Some(Time::default()),
            stream_duration,
            stream_index: 0,
            is_ogg,
            single_ogg,
            at_end: false,
            broken: false,
            buffer,
            spec,
            track_id,
            failure: None,
        };
        // Some codecs (e.g. Vorbis) emit an empty first packet; an empty
        // buffer would make `current_frame_len` report end of stream.
        if this.buffer.is_empty() {
            this.decode_next_buffer();
        }

        Ok(this)
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
                match self.format.next_packet() {
                    Ok(pkt) if pkt.track_id() == self.track_id => break pkt,
                    Ok(_) => continue,
                    Err(Error::ResetRequired) => {
                        if !self.switch_stream() {
                            self.mark_broken();
                            return Err(unsupported_seek());
                        }
                        samples_to_pass = 0;
                    }
                    Err(e) => return Err(source::SeekError::Other(Box::new(e))),
                }
            };
            if candidate.dur() > samples_to_pass {
                break candidate;
            } else {
                samples_to_pass -= candidate.dur();
            }
        };

        let mut decoded = self.decoder.decode(&packet);
        let mut decode_errors = 0;
        while decoded.is_err() && decode_errors < MAX_DECODE_RETRIES {
            let packet = loop {
                match self.format.next_packet() {
                    Ok(pkt) if pkt.track_id() == self.track_id => break pkt,
                    Ok(_) => continue,
                    Err(Error::ResetRequired) => {
                        if !self.switch_stream() {
                            self.mark_broken();
                            return Err(unsupported_seek());
                        }
                        samples_to_pass = 0;
                        // The new stream gets its own retry budget.
                        decode_errors = 0;
                    }
                    Err(e) => return Err(source::SeekError::Other(Box::new(e))),
                }
            };
            decoded = self.decoder.decode(&packet);
            if decoded.is_err() {
                decode_errors += 1;
            }
        }

        let decoded = decoded.map_err(|e| source::SeekError::Other(Box::new(e)))?;
        decoded.spec().clone_into(&mut self.spec);
        self.buffer = Self::get_buffer(decoded, &self.spec);
        self.current_frame_offset = samples_to_pass as usize * self.channels() as usize;
        // Some codecs (e.g. Vorbis) emit an empty first packet after a reset. Mid-track, a seek
        // that stops on it would report success with an empty buffer, which consumers treat as
        // the end, so decode the next packet. Only EOF from this refill is a valid empty result;
        // a read error or exhausted decode retries must not be hidden by an earlier EOF.
        if self.buffer.is_empty() {
            match self.decode_next_buffer() {
                FillOutcome::Filled => {}
                FillOutcome::EndOfStream => {}
                FillOutcome::Failed => return Err(unsupported_seek()),
            }
        }
        Ok(())
    }

    /// The position within the current physical stream for a file-relative `pos`. A position in
    /// another physical stream is an explicit error: this decoder cannot select that stream.
    fn seek_position(&self, pos: Duration) -> Result<Duration, source::SeekError> {
        if self.broken {
            return Err(unsupported_seek());
        }
        let Some(start) = self.stream_start else {
            return Err(unsupported_seek());
        };
        let start = Duration::from(start);
        if pos < start {
            return Err(unsupported_seek());
        }
        let Some(duration) = self.stream_duration else {
            // Without a declared duration there is no safe upper bound. For an Ogg chain the
            // requested position may belong to a stream this decoder cannot select.
            if self.is_ogg {
                return Err(unresolved_ogg_seek());
            }
            if self.stream_index > 0 {
                return Err(unsupported_seek());
            }
            // A non-chained format without a declared duration keeps the old unconstrained
            // seek.
            return Ok(pos);
        };
        let duration = Duration::from(duration);
        let end = start.saturating_add(duration);
        if pos >= end {
            // For Ogg, the current stream may be followed by another physical stream, so its
            // end is a boundary this decoder cannot resolve until the container has reported
            // EOF. Once EOF confirms the final stream, clamp a seek at or past its end.
            if self.is_ogg && !self.single_ogg && !self.at_end {
                return Err(unresolved_ogg_seek());
            }
            if self.total_duration.map(Duration::from) != Some(end) {
                return Err(unsupported_seek());
            }
            return Ok(duration.saturating_sub(Duration::from_micros(100)));
        }
        Ok(pos - start)
    }
}

impl SymphoniaDecoder {
    /// Marks the source as no longer readable after a reset that cannot be continued. The player
    /// observes the recorded reason when the output runs dry and reports the track as unavailable
    /// instead of advancing the playlist as if it had ended normally.
    fn mark_broken(&mut self) {
        self.broken = true;
        self.current_frame_offset = self.buffer.len();
        // The rest of the chain is unknown now, so do not keep reporting the duration of the
        // part that happened to play.
        self.total_duration = None;
        if let Some(failure) = &self.failure
            && let Ok(mut reason) = failure.lock()
        {
            reason.get_or_insert(UnavailableReason::Unreadable);
        }
    }

    /// After a reset, Symphonia has replaced the track list. Continue with its first supported
    /// audio track if it can feed the same output format; otherwise log why and stop.
    fn switch_stream(&mut self) -> bool {
        let Some(track) = self
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        else {
            log::error!("the next stream has no supported audio track");
            return false;
        };
        let track_id = track.id;
        let params = track.codec_params.clone();
        if params.sample_rate != Some(self.spec.rate) {
            log::error!(
                "the next stream changes the sample rate from {} Hz to {:?}, which is not supported",
                self.spec.rate,
                params.sample_rate
            );
            return false;
        }
        if params.channels != Some(self.spec.channels) {
            // The count matching is not enough: a different layout with the same count would
            // send each sample to the wrong channel.
            log::error!(
                "the next stream changes the channel layout from {:?} to {:?}, which is not supported",
                self.spec.channels,
                params.channels
            );
            return false;
        }
        let next_duration = params
            .time_base
            .zip(params.n_frames)
            .map(|(base, frames)| base.calc_time(frames));
        let (next_start, next_total) = if self.is_ogg {
            let next_start = match (self.stream_start, self.stream_duration) {
                (Some(start), Some(duration)) => Some(add_time(start, duration)),
                _ => None,
            };
            let next_total = match (self.total_duration, next_duration) {
                (Some(total), Some(duration)) => Some(add_time(total, duration)),
                _ => None,
            };
            (next_start, next_total)
        } else {
            // A compatible reset in a non-chained container does not move the file's start, so
            // keep the offset that lets the current stream be seeked within.
            (self.stream_start, None)
        };
        match fsio_media_source::get_codecs().make(&params, &DecoderOptions::default()) {
            Ok(decoder) => {
                self.decoder = decoder;
                self.track_id = track_id;
                self.stream_start = next_start;
                self.stream_duration = next_duration;
                self.stream_index += 1;
                self.total_duration = next_total;
                true
            }
            Err(e) => {
                log::error!("failed to create a decoder for the next stream: {e}");
                false
            }
        }
    }

    /// Decodes the next non-empty packet of the selected track into `buffer`.
    fn decode_next_buffer(&mut self) -> FillOutcome {
        if self.broken {
            return FillOutcome::Failed;
        }
        let mut decode_errors = 0;
        loop {
            let packet = loop {
                match self.format.next_packet() {
                    Ok(pkt) if pkt.track_id() == self.track_id => break pkt,
                    Ok(_) => continue,
                    Err(Error::ResetRequired) => {
                        if !self.switch_stream() {
                            self.mark_broken();
                            return FillOutcome::Failed;
                        }
                        // The new stream gets its own retry budget.
                        decode_errors = 0;
                    }
                    Err(Error::IoError(e)) if e.kind() == ErrorKind::UnexpectedEof => {
                        self.at_end = true;
                        return FillOutcome::EndOfStream;
                    }
                    Err(e) => {
                        log::error!("failed to read the next packet: {e}");
                        return FillOutcome::Failed;
                    }
                }
            };

            match self.decoder.decode(&packet) {
                Ok(decoded) => {
                    if decoded.frames() == 0 {
                        continue;
                    }
                    decoded.spec().clone_into(&mut self.spec);
                    self.buffer = Self::get_buffer(decoded, &self.spec);
                    self.current_frame_offset = 0;
                    return FillOutcome::Filled;
                }
                Err(_) => {
                    decode_errors += 1;
                    if decode_errors > MAX_DECODE_RETRIES {
                        return FillOutcome::Failed;
                    }
                }
            }
        }
    }
}

fn add_time(a: Time, b: Time) -> Time {
    let mut seconds = a.seconds.saturating_add(b.seconds);
    let mut frac = a.frac + b.frac;
    if frac >= 1.0 {
        frac -= 1.0;
        seconds = seconds.saturating_add(1);
    }
    Time::new(seconds, frac)
}

/// A seek that would leave the current physical stream. The decoder cannot select another
/// physical stream, so the caller gets an explicit error instead of a wrong position.
fn unsupported_seek() -> source::SeekError {
    source::SeekError::NotSupported {
        underlying_source: "a different physical stream",
    }
}

/// A seek at the end of an Ogg stream before the container has reported EOF. The reader may still
/// find another physical stream, so this is not yet a confirmed final end.
fn unresolved_ogg_seek() -> source::SeekError {
    source::SeekError::NotSupported {
        underlying_source: "an unresolved Ogg physical-stream boundary",
    }
}

impl Source for SymphoniaDecoder {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        if self.broken {
            return Some(0);
        }
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
        self.total_duration.map(Duration::from)
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), source::SeekError> {
        let relative = self.seek_position(pos)?;
        let time = Time::from(relative);

        let channels = self.channels() as usize;
        let to_skip = if channels > 0 {
            self.current_frame_offset % channels
        } else {
            0
        };

        let seek_res = match self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time,
                track_id: None,
            },
        ) {
            Ok(seek_res) => seek_res,
            // The seek crossed into a new physical stream while Symphonia was reading pages.
            // The reader and decoder no longer agree, so the source must not report success or
            // keep reading.
            Err(Error::ResetRequired) => {
                self.mark_broken();
                return Err(unsupported_seek());
            }
            Err(e) => return Err(source::SeekError::Other(Box::new(e))),
        };

        // A seek is discontinuous with the last decoded packet. Stateful codecs keep overlap or
        // reservoir data from the old position, so Symphonia requires a reset before the
        // decoder sees packets from the new one.
        self.decoder.reset();

        self.refine_position(seek_res)?;
        self.current_frame_offset += to_skip;

        Ok(())
    }
}

impl Iterator for SymphoniaDecoder {
    type Item = i16;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.broken {
            return None;
        }
        if self.current_frame_offset >= self.buffer.len()
            && self.decode_next_buffer() != FillOutcome::Filled
        {
            return None;
        }

        let sample = *self.buffer.samples().get(self.current_frame_offset)?;
        self.current_frame_offset += 1;

        // Refill eagerly so `current_frame_len` never reports `Some(0)` at a
        // packet boundary, which consumers (e.g. `RuneBuffered`) treat as the
        // end of the stream.
        if self.current_frame_offset >= self.buffer.len() {
            self.decode_next_buffer();
        }

        Some(sample)
    }
}

pub struct RuneDecoder<R: Read + Seek + Send + Sync + 'static> {
    inner: DecoderInner<R>,
}

enum DecoderInner<R: Read + Seek + Send + Sync + 'static> {
    Symphonia(SymphoniaDecoder),
    Rodio(Box<RodioDecoder<R>>),
}

impl<R: Read + Seek + Send + Sync + 'static> RuneDecoder<R> {
    pub fn new(reader: R, extension: Option<&str>) -> Result<Self> {
        Self::new_with_length(reader, extension, None)
    }

    /// Creates a decoder while preserving streaming reads when the container
    /// length is already known by the remote source.
    pub fn new_with_length(
        reader: R,
        extension: Option<&str>,
        known_len: Option<u64>,
    ) -> Result<Self> {
        let shared = Arc::new(Mutex::new(reader));
        let io_error = Arc::new(Mutex::new(None));
        let media_source = SharedSourceMedia::new(shared.clone(), io_error.clone(), known_len);
        let mss = MediaSourceStream::new(Box::new(media_source), Default::default());

        match SymphoniaDecoder::new(mss, extension) {
            Ok(decoder) => Ok(Self {
                inner: DecoderInner::Symphonia(decoder),
            }),
            Err(e) => {
                log::debug!("SymphoniaDecoder failed: {e:?}, falling back to rodio::Decoder");
                if let Some(kind) = *io_error.lock().unwrap() {
                    return Err(e.context(format!("audio source I/O failed: {kind}")));
                }
                // Reset seek position
                let mut guard = shared.lock().unwrap();
                let _ = guard.seek(SeekFrom::Start(0));
                drop(guard);

                let unwrapped = Arc::try_unwrap(shared)
                    .map(|mutex| mutex.into_inner().unwrap())
                    .unwrap_or_else(|_| panic!("Failed to unwrap shared reader"));
                let decoder = RodioDecoder::new(unwrapped)
                    .map_err(|err| anyhow!("All decoders failed: {err}"))?;
                Ok(Self {
                    inner: DecoderInner::Rodio(Box::new(decoder)),
                })
            }
        }
    }

    /// Records a decoder failure where the player's progress reporting can see it.
    pub(crate) fn set_failure_sink(&mut self, failure: Arc<Mutex<Option<UnavailableReason>>>) {
        if let DecoderInner::Symphonia(decoder) = &mut self.inner {
            decoder.failure = Some(failure);
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
    use crate::player::UnavailableReason;
    use std::borrow::Cow;
    use std::fs::File;
    use std::io;
    use std::io::Cursor;
    use std::io::{BufReader, Error as IoError};
    use std::panic;
    use std::path::{Path, PathBuf};
    use symphonia::core::audio::{AudioBuffer, Channels, Signal};
    use symphonia::core::codecs::{CodecDescriptor, CodecParameters, FinalizeResult};
    use symphonia::core::formats::Packet;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/chained_ogg")
            .join(name)
    }

    /// Writes `parts` one after another into a temporary Ogg file, as a chained physical
    /// stream, and returns its path. `remove` deletes the folder again.
    fn chained_ogg(name: &str, parts: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rune_{name}_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chained.ogg");
        let mut bytes = Vec::new();
        for part in parts {
            bytes.extend_from_slice(&std::fs::read(fixture(part)).unwrap());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn remove(path: &Path) {
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    /// The frequency of `samples` at `rate`, from rising zero crossings.
    fn frequency(samples: &[i16], rate: u32) -> f64 {
        let crossings = samples
            .windows(2)
            .filter(|pair| pair[0] <= 0 && pair[1] > 0)
            .count();
        crossings as f64 * f64::from(rate) / samples.len() as f64
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DecoderCall {
        Decode,
        Reset,
    }

    /// A decoder that records the calls made to it, so a test can check that a seek resets it
    /// before the first post-seek decode.
    struct RecordingDecoder {
        calls: Arc<Mutex<Vec<DecoderCall>>>,
        params: CodecParameters,
        last: AudioBuffer<f32>,
    }

    impl Decoder for RecordingDecoder {
        fn try_new(_: &CodecParameters, _: &DecoderOptions) -> std::result::Result<Self, Error> {
            Err(Error::Unsupported(
                "the recording decoder is built directly",
            ))
        }

        fn supported_codecs() -> &'static [CodecDescriptor] {
            &[]
        }

        fn reset(&mut self) {
            self.calls.lock().unwrap().push(DecoderCall::Reset);
        }

        fn codec_params(&self) -> &CodecParameters {
            &self.params
        }

        fn decode(&mut self, _packet: &Packet) -> std::result::Result<AudioBufferRef<'_>, Error> {
            self.calls.lock().unwrap().push(DecoderCall::Decode);
            Ok(AudioBufferRef::F32(Cow::Borrowed(&self.last)))
        }

        fn finalize(&mut self) -> FinalizeResult {
            FinalizeResult { verify_ok: None }
        }

        fn last_decoded(&self) -> AudioBufferRef<'_> {
            AudioBufferRef::F32(Cow::Borrowed(&self.last))
        }
    }

    /// A decoder whose first post-reset decode is an empty priming buffer and whose later decodes
    /// fail, so a failed refill can be told apart from a real end of stream.
    struct EmptyThenErrorDecoder {
        params: CodecParameters,
        empty: AudioBuffer<f32>,
        decodes: usize,
    }

    impl Decoder for EmptyThenErrorDecoder {
        fn try_new(_: &CodecParameters, _: &DecoderOptions) -> std::result::Result<Self, Error> {
            Err(Error::Unsupported("the empty decoder is built directly"))
        }

        fn supported_codecs() -> &'static [CodecDescriptor] {
            &[]
        }

        fn reset(&mut self) {
            self.decodes = 0;
        }

        fn codec_params(&self) -> &CodecParameters {
            &self.params
        }

        fn decode(&mut self, _packet: &Packet) -> std::result::Result<AudioBufferRef<'_>, Error> {
            self.decodes += 1;
            if self.decodes == 1 {
                Ok(AudioBufferRef::F32(Cow::Borrowed(&self.empty)))
            } else {
                Err(Error::DecodeError("test decode failure"))
            }
        }

        fn finalize(&mut self) -> FinalizeResult {
            FinalizeResult { verify_ok: None }
        }

        fn last_decoded(&self) -> AudioBufferRef<'_> {
            AudioBufferRef::F32(Cow::Borrowed(&self.empty))
        }
    }

    /// Builds a `SymphoniaDecoder` over a real Ogg file, with a recording decoder in place of the
    /// codec so the test can see the calls a seek makes.
    fn recording_decoder(calls: Arc<Mutex<Vec<DecoderCall>>>) -> SymphoniaDecoder {
        let file = File::open(fixture("tone_440_44100.ogg")).expect("open ogg file");
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut decoder =
            SymphoniaDecoder::new(mss, Some("ogg")).expect("create symphonia decoder");

        let spec = SignalSpec::new(44_100, Channels::FRONT_LEFT);
        let mut last = AudioBuffer::<f32>::new(units::Duration::from(4u64), spec);
        last.render_reserved(Some(4));
        for (index, sample) in last.chan_mut(0).iter_mut().enumerate() {
            *sample = index as f32 / 4.0;
        }
        decoder.decoder = Box::new(RecordingDecoder {
            calls,
            params: CodecParameters::default(),
            last,
        });
        decoder
    }

    struct RevokedReader;
    impl Read for RevokedReader {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(IoError::new(
                ErrorKind::PermissionDenied,
                "audio access revoked",
            ))
        }
    }
    impl Seek for RevokedReader {
        fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
            Err(IoError::new(
                ErrorKind::PermissionDenied,
                "audio access revoked",
            ))
        }
    }

    #[test]
    fn revoked_reader_io_error_does_not_panic_during_decoder_fallback() {
        let result = panic::catch_unwind(|| RuneDecoder::new(RevokedReader, Some("ogg")));
        assert!(
            result.is_ok(),
            "revoked stream must not panic in decoder fallback"
        );
        assert!(result.unwrap().is_err(), "revoked stream must fail to load");
    }

    #[test]
    fn non_audio_file_is_classified_as_unreadable() {
        let error = RuneDecoder::new(Cursor::new(b"plain text, not audio"), Some("ogg"))
            .err()
            .expect("non-audio data must fail probing");
        assert_eq!(
            UnavailableReason::classify(&error),
            UnavailableReason::Unreadable
        );
    }

    #[test]
    fn test_decode_and_seek_opus() {
        let path = "/tmp/test_startup.opus";
        if !Path::new(path).exists() {
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
        if !Path::new(path).exists() {
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
    fn test_buffered_decodes_past_first_packet() {
        let path = "../assets/startup_0.ogg";
        if !Path::new(path).exists() {
            return;
        }

        let file = File::open(path).expect("open ogg file");
        let decoder = RuneDecoder::new(BufReader::new(file), Some("ogg"))
            .expect("create rune decoder for ogg");
        let expected = decoder.total_duration().expect("duration").as_secs_f64()
            * decoder.sample_rate() as f64
            * decoder.channels() as f64;

        let decoded = rune_buffered(decoder).count() as f64;
        assert!(
            (decoded - expected).abs() / expected < 0.01,
            "decoded {decoded} samples, expected about {expected}"
        );
    }

    #[test]
    fn test_decode_vorbis() {
        let path = "../assets/startup_0.ogg";
        if !Path::new(path).exists() {
            return;
        }

        let file = File::open(path).expect("open ogg file");
        let mut decoder = RuneDecoder::new(BufReader::new(file), Some("ogg"))
            .expect("create rune decoder for ogg");

        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.sample_rate(), 44100);
        assert!(decoder.next().is_some());
    }

    #[test]
    fn a_chained_ogg_plays_both_streams() {
        let path = chained_ogg(
            "chained_ogg_two_streams",
            &["tone_440_44100.ogg", "tone_660_44100.ogg"],
        );
        let file = File::open(&path).expect("open chained ogg");
        let decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");
        let mut source = rune_buffered(decoder);
        let samples: Vec<i16> = source.by_ref().collect();
        assert_eq!(source.next(), None, "the chained file must end at EOF");
        remove(&path);

        // The first stream is 2 s of 440 Hz and the second 2 s of 660 Hz. Vorbis trims a
        // few samples at the stream edges, so leave the boundary and the very end out of the
        // measured windows.
        assert!(
            samples.len() > 150_000,
            "decoded {} samples, so the second stream was not reached",
            samples.len()
        );
        let first = frequency(&samples[..20_000], 44_100);
        let second = frequency(&samples[samples.len() - 20_000..], 44_100);
        assert!((first - 440.0).abs() < 10.0, "first stream is {first} Hz");
        assert!(
            (second - 660.0).abs() < 10.0,
            "second stream is {second} Hz"
        );
    }

    #[test]
    fn a_single_stream_ogg_ends_at_eof() {
        let path = fixture("tone_440_44100.ogg");
        let file = File::open(&path).expect("open single-stream ogg");
        let decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create single decoder");
        let mut source = rune_buffered(decoder);
        let samples: Vec<i16> = source.by_ref().collect();
        assert_eq!(source.next(), None, "a single stream must end at EOF");

        assert!(
            (samples.len() as f64 - 88_200.0).abs() < 2_000.0,
            "decoded {} samples",
            samples.len()
        );
        let tone = frequency(&samples[..20_000], 44_100);
        assert!((tone - 440.0).abs() < 10.0, "stream is {tone} Hz");
    }

    #[test]
    fn a_chained_ogg_with_a_different_sample_rate_stops_at_the_reset() {
        let path = chained_ogg(
            "chained_ogg_rate_change",
            &["tone_440_44100.ogg", "tone_660_48000.ogg"],
        );
        let file = File::open(&path).expect("open rate-changing chained ogg");
        let failure = Arc::new(Mutex::new(None));
        let mut decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");
        decoder.set_failure_sink(failure.clone());
        let mut source = rune_buffered(decoder);
        let samples: Vec<i16> = source.by_ref().collect();
        assert_eq!(source.next(), None, "the unsupported file must end");
        remove(&path);

        // The second stream cannot continue this source's output rate. Playback stops after the
        // first stream instead of decoding the second one at the wrong rate, reports a decoder
        // failure, and does not claim a complete duration.
        assert!(
            (samples.len() as f64 - 88_200.0).abs() < 2_000.0,
            "decoded {} samples",
            samples.len()
        );
        let tone = frequency(&samples[..20_000], 44_100);
        assert!((tone - 440.0).abs() < 10.0, "stream is {tone} Hz");
        assert_eq!(
            *failure.lock().unwrap(),
            Some(UnavailableReason::Unreadable),
            "an unsupported chained stream must reach the player as a failure"
        );
        assert!(
            source.total_duration().is_none(),
            "an unsupported chain must not report the first stream's duration as complete"
        );
    }

    #[test]
    fn a_three_segment_ogg_with_a_repeated_serial_is_not_single_stream() {
        let path = chained_ogg(
            "chained_ogg_three_segments",
            &[
                "tone_440_44100.ogg",
                "tone_660_44100.ogg",
                "tone_440_44100.ogg",
            ],
        );
        let file = File::open(&path).expect("open three-segment chained ogg");
        let mut decoder =
            RuneDecoder::new(BufReader::new(file), None).expect("create chained decoder");

        let error = decoder
            .try_seek(Duration::from_secs(3))
            .expect_err("the middle stream is behind an unresolved physical-stream boundary");
        assert!(
            matches!(error, SeekError::NotSupported { .. }),
            "unexpected seek error: {error:?}"
        );

        let samples: Vec<i16> = decoder.by_ref().collect();
        remove(&path);
        assert!(
            samples.len() > 250_000,
            "decoded {} samples, so the third stream was not reached",
            samples.len()
        );
        let duration = decoder
            .total_duration()
            .expect("the three-stream chain has a duration");
        assert!(
            (duration.as_secs_f64() - 6.0).abs() < 0.1,
            "the three-stream chain reports {duration:?}, not 6 s"
        );
    }

    #[test]
    fn a_single_stream_ogg_clamps_an_end_bound_seek() {
        let path = fixture("tone_440_44100.ogg");
        let file = File::open(&path).expect("open single-stream ogg");
        let decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create single decoder");
        let mut source = rune_buffered(decoder);
        let duration = source.total_duration().expect("single stream duration");

        source
            .try_seek(duration)
            .expect("an ordinary single-stream Ogg clamps a seek at its end");
        source
            .try_seek(duration + Duration::from_secs(1))
            .expect("an ordinary single-stream Ogg clamps a seek past its end");
        source
            .try_seek(Duration::ZERO)
            .expect("the single stream can be seeked back to its start");

        let samples: Vec<i16> = source.by_ref().collect();
        assert!(
            (samples.len() as f64 - 88_200.0).abs() < 2_000.0,
            "decoded {} samples",
            samples.len()
        );

        source
            .try_seek(duration)
            .expect("the confirmed end still clamps after EOF");
    }

    #[test]
    fn a_chained_ogg_is_recognized_without_a_helpful_extension() {
        for extension in [None, Some("audio"), Some("ogv"), Some("mp3")] {
            let path = chained_ogg(
                "chained_ogg_container_detection",
                &["tone_440_44100.ogg", "tone_660_44100.ogg"],
            );
            let file = File::open(&path).expect("open chained ogg");
            let mut decoder =
                RuneDecoder::new(BufReader::new(file), extension).expect("create chained decoder");

            let error = decoder.try_seek(Duration::from_secs(3)).expect_err(
                "a cross-stream seek must be rejected regardless of the extension hint",
            );
            assert!(
                matches!(error, SeekError::NotSupported { .. }),
                "unexpected seek error for {extension:?}: {error:?}"
            );

            let samples: Vec<i16> = decoder.by_ref().collect();
            remove(&path);
            assert!(
                samples.len() > 150_000,
                "decoded {} samples for {extension:?}, so the second stream was not reached",
                samples.len()
            );
            let duration = decoder
                .total_duration()
                .expect("the detected Ogg chain has a duration");
            assert!(
                (duration.as_secs_f64() - 4.0).abs() < 0.05,
                "the chain opened as {extension:?} reports {duration:?}, not 4 s"
            );
        }
    }

    #[test]
    fn a_chained_ogg_reports_the_whole_duration() {
        let path = chained_ogg(
            "chained_ogg_duration",
            &["tone_440_44100.ogg", "tone_660_44100.ogg"],
        );
        let file = File::open(&path).expect("open chained ogg");
        let decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");
        let mut source = rune_buffered(decoder);
        let samples: Vec<i16> = source.by_ref().collect();
        remove(&path);

        assert!(
            samples.len() > 150_000,
            "decoded {} samples, so the second stream was not reached",
            samples.len()
        );
        let duration = source.total_duration().expect("the chain has a duration");
        assert!(
            (duration.as_secs_f64() - 4.0).abs() < 0.05,
            "the chained file reports {duration:?}, not 4 s"
        );
    }

    #[test]
    fn a_seek_into_a_later_physical_stream_is_a_controlled_error() {
        let path = chained_ogg(
            "chained_ogg_cross_stream_seek",
            &["tone_440_44100.ogg", "tone_660_44100.ogg"],
        );
        let file = File::open(&path).expect("open chained ogg");
        let mut decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");

        let error = decoder
            .try_seek(Duration::from_secs(3))
            .expect_err("a seek into the second physical stream cannot be satisfied yet");
        match error {
            SeekError::NotSupported { underlying_source } => assert_eq!(
                underlying_source, "an unresolved Ogg physical-stream boundary",
                "a chained boundary must not be reported as a different stream before it is parsed"
            ),
            other => panic!("unexpected seek error: {other:?}"),
        }

        let error = decoder
            .try_seek(Duration::from_secs(2))
            .expect_err("the boundary between the two physical streams must not report success");
        remove(&path);
        assert!(
            matches!(error, SeekError::NotSupported { .. }),
            "unexpected seek error: {error:?}"
        );
    }

    #[test]
    fn a_seek_back_across_a_switched_boundary_is_a_controlled_error() {
        let path = chained_ogg(
            "chained_ogg_back_seek",
            &["tone_440_44100.ogg", "tone_660_44100.ogg"],
        );
        let file = File::open(&path).expect("open chained ogg");
        let decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");
        let mut source = rune_buffered(decoder);
        let samples: Vec<i16> = source.by_ref().collect();
        assert!(
            samples.len() > 150_000,
            "decoded {} samples, so the second stream was not reached",
            samples.len()
        );

        let error = source
            .try_seek(Duration::from_secs(1))
            .expect_err("a seek back into the first physical stream must not report success");
        assert!(
            matches!(error, SeekError::NotSupported { .. }),
            "unexpected seek error: {error:?}"
        );

        source
            .try_seek(Duration::from_secs(3))
            .expect("a seek within the current physical stream is supported");
        let samples: Vec<i16> = source.by_ref().take(20_000).collect();
        remove(&path);
        let tone = frequency(&samples, 44_100);
        assert!((tone - 660.0).abs() < 10.0, "second stream is {tone} Hz");
    }

    #[test]
    fn a_seek_past_the_current_stream_switches_the_decoder() {
        let path = chained_ogg(
            "chained_ogg_refine_boundary",
            &["tone_440_44100.ogg", "tone_660_44100.ogg"],
        );
        let file = File::open(&path).expect("open chained ogg");
        let mut decoder =
            RuneDecoder::new(BufReader::new(file), Some("ogg")).expect("create chained decoder");
        let DecoderInner::Symphonia(s) = &mut decoder.inner else {
            panic!("the fixture must use the Symphonia decoder");
        };

        let track_id = s.track_id;
        s.refine_position(SeekedTo {
            track_id,
            actual_ts: 0,
            required_ts: u64::MAX,
        })
        .expect("a position past the first stream crosses into the second");

        let samples: Vec<i16> = s.by_ref().take(20_000).collect();
        remove(&path);
        let tone = frequency(&samples, 44_100);
        assert!((tone - 660.0).abs() < 10.0, "second stream is {tone} Hz");
    }

    #[test]
    fn a_seek_resets_the_decoder_before_decoding_at_the_new_position() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut decoder = recording_decoder(calls.clone());

        // Consume the packet decoded during construction, then decode one more packet so the
        // seek is discontinuous with a packet this decoder produced.
        let initial = decoder.buffer.len();
        for _ in 0..initial {
            assert!(decoder.next().is_some(), "the initial packet must decode");
        }
        calls.lock().unwrap().clear();

        decoder
            .try_seek(Duration::from_secs(1))
            .expect("seek within the tone");

        assert_eq!(
            *calls.lock().unwrap(),
            vec![DecoderCall::Reset, DecoderCall::Decode],
            "a discontinuous seek must reset the decoder before the first post-seek decode"
        );
    }

    #[test]
    fn a_failed_refill_after_a_priming_packet_is_not_the_end_of_stream() {
        let file = File::open(fixture("tone_440_44100.ogg")).expect("open ogg file");
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut decoder =
            SymphoniaDecoder::new(mss, Some("ogg")).expect("create symphonia decoder");
        while decoder.next().is_some() {}
        assert!(decoder.at_end, "the fixture must have reached EOF");

        let spec = SignalSpec::new(44_100, Channels::FRONT_LEFT);
        let empty = AudioBuffer::<f32>::new(units::Duration::from(4u64), spec);
        decoder.decoder = Box::new(EmptyThenErrorDecoder {
            params: CodecParameters::default(),
            empty,
            decodes: 0,
        });

        let error = decoder
            .try_seek(Duration::from_secs(1))
            .expect_err("a failed refill must not be reported as a successful seek");
        assert!(
            matches!(error, SeekError::NotSupported { .. }),
            "a failed refill must report a seek error: {error:?}"
        );
    }

    #[test]
    fn a_channel_swapped_offset_is_not_a_match() {
        let channels = 2usize;
        let expected_start = 100usize;
        let reference: Vec<i16> = (0..5_000)
            .map(|index| if index % channels == 0 { 100 } else { -100 })
            .collect();
        let actual: Vec<i16> = (0..4_096)
            .map(|index| if index % channels == 0 { -100 } else { 100 })
            .collect();

        let matched = panic::catch_unwind(|| {
            matching_offset(&actual, &reference, expected_start, 8, channels, "test")
        });
        assert!(
            matched.is_err(),
            "a channel-swapped offset must not be accepted"
        );
    }

    fn decode_all(path: &str) -> Vec<i16> {
        let file = File::open(path).expect("open ogg file");
        RuneDecoder::new(BufReader::new(file), Some("ogg"))
            .expect("create rune decoder for ogg")
            .collect()
    }

    /// Finds where `actual` resumes in `reference` near `expected_start`. A decoder reset makes
    /// the first decoded packet prime the codec's overlap, so the resumed position can be up to
    /// one packet later. The samples themselves must still match exactly.
    fn matching_offset(
        actual: &[i16],
        reference: &[i16],
        expected_start: usize,
        max_shift: usize,
        channels: usize,
        context: &str,
    ) -> usize {
        let window = 2_048 * channels;
        assert!(
            actual.len() >= window,
            "{context}: only {} samples after the seek",
            actual.len()
        );
        assert!(channels > 0, "{context}: channels must be positive");
        let low = expected_start.saturating_sub(max_shift);
        let high = (expected_start + max_shift).min(reference.len().saturating_sub(window));
        // A shifted interleaved window can only match at an odd offset by swapping channels.
        // Keep candidates on the expected channel phase.
        let alignment = expected_start % channels;
        let first = low + (alignment + channels - low % channels) % channels;
        let mut best = (i64::MAX, expected_start);
        for start in (first..=high).step_by(channels) {
            let difference: i64 = actual[..window]
                .iter()
                .zip(&reference[start..start + window])
                .map(|(actual, expected)| (*actual as i64 - *expected as i64).abs())
                .sum();
            if difference < best.0 {
                best = (difference, start);
            }
        }
        assert_eq!(
            best.0,
            0,
            "{context}: no exact match within {max_shift} samples of the requested position \
             (best offset {}, mean difference {})",
            best.1 as i64 - expected_start as i64,
            best.0 as f64 / window as f64
        );
        best.1
    }

    fn assert_seek_resumes(
        reference: &[i16],
        seek_to: Duration,
        target: usize,
        take: Option<usize>,
        context: &str,
    ) {
        let path = "../assets/startup_0.ogg";
        let channels = 2usize;
        let rate = 44_100usize;
        let file = File::open(path).expect("open ogg file");
        let mut decoder = RuneDecoder::new(BufReader::new(file), Some("ogg"))
            .expect("create rune decoder for ogg");
        assert_eq!(decoder.channels() as usize, channels);
        assert_eq!(decoder.sample_rate() as usize, rate);

        for _ in 0..rate * channels / 2 {
            assert!(
                decoder.next().is_some(),
                "the first half second must decode"
            );
        }

        decoder.try_seek(seek_to).expect("seek within the track");
        // Leave one maximum Vorbis block so the comparison starts after the packet that primes
        // the reset decoder.
        let settle = 8_192 * channels;
        let after: Vec<i16> = match take {
            Some(frames) => decoder.by_ref().take(settle + frames * channels).collect(),
            None => decoder.by_ref().collect(),
        };
        assert!(
            after.len() > settle,
            "the seek must leave samples to compare"
        );

        let resumed = matching_offset(
            &after[settle..],
            reference,
            target + settle,
            8_192 * channels,
            channels,
            context,
        );
        assert_eq!(
            &after[settle..],
            &reference[resumed..resumed + after.len() - settle],
            "{context} must match the same position decoded from the start"
        );
    }

    #[test]
    fn a_seek_resumes_with_correct_samples_and_channel_phase() {
        let reference = decode_all("../assets/startup_0.ogg");
        assert_seek_resumes(
            &reference,
            Duration::from_secs(5),
            5 * 44_100 * 2,
            Some(4_096),
            "samples after a seek",
        );
    }

    #[test]
    fn a_near_end_seek_resumes_with_correct_samples_and_channel_phase() {
        let reference = decode_all("../assets/startup_0.ogg");
        assert_seek_resumes(
            &reference,
            Duration::from_millis(8_500),
            8_500 * 44_100 / 1_000 * 2,
            None,
            "samples after a near-end seek",
        );
    }
}
