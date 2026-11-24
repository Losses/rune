use anyhow::Result;
use subtp::vtt::{VttBlock, VttCue, VttTimestamp, WebVtt};

use crate::{
    types::{LyricFile, LyricLine, TimeTag, VoiceType},
    utils::{parse_word_time_tags, strip_bom},
};

fn to_time_tag(timestamp: &VttTimestamp) -> TimeTag {
    let total = ((timestamp.hours as u64 * 60 + timestamp.minutes as u64) * 60
        + timestamp.seconds as u64)
        * 1000
        + timestamp.milliseconds as u64;
    TimeTag::from_millis(total)
}

pub fn parse_vtt(content: &str) -> Result<LyricFile> {
    let mut vtt = LyricFile::new();

    // The cue parser wants the file to end with a blank line (see srt.rs)
    let content = strip_bom(content).trim_start();
    let content = if content.ends_with("\n\n") {
        content.to_string()
    } else {
        format!("{content}\n\n")
    };
    let webvtt = WebVtt::parse(&content)?;

    for block in webvtt.blocks {
        if let VttBlock::Que(VttCue {
            timings, payload, ..
        }) = block
        {
            let start_time = to_time_tag(&timings.start);
            // A cue that ends before it starts is malformed; keep it a moment
            let end_time = to_time_tag(&timings.end).max_millis(&start_time);

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
        assert_eq!(
            first_line.word_time_tags[0].2,
            "Hello world this is a test "
        );

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
            "Another line with styles and "
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

    #[test]
    fn test_parse_vtt_hours_minutes_and_no_trailing_newline() {
        let content = "\u{FEFF}WEBVTT\n\n01:02:03.004 --> 01:02:05.500\nLate cue\n\n10:00.000 --> 10:01.000\nTen minutes";
        let result = parse_vtt(content).unwrap();
        assert_eq!(result.lyrics.len(), 2);
        assert_eq!(result.lyrics[0].start_time.as_millis(), 3_723_004);
        assert_eq!(result.lyrics[0].end_time.as_millis(), 3_725_500);
        assert_eq!(result.lyrics[1].start_time.as_millis(), 600_000);
    }

    #[test]
    fn test_parse_vtt_short_word_tags_keep_spaces() {
        let content = "WEBVTT\n\n00:01.000 --> 00:05.000\nHello <00:02.500>big <c.x>world</c>\n";
        let line = &parse_vtt(content).unwrap().lyrics[0];
        let words: Vec<&str> = line.word_time_tags.iter().map(|w| w.2.as_str()).collect();
        assert_eq!(words, ["Hello ", "big world"]);
        assert_eq!(line.word_time_tags[1].0.as_millis(), 2500);
    }

    #[test]
    fn test_clean_style_tags_keeps_plain_angle_brackets() {
        assert_eq!(clean_style_tags("a<b and c>d"), "a<b and c>d");
        assert_eq!(clean_style_tags("1 < 2 > 0"), "1 < 2 > 0");
        assert_eq!(
            clean_style_tags("<v Bob><i>hi</i></v> <lang en>x</lang> <ruby>a<rt>b</rt></ruby>"),
            "hi x ab"
        );
    }
}
