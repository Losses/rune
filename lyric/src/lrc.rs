use std::str::FromStr;

use anyhow::Result;

use crate::{
    types::{LyricFile, LyricLine, TimeTag, VoiceType},
    utils::{push_section, strip_bom},
};

const DUMMY_END_TIME: TimeTag = TimeTag {
    minutes: 9999,
    seconds: 0,
    milliseconds: 0,
};

type Section = (TimeTag, TimeTag, String);

pub fn parse_lrc(content: &str) -> Result<LyricFile> {
    let mut lrc = LyricFile::new();

    for line in strip_bom(content).lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse ID tags
        if line.starts_with('[') && !line[1..].starts_with(char::is_numeric) {
            if let Some((key, value)) = line
                .strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
                .and_then(|l| l.split_once(':'))
            {
                lrc.metadata.insert(key.to_string(), value.to_string());
            }
            continue;
        }

        // Parse lyric lines, one per leading time tag
        let Some((start_times, remaining_content)) = split_time_tags(line) else {
            continue; // Invalid line
        };

        let remaining_content = remaining_content.trim();

        // Extract lyric text and voice type
        let (voice_type, remaining_content) =
            if let Some(stripped) = remaining_content.strip_prefix("M:") {
                (VoiceType::Male, stripped.trim())
            } else if let Some(stripped) = remaining_content.strip_prefix("F:") {
                (VoiceType::Female, stripped.trim())
            } else if let Some(stripped) = remaining_content.strip_prefix("D:") {
                (VoiceType::Duet, stripped.trim())
            } else {
                (VoiceType::Default, remaining_content)
            };

        for start_time in start_times {
            // Parse enhanced format word-level time tags
            let word_time_tags = parse_enhanced_lrc(remaining_content, &start_time);
            let text = word_time_tags.iter().map(|tag| tag.2.as_str()).collect();

            lrc.lyrics.push(LyricLine {
                start_time,
                end_time: DUMMY_END_TIME, // Temporary, will be updated below
                voice_type: voice_type.clone(),
                text,
                word_time_tags,
            });
        }
    }

    apply_offset(&mut lrc);
    lrc.lyrics.sort_by_key(|line| line.start_time.as_millis());
    update_end_times(&mut lrc.lyrics);

    Ok(lrc)
}

/// Splits the leading `[mm:ss.xx]` tags from the rest of the line.
/// Returns `None` when the first tag is not a valid time tag.
fn split_time_tags(line: &str) -> Option<(Vec<TimeTag>, &str)> {
    let mut tags = Vec::new();
    let mut rest = line;

    while rest.starts_with('[') {
        let Some(close) = rest.find(']') else { break };
        match TimeTag::from_str(&rest[..=close]) {
            Ok(tag) => tags.push(tag),
            // Something like `[Chorus]` after the time tags is lyric text
            Err(_) if !tags.is_empty() => break,
            Err(_) => return None,
        }
        rest = &rest[close + 1..];
    }

    if tags.is_empty() {
        None
    } else {
        Some((tags, rest))
    }
}

/// `[offset:+500]` makes the lyrics appear 500 ms sooner.
fn apply_offset(lrc: &mut LyricFile) {
    let Some(offset) = lrc
        .metadata
        .get("offset")
        .and_then(|value| value.trim().parse::<i64>().ok())
        .filter(|offset| *offset != 0)
    else {
        return;
    };

    let shift = |time: &mut TimeTag| {
        if *time != DUMMY_END_TIME {
            let shifted = (time.as_millis() as i64).saturating_sub(offset).max(0);
            *time = TimeTag::from_millis(shifted as u64);
        }
    };

    for line in &mut lrc.lyrics {
        shift(&mut line.start_time);
        for (start, end, _) in &mut line.word_time_tags {
            shift(start);
            shift(end);
        }
    }
}

/// A line ends where the next line with a later start time begins, so lines
/// sharing a start time (e.g. a translation) are displayed together.
fn update_end_times(lyrics: &mut [LyricLine]) {
    let mut end_time = DUMMY_END_TIME;

    for i in (0..lyrics.len()).rev() {
        if let Some(next) = lyrics.get(i + 1)
            && next.start_time.as_millis() > lyrics[i].start_time.as_millis()
        {
            end_time = next.start_time.clone();
        }

        let line = &mut lyrics[i];
        // Absurd times past DUMMY_END_TIME must not end before they start
        line.end_time = end_time.clone().max_millis(&line.start_time);
        let line_end = line.end_time.clone();
        // A trailing `<mm:ss.xx>` tag says when the last word ends; keep it
        // unless it runs past the line. Without one, the word lasts the line.
        if let Some(last_detail) = line.word_time_tags.last_mut()
            && (last_detail.1 == DUMMY_END_TIME || last_detail.1.as_millis() > line_end.as_millis())
        {
            last_detail.1 = line_end;
        }
    }
}

/// Splits a line into word sections at its `<mm:ss.xx>` tags. Text without
/// any valid tag becomes a single section covering the whole line.
fn parse_enhanced_lrc(content: &str, line_start: &TimeTag) -> Vec<Section> {
    // Find the valid word tags; a stray `<` (e.g. `I <3 you`) is just text
    let mut tags: Vec<(usize, usize, TimeTag)> = Vec::new();
    let mut search_from = 0;
    while let Some(pos) = content[search_from..].find('<') {
        let start = search_from + pos;
        let parsed = content[start..].find('>').and_then(|end| {
            Some((
                start + end + 1,
                TimeTag::from_str(&content[start..=start + end]).ok()?,
            ))
        });
        match parsed {
            Some((end, tag)) => {
                tags.push((start, end, tag));
                search_from = end;
            }
            None => search_from = start + 1,
        }
    }

    let mut sections: Vec<Section> = Vec::new();
    if tags.is_empty() {
        return vec![(line_start.clone(), DUMMY_END_TIME, content.to_string())];
    }

    // Text before the first word tag starts with the line
    push_section(&mut sections, line_start, &tags[0].2, &content[..tags[0].0]);

    for (i, (_, end, start_time)) in tags.iter().enumerate() {
        let (next_start, end_time) = match tags.get(i + 1) {
            Some((next_start, _, next_time)) => (*next_start, next_time),
            None => (content.len(), &DUMMY_END_TIME),
        };
        push_section(
            &mut sections,
            start_time,
            end_time,
            &content[*end..next_start],
        );
    }

    if sections.is_empty() {
        sections.push((line_start.clone(), DUMMY_END_TIME, String::new()));
    }
    sections
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
        assert_eq!(male.text, "Male line");
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

    fn ms(tag: &TimeTag) -> u64 {
        tag.as_millis()
    }

    #[test]
    fn test_lrc_malformed_id_tags() -> Result<()> {
        let lrc_file = parse_lrc("[ti:\n[ti:歌\n[nocolon]\n[ar:Ok]\n[00:01.00]x")?;
        assert_eq!(lrc_file.metadata.len(), 1);
        assert_eq!(lrc_file.metadata.get("ar").unwrap(), "Ok");
        assert_eq!(lrc_file.lyrics.len(), 1);
        Ok(())
    }

    #[test]
    fn test_lrc_lenient_time_tags_skip_bad_lines() -> Result<()> {
        let lrc_file = parse_lrc(
            "[00:01]a\n[00:02.5]b\n[00:03.50]c\n[00:04.500]d\n[00:05:50]e\n[01:00:06.00]f\n[xx:yy]bad\n[00:99.9999]bad\n[00:07.00]g",
        )?;
        let starts: Vec<u64> = lrc_file.lyrics.iter().map(|l| ms(&l.start_time)).collect();
        assert_eq!(starts, [1000, 2500, 3500, 4500, 5500, 7000, 3_606_000]);

        // Nothing parseable is an empty result, not an error
        assert!(parse_lrc("[xx:yy]bad")?.lyrics.is_empty());
        Ok(())
    }

    #[test]
    fn test_lrc_huge_minutes_do_not_overflow() -> Result<()> {
        let lrc_file = parse_lrc("[99999999999:00.00]x")?;
        assert_eq!(lrc_file.lyrics.len(), 0);
        let lrc_file = parse_lrc("[4294967295:59.99]x")?;
        assert_eq!(i32::from(lrc_file.lyrics[0].start_time.clone()), i32::MAX);
        Ok(())
    }

    #[test]
    fn test_lrc_sorted_and_same_start_group() -> Result<()> {
        let lrc_file = parse_lrc("[00:10.00]c\n[00:05.00]a\n[00:05.00]a translation\n[00:08.00]b")?;
        let texts: Vec<&str> = lrc_file.lyrics.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["a", "a translation", "b", "c"]);

        let ends: Vec<u64> = lrc_file.lyrics.iter().map(|l| ms(&l.end_time)).collect();
        assert_eq!(ends[..3], [8000, 8000, 10_000]);
        assert_eq!(lrc_file.lyrics[3].end_time, DUMMY_END_TIME);
        // The last section follows the line
        assert_eq!(ms(&lrc_file.lyrics[0].word_time_tags[0].1), 8000);
        assert_eq!(lrc_file.lyrics[3].word_time_tags[0].1, DUMMY_END_TIME);
        Ok(())
    }

    #[test]
    fn test_lrc_multiple_time_tags() -> Result<()> {
        let lrc_file = parse_lrc("[00:05.00][00:01.00]hi\n[00:03.00][Chorus] yo")?;
        let lines: Vec<(u64, &str)> = lrc_file
            .lyrics
            .iter()
            .map(|l| (ms(&l.start_time), l.text.as_str()))
            .collect();
        assert_eq!(lines, [(1000, "hi"), (3000, "[Chorus] yo"), (5000, "hi")]);
        Ok(())
    }

    #[test]
    fn test_lrc_offset() -> Result<()> {
        let lrc_file = parse_lrc("[00:01.00]a\n[offset:+500]\n[00:00.20]<00:00.20>b <00:00.80>c")?;
        // Positive offset makes lyrics appear sooner, clamped at 0
        assert_eq!(ms(&lrc_file.lyrics[0].start_time), 0);
        assert_eq!(ms(&lrc_file.lyrics[1].start_time), 500);
        let words = &lrc_file.lyrics[0].word_time_tags;
        assert_eq!(
            (ms(&words[0].0), ms(&words[0].1), ms(&words[1].0)),
            (0, 300, 300)
        );

        let lrc_file = parse_lrc("[offset:-500]\n[00:01.00]a")?;
        assert_eq!(ms(&lrc_file.lyrics[0].start_time), 1500);
        Ok(())
    }

    #[test]
    fn test_lrc_bom() -> Result<()> {
        let lrc_file = parse_lrc("\u{FEFF}[ti:Song]\n[00:01.00]a")?;
        assert_eq!(lrc_file.metadata.get("ti").unwrap(), "Song");
        assert_eq!(lrc_file.lyrics.len(), 1);
        Ok(())
    }

    #[test]
    fn test_lrc_enhanced_edge_cases() -> Result<()> {
        let lrc_file = parse_lrc(
            "[00:01.00]Intro <00:02.00>word\n[00:03.00]I <3 you\n[00:04.00]<01:19.310>Time <01:19.310>is <01:19.310>money\n[00:05.00]  spaced  \n[00:06.00]",
        )?;
        let sections = |i: usize| -> Vec<&str> {
            lrc_file.lyrics[i]
                .word_time_tags
                .iter()
                .map(|s| s.2.as_str())
                .collect()
        };
        assert_eq!(sections(0), ["Intro ", "word"]);
        assert_eq!(ms(&lrc_file.lyrics[0].word_time_tags[0].0), 1000);
        assert_eq!(ms(&lrc_file.lyrics[0].word_time_tags[0].1), 2000);
        assert_eq!(sections(1), ["I <3 you"]);
        assert_eq!(sections(2), ["Time ", "is ", "money"]);
        assert_eq!(lrc_file.lyrics[2].text, "Time is money");
        assert_eq!(sections(3), ["spaced"]);
        assert_eq!(sections(4), [""]);
        Ok(())
    }

    #[test]
    fn test_trailing_word_tag_keeps_the_last_word_end() -> Result<()> {
        let lyrics = parse_lrc(
            "[00:01.00]<00:01.00>one <00:01.50>two<00:02.00>\n\
             [00:05.00]<00:05.00>last<00:05.80>\n\
             [00:03.00]<00:03.00>open",
        )?
        .lyrics;
        let last_word_end =
            |i: usize| i32::from(lyrics[i].word_time_tags.last().unwrap().1.clone());

        assert_eq!(last_word_end(0), 2000); // as tagged, not the next line's start
        assert_eq!(last_word_end(1), 5000); // no trailing tag: lasts until the next line
        assert_eq!(last_word_end(2), 5800); // the last line keeps its tag too
        Ok(())
    }
}
