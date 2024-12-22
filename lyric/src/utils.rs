use once_cell::sync::Lazy;
use regex::Regex;

use crate::types::TimeTag;

static STYLE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"</?([a-zA-Z][^>]*)>").unwrap());
pub static TIME_TAG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<(\d{2}):(\d{2}):(\d{2}).(\d{3})>").unwrap());

pub fn clean_style_tags(text: &str) -> String {
    let mut stack: Vec<String> = vec![];
    let mut result = String::new();
    let mut last_pos = 0;

    for cap in STYLE_RE.captures_iter(text) {
        if let Some(pos) = cap.get(0) {
            let tag = &cap[1];
            if !TIME_TAG_RE.is_match(&cap[0]) {
                if cap[0].starts_with("</") {
                    let mut found_match = false;
                    for (i, open_tag) in stack.iter().enumerate().rev() {
                        if open_tag == tag {
                            result.push_str(&text[last_pos..pos.start()]);
                            last_pos = pos.end();
                            stack.drain(i..); // Remove the matched tag and any tags above it
                            found_match = true;
                            break;
                        }
                    }
                    // Unmatched closing tag, remove it by NOT updating last_pos
                    if !found_match {
                        result.push_str(&text[last_pos..pos.start()]);
                        last_pos = pos.end(); // Update last_pos to skip the unmatched tag
                    }
                } else {
                    // Opening tag, push it onto the stack
                    result.push_str(&text[last_pos..pos.start()]); // Append text before opening tag
                    stack.push(tag.to_string());
                    last_pos = pos.end();
                }
            }
        }
    }

    result.push_str(&text[last_pos..]);
    result
}

pub fn parse_word_time_tags(
    text: &str,
    start_time: &TimeTag,
    end_time: &TimeTag,
) -> (String, Vec<(TimeTag, TimeTag, String)>) {
    let cleaned_text = clean_style_tags(text);
    let mut word_time_tags = vec![];
    let mut last_end_time = start_time.clone();
    let mut current_pos = 0;

    for cap in TIME_TAG_RE.captures_iter(&cleaned_text) {
        let hours: u32 = cap[1].parse().unwrap();
        let minutes: u32 = cap[2].parse().unwrap();
        let seconds: u32 = cap[3].parse().unwrap();
        let milliseconds: u32 = cap[4].parse().unwrap();

        let total_seconds = hours * 3600 + minutes * 60 + seconds;

        let word_start_time = TimeTag {
            minutes: total_seconds / 60,
            seconds: total_seconds % 60,
            milliseconds,
        };

        if let Some(pos) = cap.get(0) {
            let word = &cleaned_text[current_pos..pos.start()].trim();
            if !word.is_empty() {
                word_time_tags.push((
                    last_end_time.clone(),
                    word_start_time.clone(),
                    word.to_string(),
                ));
            }
            last_end_time = word_start_time.clone();
            current_pos = pos.end();
        }
    }

    if current_pos < cleaned_text.len() {
        let remaining_text = cleaned_text[current_pos..].trim();
        if !remaining_text.is_empty() {
            word_time_tags.push((last_end_time, end_time.clone(), remaining_text.to_string()));
        }
    }

    (cleaned_text.trim().to_string(), word_time_tags)
}
