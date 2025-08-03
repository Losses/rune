use anyhow::Result;
use subtp::srt::{SrtTimestamp, SubRip};

use crate::{
    types::{LyricFile, LyricLine, TimeTag, VoiceType},
    utils::parse_word_time_tags,
};

impl From<SrtTimestamp> for TimeTag {
    fn from(timestamp: SrtTimestamp) -> Self {
        let total_minutes = timestamp.hours as u32 * 60 + timestamp.minutes as u32;
        let total_seconds = timestamp.seconds as u32;
        let total_milliseconds = timestamp.milliseconds as u32;

        TimeTag {
            minutes: total_minutes,
            seconds: total_seconds,
            milliseconds: total_milliseconds,
        }
    }
}

pub fn parse_srt(content: &str) -> Result<LyricFile> {
    let mut srt = LyricFile::new();

    if content.trim().is_empty() {
        return Ok(srt);
    }

    let content = if content.ends_with("\n\n") {
        content.to_string()
    } else {
        format!("{content}\n\n")
    };

    let subrip = SubRip::parse(&content)?;

    for block in subrip.subtitles {
        let start_time: TimeTag = block.start.into();
        let end_time: TimeTag = block.end.into();

        let (text, word_time_tags) =
            parse_word_time_tags(&block.text.join("\n"), &start_time, &end_time);

        let lyric_line = LyricLine {
            start_time,
            end_time,
            voice_type: VoiceType::Default,
            text,
            word_time_tags,
        };

        srt.lyrics.push(lyric_line);
    }

    Ok(srt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LyricLine, TimeTag, VoiceType};

    #[test]
    fn test_srt_timestamp_conversion() {
        let srt_timestamp = SrtTimestamp {
            hours: 1,
            minutes: 30,
            seconds: 45,
            milliseconds: 500,
        };

        let time_tag: TimeTag = srt_timestamp.into();
        assert_eq!(time_tag.minutes, 90); // 1 hour 30 minutes = 90 minutes
        assert_eq!(time_tag.seconds, 45);
        assert_eq!(time_tag.milliseconds, 500);
    }

    #[test]
    fn test_parse_srt_basic() {
        let srt_content = r#"1
00:00:01,000 --> 00:00:02,000
Hello world

2
00:00:03,000 --> 00:00:04,000
This is a test"#;

        let result = parse_srt(srt_content).unwrap();
        assert_eq!(result.lyrics.len(), 2);

        assert_eq!(
            result.lyrics[0],
            LyricLine {
                start_time: TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0
                },
                end_time: TimeTag {
                    minutes: 0,
                    seconds: 2,
                    milliseconds: 0
                },
                voice_type: VoiceType::Default,
                text: "Hello world".to_string(),
                word_time_tags: vec![(
                    TimeTag {
                        minutes: 0,
                        seconds: 1,
                        milliseconds: 0
                    },
                    TimeTag {
                        minutes: 0,
                        seconds: 2,
                        milliseconds: 0
                    },
                    "Hello world".to_string()
                )],
            }
        );
    }

    #[test]
    fn test_parse_srt_with_styles() {
        let srt_content = r#"1
00:00:01,000 --> 00:00:02,000
<b>Hello</b> <i>world</i>

2
00:00:03,000 --> 00:00:04,000
This is a <font color="red">test</font>"#;

        let result = parse_srt(srt_content).unwrap();
        assert_eq!(result.lyrics.len(), 2);
        assert_eq!(result.lyrics[0].text, "Hello world");
        assert_eq!(result.lyrics[1].text, "This is a test");
    }

    #[test]
    fn test_parse_srt_with_time_tags() {
        let srt_content = r#"1
00:00:01,000 --> 00:00:05,000
Hello world <00:00:02,500>this is <00:00:03,500>a test

2
00:00:06,000 --> 00:00:10,000
Another <00:00:07,500>line"#;

        let result = parse_srt(srt_content).unwrap();

        let first_line = &result.lyrics[0];
        assert_eq!(first_line.word_time_tags.len(), 3);
        assert_eq!(first_line.word_time_tags[0].2, "Hello world");
        assert_eq!(first_line.word_time_tags[1].2, "this is");
        assert_eq!(first_line.word_time_tags[2].2, "a test");

        assert_eq!(
            first_line.word_time_tags[1].0,
            TimeTag {
                minutes: 0,
                seconds: 2,
                milliseconds: 500
            }
        );
    }

    #[test]
    fn test_parse_srt_with_multiline_text() {
        let srt_content = r#"1
00:00:01,000 --> 00:00:02,000
Line one
Line two

2
00:00:03,000 --> 00:00:04,000
Another line"#;

        let result = parse_srt(srt_content).unwrap();
        assert_eq!(result.lyrics[0].text, "Line one\nLine two");
    }

    #[test]
    fn test_parse_srt_empty() {
        let srt_content = "";
        let result = parse_srt(srt_content).unwrap();
        assert_eq!(result.lyrics, []);
    }

    #[test]
    fn test_parse_srt_invalid_format() {
        let srt_content = "Invalid content";
        let result = parse_srt(srt_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_srt_with_hour_timestamps() {
        let srt_content = r#"1
01:30:00,000 --> 01:30:05,000
Text spanning over an hour"#;

        let result = parse_srt(srt_content).unwrap();
        assert_eq!(
            result.lyrics[0].start_time,
            TimeTag {
                minutes: 90,
                seconds: 0,
                milliseconds: 0
            }
        );
    }
}
