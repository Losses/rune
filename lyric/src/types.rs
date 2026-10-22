use std::{collections::HashMap, fmt, str::FromStr};

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub struct TimeTag {
    pub minutes: u32,
    pub seconds: u32,
    pub milliseconds: u32,
}

impl TimeTag {
    pub fn from_millis(total: u64) -> Self {
        // Saturate so that absurd inputs cannot overflow the u32 fields, and
        // saturate the total so that a later time never becomes an earlier one
        let total = total.min(u32::MAX as u64 * 60_000 + 59_999);
        let minutes = (total / 60_000) as u32;
        TimeTag {
            minutes,
            seconds: (total / 1000 % 60) as u32,
            milliseconds: (total % 1000) as u32,
        }
    }

    pub fn as_millis(&self) -> u64 {
        (self.minutes as u64)
            .saturating_mul(60_000)
            .saturating_add(self.seconds as u64 * 1000)
            .saturating_add(self.milliseconds as u64)
    }

    /// This time, or `other` if that is later.
    pub(crate) fn max_millis(self, other: &TimeTag) -> Self {
        if self.as_millis() < other.as_millis() {
            other.clone()
        } else {
            self
        }
    }
}

impl From<TimeTag> for i32 {
    fn from(val: TimeTag) -> Self {
        val.as_millis().min(i32::MAX as u64) as i32
    }
}

/// Converts the digits after the decimal separator into milliseconds,
/// so `5` is 500, `50` is 500 and `500` is 500. Digits past the third are
/// dropped (TTML); LRC time tags reject them before getting here.
pub(crate) fn fraction_to_millis(fraction: &str) -> Result<u32> {
    if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        bail!("Invalid fraction format");
    }
    let padded: String = fraction.chars().chain("000".chars()).take(3).collect();
    Ok(padded.parse::<u32>()?)
}

impl fmt::Display for TimeTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{:02}:{:02}.{:02}]",
            self.minutes,
            self.seconds,
            self.milliseconds / 10
        )
    }
}

impl FromStr for TimeTag {
    type Err = anyhow::Error;

    /// Accepts `mm:ss`, `mm:ss.f` to `mm:ss.fff`, `mm:ss:ff` and `hh:mm:ss.fff`,
    /// optionally wrapped in `[]` or `<>`.
    fn from_str(s: &str) -> Result<Self> {
        let s = s
            .trim()
            .trim_start_matches(['[', '<'])
            .trim_end_matches([']', '>']);

        let parts: Vec<&str> = s.split(':').collect();
        let (hours, minutes, seconds, fraction) = match parts.as_slice() {
            [m, s] => {
                let (s, f) = s.split_once('.').unwrap_or((s, ""));
                (0, m.parse::<u32>()?, s.parse::<u32>()?, f)
            }
            // `hh:mm:ss.fff` when the last part has a dot, `mm:ss:ff` otherwise
            [a, b, c] => match c.split_once('.') {
                Some((s, f)) => (a.parse::<u32>()?, b.parse::<u32>()?, s.parse::<u32>()?, f),
                None => (0, a.parse::<u32>()?, b.parse::<u32>()?, *c),
            },
            _ => bail!("Invalid time format"),
        };

        if fraction.len() > 3 {
            bail!("Invalid milliseconds format");
        }
        let milliseconds = if fraction.is_empty() {
            0
        } else {
            fraction_to_millis(fraction)?
        };

        let minutes = (hours as u64 * 60 + minutes as u64).min(u32::MAX as u64) as u32;
        Ok(TimeTag {
            minutes,
            seconds,
            milliseconds,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VoiceType {
    Male,
    Female,
    Duet,
    Default,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    pub start_time: TimeTag,
    pub end_time: TimeTag,
    pub voice_type: VoiceType,
    pub text: String,
    pub word_time_tags: Vec<(TimeTag, TimeTag, String)>, // Start and end time tags for each word
}

#[derive(Debug, Default)]
pub struct LyricFile {
    // ID tags
    pub metadata: HashMap<String, String>,
    // Lyrics content
    pub lyrics: Vec<LyricLine>,
}

impl LyricFile {
    pub fn new() -> Self {
        Self {
            metadata: HashMap::new(),
            lyrics: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_millis_keeps_order_past_the_limit() {
        let limit = u32::MAX as u64 * 60_000 + 59_999;
        let times = [limit - 1, limit, limit + 1, limit + 60_000, u64::MAX];
        for pair in times.windows(2) {
            let (a, b) = (TimeTag::from_millis(pair[0]), TimeTag::from_millis(pair[1]));
            assert!(a.as_millis() <= b.as_millis(), "{pair:?}");
        }
        assert_eq!(TimeTag::from_millis(u64::MAX).as_millis(), limit);
    }
}
