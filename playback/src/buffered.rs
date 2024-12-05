use log::warn;
use rodio::source::SeekError;
use rodio::{Sample, Source};
use std::time::Duration;

/// Internal function that builds a `RuneBuffered` object.
#[inline]
pub fn rune_buffered<I>(input: I) -> RuneBuffered<I>
where
    I: Source,
    I::Item: Sample,
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
    I::Item: Sample,
{
    /// The underlying source
    source: I,

    /// Current frame data for current_samples() support
    current_frame_data: Vec<I::Item>,

    /// Number of channels in current frame
    current_frame_channels: u16,

    /// The position in number of samples inside current frame
    position_in_frame: usize,
}

impl<I> RuneBuffered<I>
where
    I: Source,
    I::Item: Sample,
{
    /// Returns the current samples for all channels at the current position
    /// Returns None if we're at the end of the stream or the position is invalid
    pub fn current_samples(&self) -> Option<Vec<I::Item>> {
        let channels = self.current_frame_channels as usize;
        if channels == 0 {
            return None;
        }

        let base_pos = self.position_in_frame * channels;
        if base_pos + channels <= self.current_frame_data.len() {
            Some(self.current_frame_data[base_pos..base_pos + channels].to_vec())
        } else {
            None
        }
    }

    /// Updates the current frame data from source
    fn update_frame(&mut self) {
        self.current_frame_data.clear();
        self.current_frame_channels = self.source.channels();

        let frame_len = self.source.current_frame_len().unwrap_or(0);
        if frame_len > 0 {
            self.current_frame_data = (&mut self.source).take(frame_len).collect();
        }

        self.position_in_frame = 0;
    }
}

impl<I> Iterator for RuneBuffered<I>
where
    I: Source,
    I::Item: Sample,
{
    type Item = I::Item;

    #[inline]
    fn next(&mut self) -> Option<I::Item> {
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
    I::Item: Sample,
{
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        self.source.current_frame_len()
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.source.channels()
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
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
    I::Item: Sample,
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
