use std::{collections::HashMap, fmt, str::FromStr};

use anyhow::{Result, bail};

#[derive(Debug, Clone, PartialEq)]
pub struct TimeTag {
    pub minutes: u32,
    pub seconds: u32,
    pub milliseconds: u32,
}

impl From<TimeTag> for i32 {
    fn from(val: TimeTag) -> Self {
        (val.minutes * 60 * 1000 + val.seconds * 1000 + val.milliseconds) as i32
    }
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

    fn from_str(s: &str) -> Result<Self> {
        // Remove [] or <> brackets
        let s = s
            .trim_start_matches(['[', '<'])
            .trim_end_matches([']', '>']);

        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            bail!("Invalid time format");
        }

        let minutes = parts[0].parse::<u32>()?;
        let second_parts: Vec<&str> = parts[1].split('.').collect();
        if second_parts.len() != 2 {
            bail!("Invalid seconds format");
        }

        let seconds = second_parts[0].parse::<u32>()?;
        let centiseconds = second_parts[1].parse::<u32>()?;

        Ok(TimeTag {
            minutes,
            seconds,
            milliseconds: centiseconds * 10,
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
