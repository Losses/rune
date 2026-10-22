use anyhow::Result;
use xmltree::{Element, ParserConfig, XMLNode};

use crate::{
    types::{LyricFile, LyricLine, TimeTag, VoiceType, fraction_to_millis},
    utils::strip_bom,
};

/// Parses a TTML time expression into milliseconds: clock times
/// (`ss.fff`, `mm:ss.fff`, `hh:mm:ss.fff`) and offset times (`5.2s`, `1500ms`, `2m`, `1h`).
fn parse_time(s: &str) -> Option<u64> {
    let s = s.trim();

    let offset_units: [(&str, f64); 4] = [
        ("ms", 1.0),
        ("h", 3_600_000.0),
        ("m", 60_000.0),
        ("s", 1000.0),
    ];
    for (suffix, unit) in offset_units {
        if let Some(value) = s.strip_suffix(suffix) {
            let value = value.parse::<f64>().ok()?;
            return (value.is_finite() && value >= 0.0).then(|| (value * unit).round() as u64);
        }
    }

    let parts: Vec<&str> = s.split(':').collect();
    let (hours, minutes, seconds) = match parts.as_slice() {
        [s] => (0, 0, *s),
        [m, s] => (0, m.parse::<u64>().ok()?, *s),
        [h, m, s] => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?, *s),
        _ => return None, // Frames are not supported
    };

    let (seconds, fraction) = seconds.split_once('.').unwrap_or((seconds, ""));
    let milliseconds = if fraction.is_empty() {
        0
    } else {
        fraction_to_millis(fraction).ok()? as u64
    };
    let seconds = seconds.parse::<u64>().ok()?;

    // Times that do not fit are rejected like any other bad value
    hours
        .checked_mul(60)?
        .checked_add(minutes)?
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(milliseconds)
}

/// Translations and romanisations are not part of the main sections.
fn is_secondary(element: &Element) -> bool {
    // xmltree strips the `ttm:` prefix from attribute names
    matches!(
        element.attributes.get("role").map(String::as_str),
        Some("x-translation" | "x-roman" | "x-transliteration")
    )
}

/// Collapses runs of whitespace into one space, keeping a leading/trailing space.
fn collapse_whitespace(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    if text.starts_with(char::is_whitespace) {
        result.push(' ');
    }
    result.push_str(&text.split_whitespace().collect::<Vec<_>>().join(" "));
    if text.ends_with(char::is_whitespace) && !result.ends_with(' ') {
        result.push(' ');
    }
    result
}

fn extract_text(element: &Element) -> String {
    let mut text = String::new();
    for child in &element.children {
        match child {
            XMLNode::Text(t) | XMLNode::CData(t) => text.push_str(&collapse_whitespace(t)),
            XMLNode::Element(e) if e.name == "br" => text.push(' '),
            XMLNode::Element(e) if !is_secondary(e) => text.push_str(&extract_text(e)),
            _ => {}
        }
    }
    text
}

/// The content of a `<p>`, in document order.
enum Piece {
    Text(String),
    Span {
        start: u64,
        end: Option<u64>,
        text: String,
    },
}

fn span_times(element: &Element) -> Option<(u64, Option<u64>)> {
    let start = parse_time(element.attributes.get("begin")?)?;
    let end = element
        .attributes
        .get("end")
        .and_then(|end| parse_time(end))
        .or_else(|| {
            let dur = parse_time(element.attributes.get("dur")?)?;
            start.checked_add(dur)
        });
    Some((start, end))
}

fn collect_pieces(p: &Element) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for child in &p.children {
        match child {
            XMLNode::Text(t) | XMLNode::CData(t) => {
                // Indentation between elements is not a space, a single-line gap is
                if !(t.trim().is_empty() && t.contains('\n')) {
                    pieces.push(Piece::Text(collapse_whitespace(t)));
                }
            }
            XMLNode::Element(e) if e.name == "br" => pieces.push(Piece::Text(" ".to_string())),
            XMLNode::Element(e) if is_secondary(e) => {}
            XMLNode::Element(e) => match (e.name == "span").then(|| span_times(e)).flatten() {
                Some((start, end)) => pieces.push(Piece::Span {
                    start,
                    end,
                    text: extract_text(e),
                }),
                None => pieces.push(Piece::Text(extract_text(e))),
            },
            _ => {}
        }
    }
    pieces
}

pub fn parse_ttml(content: &str) -> Result<LyricFile> {
    // Whitespace-only text between spans can be a real space
    let config = ParserConfig::new().whitespace_to_characters(true);
    let root = Element::parse_with_config(strip_bom(content).as_bytes(), config)?;

    let mut lyric_file = LyricFile::new();

    if let Some(head) = root.get_child("head")
        && let Some(metadata) = head.get_child("metadata")
    {
        for child in &metadata.children {
            // xmltree strips the `xml:` prefix from attribute names
            if let Some(element) = child.as_element()
                && element.name == "agent"
                && let Some(id) = element.attributes.get("id")
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

                if let Some(line) = parse_paragraph(p) {
                    lyric_file.lyrics.push(line);
                }
            }
        }
    }

    Ok(lyric_file)
}

fn parse_paragraph(p: &Element) -> Option<LyricLine> {
    let (start, p_end) = span_times(p)?;
    let pieces = collect_pieces(p);

    let text = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Text(text) | Piece::Span { text, .. } => text.as_str(),
        })
        .collect::<String>();
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }

    // Without an end on the <p>, the last timed span decides
    let end = p_end.or_else(|| {
        pieces.iter().rev().find_map(|piece| match piece {
            Piece::Span { start, end, .. } => Some(end.unwrap_or(*start)),
            Piece::Text(_) => None,
        })
    })?;
    // A paragraph that ends before it begins is malformed; keep it a moment
    let end = end.max(start);

    if !pieces
        .iter()
        .any(|piece| matches!(piece, Piece::Span { .. }))
    {
        return Some(LyricLine {
            start_time: TimeTag::from_millis(start),
            end_time: TimeTag::from_millis(end),
            voice_type: VoiceType::Default,
            word_time_tags: vec![(
                TimeTag::from_millis(start),
                TimeTag::from_millis(end),
                text.clone(),
            )],
            text,
        });
    }

    let mut sections: Vec<(u64, Option<u64>, String)> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Span { start, end, text } => sections.push((start, end, text)),
            // Text between spans belongs to the span before it
            Piece::Text(text) => match sections.last_mut() {
                Some(last) => last.2.push_str(&text),
                None if !text.trim().is_empty() => sections.push((start, None, text)),
                None => {}
            },
        }
    }

    // A span without an end lasts until the next span, or the end of the <p>
    let starts: Vec<u64> = sections.iter().map(|section| section.0).collect();
    let last = sections.len() - 1;
    let word_time_tags = sections
        .into_iter()
        .enumerate()
        .map(|(i, (s, e, text))| {
            let e = e.or_else(|| starts.get(i + 1).copied()).unwrap_or(end);
            let text = match i {
                0 if i == last => text.trim().to_string(),
                0 => text.trim_start().to_string(),
                _ if i == last => text.trim_end().to_string(),
                _ => text,
            };
            (TimeTag::from_millis(s), TimeTag::from_millis(e), text)
        })
        .collect();

    Some(LyricLine {
        start_time: TimeTag::from_millis(start),
        end_time: TimeTag::from_millis(end),
        voice_type: VoiceType::Default,
        text,
        word_time_tags,
    })
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
                milliseconds: 212,
            },
            voice_type: VoiceType::Default,
            text: "Hello, world!".to_string(),
            word_time_tags: vec![(
                TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0,
                },
                TimeTag {
                    minutes: 0,
                    seconds: 5,
                    milliseconds: 212,
                },
                "Hello, world!".to_string(),
            )],
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
                word_time_tags: vec![(
                    TimeTag {
                        minutes: 0,
                        seconds: 1,
                        milliseconds: 0,
                    },
                    TimeTag {
                        minutes: 0,
                        seconds: 2,
                        milliseconds: 0,
                    },
                    "Line 1".to_string(),
                )],
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
                word_time_tags: vec![(
                    TimeTag {
                        minutes: 0,
                        seconds: 3,
                        milliseconds: 0,
                    },
                    TimeTag {
                        minutes: 0,
                        seconds: 4,
                        milliseconds: 0,
                    },
                    "Line 2".to_string(),
                )],
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
            word_time_tags: vec![(
                TimeTag {
                    minutes: 0,
                    seconds: 1,
                    milliseconds: 0,
                },
                TimeTag {
                    minutes: 0,
                    seconds: 2,
                    milliseconds: 0,
                },
                "No spans here".to_string(),
            )],
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

    fn ms_of(tag: &TimeTag) -> u64 {
        tag.as_millis()
    }

    #[test]
    fn test_parse_time_forms() {
        assert_eq!(parse_time("00:05.212"), Some(5212));
        assert_eq!(parse_time("00:05.2"), Some(5200));
        assert_eq!(parse_time("5.2s"), Some(5200));
        assert_eq!(parse_time("1500ms"), Some(1500));
        assert_eq!(parse_time("2m"), Some(120_000));
        assert_eq!(parse_time("1h"), Some(3_600_000));
        assert_eq!(parse_time("01:00:01.250"), Some(3_601_250));
        assert_eq!(parse_time("bogus"), None);
        assert_eq!(parse_time("9999999999999:00:00"), None);
    }

    #[test]
    fn test_parse_huge_times() {
        let ttml_content = r#"
            <tt><body><div>
                <p begin="1e30s" dur="1s">huge</p>
                <p begin="18446744073709551615ms" end="1s"><span begin="1e30s" dur="1s">max</span></p>
            </div></body></tt>
        "#;

        for line in parse_ttml(ttml_content).unwrap().lyrics {
            assert!(ms_of(&line.start_time) <= ms_of(&line.end_time));
            for (start, end, _) in &line.word_time_tags {
                assert!(ms_of(start) <= ms_of(end));
            }
        }
    }

    #[test]
    fn test_parse_dur_and_untimed_span_end() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml">
                <body><div>
                    <p begin="1s" dur="4s">
                        <span begin="1s">Hello</span> <span begin="2s" dur="500ms">world</span>
                    </p>
                </div></body>
            </tt>
        "#;

        let line = &parse_ttml(ttml_content).unwrap().lyrics[0];
        assert_eq!(ms_of(&line.end_time), 5000);
        assert_eq!(line.text, "Hello world");
        let words = &line.word_time_tags;
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].2, "Hello ");
        assert_eq!(ms_of(&words[0].1), 2000);
        assert_eq!((ms_of(&words[1].0), ms_of(&words[1].1)), (2000, 2500));
        assert_eq!(words[1].2, "world");
    }

    #[test]
    fn test_parse_br_and_translation_roles() {
        let ttml_content = r#"
            <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:xml="http://www.w3.org/XML/1998/namespace">
                <head><metadata><ttm:agent xml:id="v1" type="person"/></metadata></head>
                <body><div>
                    <p begin="1s" end="2s">one<br/>two</p>
                    <p begin="2s" end="3s"><span begin="2s" end="3s">hi</span><span ttm:role="x-translation">你好</span><span ttm:role="x-roman">ni hao</span></p>
                    <p begin="3s" end="4s"><span ttm:role="x-translation">only translation</span></p>
                </div></body>
            </tt>
        "#;

        let result = parse_ttml(ttml_content).unwrap();
        assert_eq!(result.metadata.get("v1").unwrap(), "person");
        assert_eq!(result.lyrics.len(), 2);
        assert_eq!(result.lyrics[0].word_time_tags[0].2, "one two");
        assert_eq!(result.lyrics[1].text, "hi");
        assert_eq!(result.lyrics[1].word_time_tags.len(), 1);
        assert_eq!(result.lyrics[1].word_time_tags[0].2, "hi");
    }

    #[test]
    fn test_parse_ttml_bom() {
        let content = "\u{FEFF}<tt><body><div><p begin=\"1s\" end=\"2s\">a</p></div></body></tt>";
        assert_eq!(parse_ttml(content).unwrap().lyrics.len(), 1);
    }
}
