use anyhow::Result;
use subtp::vtt::{VttBlock, VttCue, WebVtt};

use crate::{
    types::{LyricFile, LyricLine, TimeTag, VoiceType},
    utils::parse_word_time_tags,
};

pub fn parse_vtt(content: &str) -> Result<LyricFile> {
    let mut vtt = LyricFile::new();
    let webvtt = WebVtt::parse(content.trim_start())?;

    for block in webvtt.blocks {
        if let VttBlock::Que(VttCue {
            timings, payload, ..
        }) = block
        {
            let start_time = TimeTag {
                minutes: (timings.start.seconds / 60) as u32,
                seconds: (timings.start.seconds % 60) as u32,
                milliseconds: timings.start.milliseconds as u32,
            };

            let end_time = TimeTag {
                minutes: (timings.end.seconds / 60) as u32,
                seconds: (timings.end.seconds % 60) as u32,
                milliseconds: timings.end.milliseconds as u32,
            };

            let (text, word_time_tags) =
                parse_word_time_tags(&payload.join("\n"), &start_time, &end_time);

            let lyric_line = LyricLine {
                start_time,
                end_time,
                voice_type: VoiceType::Default,
                text,
                word_time_tags,
            };

            vtt.lyrics.push(lyric_line);
        }
    }

    Ok(vtt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        types::{LyricLine, TimeTag, VoiceType},
        utils::clean_style_tags,
    };

    #[test]
    fn test_parse_vtt_basic() {
        let vtt_content = r#"WEBVTT

00:01.000 --> 00:02.000
Hello world

00:03.000 --> 00:04.000
This is a test
"#;
        let result = parse_vtt(vtt_content).unwrap();

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

        assert_eq!(
            result.lyrics[1],
            LyricLine {
                start_time: TimeTag {
                    minutes: 0,
                    seconds: 3,
                    milliseconds: 0
                },
                end_time: TimeTag {
                    minutes: 0,
                    seconds: 4,
                    milliseconds: 0
                },
                voice_type: VoiceType::Default,
                text: "This is a test".to_string(),
                word_time_tags: vec![(
                    TimeTag {
                        minutes: 0,
                        seconds: 3,
                        milliseconds: 0
                    },
                    TimeTag {
                        minutes: 0,
                        seconds: 4,
                        milliseconds: 0
                    },
                    "This is a test".to_string()
                )],
            }
        );
    }

    #[test]
    fn test_parse_vtt_with_styles() {
        let vtt_content = r#"WEBVTT

STYLE
::cue {
  color: lime;
}

00:01.000 --> 00:02.000
Hello <c.color>world</c>

00:03.000 --> 00:04.000
This is a <b>test</b>
"#;
        let result = parse_vtt(vtt_content).unwrap();

        assert_eq!(result.lyrics.len(), 2);

        assert_eq!(result.lyrics[0].text, "Hello world");
        assert_eq!(result.lyrics[1].text, "This is a test");
    }

    #[test]
    fn test_parse_vtt_empty() {
        let vtt_content = "WEBVTT\n\n";
        let result = parse_vtt(vtt_content).unwrap();
        assert_eq!(result.lyrics.len(), 0);
    }

    #[test]
    fn test_parse_vtt_invalid_format() {
        let vtt_content = "Invalid content";
        let result = parse_vtt(vtt_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_clean_style_tags() {
        let input = "<b>Bold</b> text <c.colorF890B5>colored</c> <12:32:31.412> time";
        let expected = "Bold text colored <12:32:31.412> time";
        assert_eq!(clean_style_tags(input), expected);

        let input_unmatched = "<b>Bold text <c.colorF890B5>colored</b> <12:32:31.412> time";
        let expected_unmatched = "Bold text colored <12:32:31.412> time";
        assert_eq!(clean_style_tags(input_unmatched), expected_unmatched);
    }

    #[test]
    fn test_parse_vtt() {
        let content = r"
        WEBVTT

        00:00:01.000 --> 00:00:05.000
        <b>Hello</b> world <c.colorF890B5>this</c> is a test <00:00:03.000>.

        00:00:06.000 --> 00:00:10.000
        Another line <i>with styles</i> and <00:00:08.000>time.
        ";

        let result = parse_vtt(content).unwrap();

        assert_eq!(result.lyrics.len(), 2);

        let first_line = &result.lyrics[0];
        assert_eq!(
            first_line.text,
            "Hello world this is a test <00:00:03.000>."
        );
        assert_eq!(first_line.word_time_tags.len(), 2);
        assert_eq!(first_line.word_time_tags[0].2, "Hello world this is a test");

        let second_line = &result.lyrics[1];
        assert_eq!(
            second_line.text,
            "Another line with styles and <00:00:08.000>time."
        );
        assert_eq!(second_line.word_time_tags.len(), 2);
        assert_eq!(
            second_line.word_time_tags[0].0,
            TimeTag {
                minutes: 0,
                seconds: 6,
                milliseconds: 0
            }
        );
        assert_eq!(
            second_line.word_time_tags[0].1,
            TimeTag {
                minutes: 0,
                seconds: 8,
                milliseconds: 0
            }
        );
        assert_eq!(
            second_line.word_time_tags[0].2,
            "Another line with styles and"
        );
        assert_eq!(
            second_line.word_time_tags[1].0,
            TimeTag {
                minutes: 0,
                seconds: 8,
                milliseconds: 0
            }
        );
        assert_eq!(
            second_line.word_time_tags[1].1,
            TimeTag {
                minutes: 0,
                seconds: 10,
                milliseconds: 0
            }
        );
        assert_eq!(second_line.word_time_tags[1].2, "time.");
    }
}
