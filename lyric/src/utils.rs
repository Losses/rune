use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::TimeTag;

// Only real WebVTT tags, so that text like `a<b and c>d` is left alone.
// `b`, `i`, `u`, `c`, `ruby`, `rt` take classes; `v` and `lang` take an annotation;
// `font` and `s` are the SRT flavours.
static STYLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"</?(?:b|i|u|s|c|ruby|rt|rp)(?:\.[^\s<>]+)*>|</?font(?:[ \t][^<>]*)?>|<(?:v|lang)(?:\.[^\s<>]+)*[ \t]+[^<>]+>|</(?:v|lang)>",
    )
    .unwrap()
});
/// `<hh:mm:ss.mmm>` or `<mm:ss.mmm>`; the separator may also be a comma (SRT).
pub static TIME_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<(?:(\d{1,2}):)?(\d{2}):(\d{2})[.,](\d{3})>").unwrap());

/// Removes a leading UTF-8 byte order mark.
pub fn strip_bom(content: &str) -> &str {
    content.strip_prefix('\u{FEFF}').unwrap_or(content)
}

pub fn clean_style_tags(text: &str) -> String {
    STYLE_RE.replace_all(text, "").into_owned()
}

pub fn parse_word_time_tags(
    text: &str,
    start_time: &TimeTag,
    end_time: &TimeTag,
) -> (String, Vec<(TimeTag, TimeTag, String)>) {
    let cleaned_text = clean_style_tags(text);
    let cleaned_text = cleaned_text.trim();
    let mut word_time_tags: Vec<(TimeTag, TimeTag, String)> = vec![];
    let mut last_end_time = start_time.clone();
    let mut current_pos = 0;

    for cap in TIME_TAG_RE.captures_iter(cleaned_text) {
        let hours: u64 = cap.get(1).map_or(0, |m| m.as_str().parse().unwrap());
        let minutes: u64 = cap[2].parse().unwrap();
        let seconds: u64 = cap[3].parse().unwrap();
        let milliseconds: u64 = cap[4].parse().unwrap();

        let word_start_time =
            TimeTag::from_millis(((hours * 60 + minutes) * 60 + seconds) * 1000 + milliseconds);

        if let Some(pos) = cap.get(0) {
            // Inter-word spaces belong to the preceding word, so no trimming
            push_section(
                &mut word_time_tags,
                &last_end_time,
                &word_start_time,
                &cleaned_text[current_pos..pos.start()],
            );
            last_end_time = word_start_time;
            current_pos = pos.end();
        }
    }

    push_section(
        &mut word_time_tags,
        &last_end_time,
        end_time,
        &cleaned_text[current_pos..],
    );

    (cleaned_text.to_string(), word_time_tags)
}

/// Adds a section; text that is only whitespace is glued to the previous section
/// (or dropped when there is none) so that the spaces between words survive.
pub(crate) fn push_section(
    sections: &mut Vec<(TimeTag, TimeTag, String)>,
    start: &TimeTag,
    end: &TimeTag,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    if text.trim().is_empty() {
        if let Some(last) = sections.last_mut() {
            last.2.push_str(text);
        }
        return;
    }
    sections.push((start.clone(), end.clone(), text.to_string()));
}
