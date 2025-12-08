use std::str::FromStr;

use anyhow::Result;

use crate::types::{LyricFile, LyricLine, TimeTag, VoiceType};

const DUMMY_END_TIME: TimeTag = TimeTag {
    minutes: 9999,
    seconds: 0,
    milliseconds: 0,
};

pub fn parse_lrc(content: &str) -> Result<LyricFile> {
    let mut lrc = LyricFile::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse ID tags
        if line.starts_with('[') && !line[1..].starts_with(char::is_numeric) {
            if let Some(pos) = line.find(':') {
                let key = &line[1..pos];
                let value = &line[pos + 1..line.len() - 1];
                lrc.metadata.insert(key.to_string(), value.to_string());
            }
            continue;
        }

        // Parse lyric lines
        if let Some(first_bracket) = line.find('[') {
            if first_bracket > 0 {
                continue; // Invalid line
            }

            let mut voice_type = VoiceType::Default;

            // Parse main time tag
            if let Some(close_bracket) = line[first_bracket..].find(']') {
                let time_str = &line[first_bracket..=first_bracket + close_bracket];
                let start_time = TimeTag::from_str(time_str)?;

                // Update previous line's end_time
                if let Some(last_line) = lrc.lyrics.last_mut() {
                    last_line.end_time = start_time.clone();

                    if let Some(last_detail) = last_line.word_time_tags.last_mut() {
                        last_detail.1 = start_time.clone();
                    }
                }

                let remaining_content = &line[first_bracket + close_bracket + 1..];

                // Extract lyric text and voice type
                let text = if let Some(stripped) = remaining_content.strip_prefix("M:") {
                    voice_type = VoiceType::Male;
                    stripped.trim().to_string()
                } else if let Some(stripped) = remaining_content.strip_prefix("F:") {
                    voice_type = VoiceType::Female;
                    stripped.trim().to_string()
                } else if let Some(stripped) = remaining_content.strip_prefix("D:") {
                    voice_type = VoiceType::Duet;
                    stripped.trim().to_string()
                } else {
                    remaining_content.to_string()
                };

                // Parse enhanced format word-level time tags
                let word_time_tags = if remaining_content.contains('<') {
                    parse_enhanced_lrc(remaining_content).unwrap_or_else(|_| {
                        vec![(
                            start_time.clone(),
                            DUMMY_END_TIME,
                            text.to_string(),
                        )]
                    })
                } else {
                    vec![(
                        start_time.clone(),
                        DUMMY_END_TIME,
                        text.to_string(),
                    )]
                };

                lrc.lyrics.push(LyricLine {
                    start_time: start_time.clone(),
                    end_time: DUMMY_END_TIME, // Temporary, will be updated in next iteration
                    voice_type,
                    text,
                    word_time_tags,
                });
            }
        }
    }

    Ok(lrc)
}

fn parse_enhanced_lrc(content: &str) -> Result<Vec<(TimeTag, TimeTag, String)>> {
    let mut word_time_tags: Vec<(TimeTag, TimeTag, String)> = Vec::new();
    let mut current_pos = 0;

    while let Some(start_pos) = content[current_pos..].find('<') {
        if let Some(end_pos) = content[current_pos + start_pos..].find('>') {
            let time_tag_str =
                &content[current_pos + start_pos..current_pos + start_pos + end_pos + 1];
            let start_time = TimeTag::from_str(time_tag_str)?;

            // Determine end_time using the previous start_time
            let end_time = DUMMY_END_TIME;

            // Update previous line's end_time
            if let Some(last_line) = word_time_tags.last_mut() {
                last_line.1 = start_time.clone();
            }

            // Find the next time tag or end of string
            let next_tag_start = content[current_pos + start_pos + end_pos + 1..]
                .find('<')
                .map(|pos| current_pos + start_pos + end_pos + 1 + pos)
                .unwrap_or(content.len());

            let word = content[current_pos + start_pos + end_pos + 1..next_tag_start].to_string();
            if !word.is_empty() && !word.trim().is_empty() {
                word_time_tags.push((start_time, end_time, word));
            }

            current_pos = next_tag_start;
        } else {
            break;
        }
    }

    Ok(word_time_tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lrc_parser() -> Result<()> {
        let lrc_content = r#"[ti:Let's Twist Again]
[ar:Chubby Checker]
[00:12.00]First line
[00:15.30]F: Female line
[00:21.10]M: <00:21.10>Male <00:23.10>line
[00:24.00]<00:24.00>Word <00:24.50>by <00:25.00>word"#;

        let lrc_file = parse_lrc(lrc_content)?;

        // Test metadata
        assert_eq!(lrc_file.metadata.get("ti").unwrap(), "Let's Twist Again");
        assert_eq!(lrc_file.metadata.get("ar").unwrap(), "Chubby Checker");

        // Test lyrics - should have 4 lines
        assert_eq!(lrc_file.lyrics.len(), 4);

        // Test first line
        let first = &lrc_file.lyrics[0];
        assert_eq!(first.start_time.to_string(), "[00:12.00]");
        assert_eq!(first.text, "First line");
        assert_eq!(first.voice_type, VoiceType::Default);

        // Test female line
        let female = &lrc_file.lyrics[1];
        assert_eq!(female.start_time.to_string(), "[00:15.30]");
        assert_eq!(female.text, "Female line");
        assert_eq!(female.voice_type, VoiceType::Female);

        // Test male line
        let male = &lrc_file.lyrics[2];
        assert_eq!(male.start_time.to_string(), "[00:21.10]");
        assert_eq!(male.text, "<00:21.10>Male <00:23.10>line");
        assert_eq!(male.voice_type, VoiceType::Male);

        // Test word time tags
        let word_time = &lrc_file.lyrics[3];
        assert_eq!(word_time.word_time_tags.len(), 3);
        assert_eq!(word_time.word_time_tags[0].2, "Word ");
        assert_eq!(word_time.word_time_tags[1].2, "by ");
        assert_eq!(word_time.word_time_tags[2].2, "word");

        assert_eq!(word_time.word_time_tags[0].0.to_string(), "[00:24.00]");
        assert_eq!(word_time.word_time_tags[1].0.to_string(), "[00:24.50]");
        assert_eq!(word_time.word_time_tags[2].0.to_string(), "[00:25.00]");

        Ok(())
    }
}
