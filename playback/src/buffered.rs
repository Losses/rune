use rodio::source::SeekError;
use rodio::{ChannelCount, Sample, SampleRate, Source};
use std::time::Duration;

/// Internal function that builds a `RuneBuffered` object.
#[inline]
pub fn rune_buffered<I>(input: I) -> RuneBuffered<I>
where
    I: Source,
{
    RuneBuffered {
        source: input,
        current_frame_data: Vec::new(),
        current_frame_channels: 0,
        position_in_frame: 0,
    }
}

/// A source that proxies to the underlying source while maintaining current frame data
pub struct RuneBuffered<I>
where
    I: Source,
{
    /// The underlying source
    source: I,

    /// Current frame data for current_samples() support
    current_frame_data: Vec<Sample>,

    /// Number of channels in current frame
    current_frame_channels: u16,

    /// The position in number of samples inside current frame
    position_in_frame: usize,
}

impl<I> RuneBuffered<I>
where
    I: Source,
{
    /// Returns all channels of the sample frame containing the current scalar position,
    /// starting at the first channel. Returns `None` when there is no complete buffered frame
    /// at that position.
    pub fn current_samples(&self) -> Option<Vec<Sample>> {
        let channels = self.current_frame_channels as usize;
        if channels == 0 {
            return None;
        }

        // Align the scalar position to the first channel of its sample frame.
        let base_pos = self.position_in_frame - self.position_in_frame % channels;
        self.current_frame_data
            .get(base_pos..base_pos + channels)
            .map(<[_]>::to_vec)
    }

    /// Updates the current frame data from source
    fn update_frame(&mut self) {
        self.current_frame_data.clear();
        self.current_frame_channels = self.source.channels().get();

        let frame_len = self.source.current_span_len().unwrap_or(0);
        if frame_len > 0 {
            self.current_frame_data = (&mut self.source).take(frame_len).collect();
        }

        self.position_in_frame = 0;
    }
}

impl<I> Iterator for RuneBuffered<I>
where
    I: Source,
{
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        // If we've exhausted current frame data, get next frame
        if self.position_in_frame >= self.current_frame_data.len() {
            self.update_frame();
        }

        if self.position_in_frame < self.current_frame_data.len() {
            let sample = self.current_frame_data[self.position_in_frame];
            self.position_in_frame += 1;
            Some(sample)
        } else {
            None
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }
}

impl<I> Source for RuneBuffered<I>
where
    I: Source,
{
    #[inline]
    fn current_span_len(&self) -> Option<usize> {
        self.source.current_span_len()
    }

    #[inline]
    fn channels(&self) -> ChannelCount {
        self.source.channels()
    }

    #[inline]
    fn sample_rate(&self) -> SampleRate {
        self.source.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.source.total_duration()
    }

    #[inline]
    fn try_seek(&mut self, time: Duration) -> Result<(), SeekError> {
        let result = self.source.try_seek(time);

        if result.is_ok() {
            self.update_frame();
        }

        result
    }
}

impl<I> Clone for RuneBuffered<I>
where
    I: Source + Clone,
{
    #[inline]
    fn clone(&self) -> RuneBuffered<I> {
        RuneBuffered {
            source: self.source.clone(),
            current_frame_data: self.current_frame_data.clone(),
            current_frame_channels: self.current_frame_channels,
            position_in_frame: self.position_in_frame,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::decoder::RuneDecoder;

    fn buffered_wav(channels: u16, samples: &[i16]) -> RuneBuffered<RuneDecoder<Cursor<Vec<u8>>>> {
        let rate = 8_000_u32;
        let sample_bytes = channels * 2;
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::from(&b"RIFF"[..]);
        bytes.extend((36 + data_len).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16_u32.to_le_bytes());
        bytes.extend(1_u16.to_le_bytes()); // PCM
        bytes.extend(channels.to_le_bytes());
        bytes.extend(rate.to_le_bytes());
        bytes.extend((rate * u32::from(sample_bytes)).to_le_bytes());
        bytes.extend(sample_bytes.to_le_bytes());
        bytes.extend(16_u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(data_len.to_le_bytes());
        bytes.extend(samples.iter().flat_map(|sample| sample.to_le_bytes()));
        rune_buffered(RuneDecoder::new(Cursor::new(bytes), Some("wav")).unwrap())
    }

    /// The samples of the 16-bit file as the decoder yields them, on the scale of -1 to 1.
    fn decoded(samples: &[i16]) -> Vec<Sample> {
        samples
            .iter()
            .map(|&sample| f32::from(sample) / 32768.0)
            .collect()
    }

    #[test]
    fn stereo_samples_follow_the_scalar_position() {
        let input: Vec<i16> = (0..24).collect();
        let expected = decoded(&input);
        let mut source = buffered_wav(2, &input);

        assert_eq!(source.by_ref().take(10).collect::<Vec<_>>(), expected[..10]);
        assert_eq!(source.current_samples(), Some(expected[10..12].to_vec()));
        assert_eq!(source.next(), Some(expected[10]));
        assert_eq!(
            source.current_samples(),
            Some(expected[10..12].to_vec()),
            "mid-frame"
        );
        assert_eq!(
            source.by_ref().take(11).collect::<Vec<_>>(),
            expected[11..22]
        );
        assert_eq!(
            source.current_samples(),
            Some(expected[22..24].to_vec()),
            "near packet end"
        );
        assert_eq!(source.next(), Some(expected[22]));
        assert_eq!(source.next(), Some(expected[23]));
        assert_eq!(source.current_samples(), None, "exhausted packet");
        assert_eq!(source.next(), None);
    }

    #[test]
    fn mono_samples_and_seeking_use_the_current_position() {
        let input: Vec<i16> = (0..32).collect();
        let expected = decoded(&input);
        let mut source = buffered_wav(1, &input);

        assert_eq!(source.next(), Some(expected[0]));
        assert_eq!(source.current_samples(), Some(vec![expected[1]]));
        source.try_seek(Duration::from_millis(1)).unwrap();
        assert_eq!(source.current_samples(), Some(vec![expected[8]]));
        assert_eq!(source.next(), Some(expected[8]));
    }
}
