use anyhow::Result;
use xmltree::Element;

use crate::types::{LyricFile, LyricLine, TimeTag, VoiceType};

fn parse_timestamp(s: &str) -> TimeTag {
    let parts: Vec<&str> = s.split(':').collect();

    let (minutes, seconds, milliseconds) = match parts.len() {
        1 => {
            let sec_parts: Vec<&str> = parts[0].split('.').collect();
            let seconds = sec_parts
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let centiseconds = sec_parts
                .get(1)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            (0, seconds, centiseconds * 10)
        }
        2 => {
            let sec_parts: Vec<&str> = parts[1].split('.').collect();
            let minutes = parts
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let seconds = sec_parts
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let centiseconds = sec_parts
                .get(1)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            (minutes, seconds, centiseconds * 10)
        }
        3 => {
            let sec_parts: Vec<&str> = parts[2].split('.').collect();
            let hours = parts
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let minutes = parts
                .get(1)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let seconds = sec_parts
                .first()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            let centiseconds = sec_parts
                .get(1)
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or_default();
            (hours * 60 + minutes, seconds, centiseconds * 10)
        }
        _ => (0, 0, 0),
    };

    TimeTag {
        minutes,
        seconds,
        milliseconds,
    }
}

fn extract_text(element: &Element) -> String {
    let mut text = String::new();
    for child in &element.children {
        if let Some(text_content) = child.as_text() {
            text.push_str(text_content);
        } else if let Some(child_element) = child.as_element() {
            text.push_str(&extract_text(child_element));
        }
    }
    text
}

pub fn parse_ttml(content: &str) -> Result<LyricFile> {
    let root = Element::parse(content.as_bytes())?;

    let mut lyric_file = LyricFile::new();

    if let Some(head) = root.get_child("head")
        && let Some(metadata) = head.get_child("metadata")
    {
        for child in &metadata.children {
            if let Some(element) = child.as_element()
                && element.name == "agent"
                && let Some(id) = element.attributes.get("xml:id")
                && let Some(agent_type) = element.attributes.get("type")
            {
                lyric_file
                    .metadata
                    .insert(id.to_string(), agent_type.to_string());
            }
        }
    }

    if let Some(body) = root.get_child("body") {
        for div in body.children.iter().filter_map(|c| c.as_element()) {
            if div.name != "div" {
                continue;
            }

            for p in div.children.iter().filter_map(|c| c.as_element()) {
                if p.name != "p" {
                    continue;
                }

                let start_time = match p.attributes.get("begin") {
                    Some(time) => parse_timestamp(time),
                    None => continue,
                };
                let end_time = match p.attributes.get("end") {
                    Some(time) => parse_timestamp(time),
                    None => continue,
                };

                let voice_type = VoiceType::Default;

                let mut text = String::new();
                let mut word_time_tags = Vec::new();

                let spans: Vec<&Element> =
                    p.children.iter().filter_map(|c| c.as_element()).collect();

                if spans.is_empty() {
                    text.push_str(&extract_text(p));
                } else {
                    for span in spans {
                        if span.name != "span" {
                            continue;
                        }

                        if let Some(word_begin) = span.attributes.get("begin") {
                            if let Some(word_end) = span.attributes.get("end") {
                                let word_start_time = parse_timestamp(word_begin);
                                let word_end_time = parse_timestamp(word_end);
                                let word_text = extract_text(span);
                                text.push_str(&word_text);
                                word_time_tags.push((word_start_time, word_end_time, word_text));
                            }
                        } else if let Some(translation) = span.attributes.get("ttm:role")
                            && translation == "x-translation"
                        {
                            text.push_str(&extract_text(span));
                        }
                    }
                }

                lyric_file.lyrics.push(LyricLine {
                    start_time,
                    end_time,
                    voice_type,
                    text,
                    word_time_tags,
                });
            }
        }
    }

    Ok(lyric_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{LyricLine, TimeTag, VoiceType};

    #[test]
    fn test_parse_single_div_single_p() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml">
                <body>
                    <div>
                        <p begin="0:01.00" end="0:05.212">Hello, world!</p>
                    </div>
                </body>
            </tt>
        "#;

        let result = parse_ttml(ttml_content).unwrap();
        let expected = vec![LyricLine {
            start_time: TimeTag {
                minutes: 0,
                seconds: 1,
                milliseconds: 0,
            },
            end_time: TimeTag {
                minutes: 0,
                seconds: 5,
                milliseconds: 2120,
            },
            voice_type: VoiceType::Default,
            text: "Hello, world!".to_string(),
            word_time_tags: vec![],
        }];

        assert_eq!(result.lyrics, expected);
    }

    #[test]
    fn test_parse_multiple_divs() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml">
                <body>
                    <div>
                        <p begin="0:01.00" end="0:02.00">Line 1</p>
                    </div>
                    <div>
                        <p begin="0:03.00" end="0:04.00">Line 2</p>
                    </div>
                </body>
            </tt>
        "#;

        let result = parse_ttml(ttml_content).unwrap();
        let expected = vec![
            LyricLine {
                start_time: TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0,
                },
                end_time: TimeTag {
                    minutes: 0,
                    seconds: 2,
                    milliseconds: 0,
                },
                voice_type: VoiceType::Default,
                text: "Line 1".to_string(),
                word_time_tags: vec![],
            },
            LyricLine {
                start_time: TimeTag {
                    minutes: 0,
                    seconds: 3,
                    milliseconds: 0,
                },
                end_time: TimeTag {
                    minutes: 0,
                    seconds: 4,
                    milliseconds: 0,
                },
                voice_type: VoiceType::Default,
                text: "Line 2".to_string(),
                word_time_tags: vec![],
            },
        ];

        assert_eq!(result.lyrics, expected);
    }

    #[test]
    fn test_parse_p_without_span() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml">
                <body>
                    <div>
                        <p begin="0:01.00" end="0:02.00">No spans here</p>
                    </div>
                </body>
            </tt>
        "#;

        let result = parse_ttml(ttml_content).unwrap();
        let expected = vec![LyricLine {
            start_time: TimeTag {
                minutes: 0,
                seconds: 1,
                milliseconds: 0,
            },
            end_time: TimeTag {
                minutes: 0,
                seconds: 2,
                milliseconds: 0,
            },
            voice_type: VoiceType::Default,
            text: "No spans here".to_string(),
            word_time_tags: vec![],
        }];

        assert_eq!(result.lyrics, expected);
    }

    #[test]
    fn test_parse_nested_elements_in_span() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml">
                <body>
                    <div>
                        <p begin="0:01.00" end="0:02.00">
                            <span begin="0:01.00" end="0:01.50">Hello <em>world</em></span>
                        </p>
                    </div>
                </body>
            </tt>
        "#;

        let result = parse_ttml(ttml_content).unwrap();
        let expected = vec![LyricLine {
            start_time: TimeTag {
                minutes: 0,
                seconds: 1,
                milliseconds: 0,
            },
            end_time: TimeTag {
                minutes: 0,
                seconds: 2,
                milliseconds: 0,
            },
            voice_type: VoiceType::Default,
            text: "Hello world".to_string(),
            word_time_tags: vec![(
                TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0,
                },
                TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 500,
                },
                "Hello world".to_string(),
            )],
        }];

        assert_eq!(result.lyrics, expected);
    }
}
