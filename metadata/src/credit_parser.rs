//! Parse the tag layouts used for free-form personnel credits.

use std::borrow::Cow;

use crate::credit_roles::ROLES;

const INVOLVED: &[&str] = &["INVOLVEDPEOPLE", "TIPL", "IPLS", "TMCL"];

pub(super) struct Credit<'a> {
    pub name: &'a str,
    pub role: Cow<'a, str>,
}

impl<'a> Credit<'a> {
    fn new(name: &'a str, role: &'a str) -> Option<Self> {
        let (name, role) = (name.trim(), role.trim());
        if name.is_empty() || role.is_empty() {
            return None;
        }
        // Roon writes qualifiers with a space before the opening bracket.
        let role = match role.split_once('[') {
            Some((base, qualifier)) if !base.is_empty() && qualifier.ends_with(']') => {
                Cow::Owned(format!("{} [{qualifier}", base.trim_end()))
            }
            _ => Cow::Borrowed(role),
        };
        Some(Self { name, role })
    }
}

pub(super) fn canonical_roles(role: &str) -> &'static [&'static str] {
    let key = role.to_lowercase();
    ROLES
        .binary_search_by_key(&key.as_str(), |(alias, _)| alias)
        .map_or(&[], |index| ROLES[index].1)
}

pub(super) fn structured_credits<'a>(key: &'a str, value: &'a str) -> Vec<Credit<'a>> {
    if let Some((prefix, role)) = key.split_once(':')
        && INVOLVED
            .iter()
            .chain(["PERFORMER", "MUSICIANCREDITS", "CREDIT"].iter())
            .any(|kind| prefix.eq_ignore_ascii_case(kind))
    {
        return Credit::new(value, role).into_iter().collect();
    }

    let mut credits = Vec::new();
    if key.eq_ignore_ascii_case("PERSONNEL") {
        for line in value.lines() {
            personnel_line(line, &mut credits);
        }
    } else if INVOLVED.iter().any(|kind| key.eq_ignore_ascii_case(kind))
        || key.eq_ignore_ascii_case("PERFORMER")
    {
        for line in value.lines() {
            involved_line(line, &mut credits);
        }
    }
    credits
}

fn personnel_line<'a>(line: &'a str, credits: &mut Vec<Credit<'a>>) {
    let Some((left, right)) = ["\t", "  ", " - "]
        .iter()
        .find_map(|separator| line.split_once(separator))
    else {
        return;
    };

    // Some producers use role-first PERSONNEL rows. Use the vocabulary to recognize them.
    let role_first = Credit::new(right, left)
        .is_some_and(|credit| !canonical_roles(&credit.role).is_empty())
        && !right
            .split(',')
            .any(|part| !canonical_roles(part.trim()).is_empty());
    for description in right.split(',') {
        let entry = if role_first {
            Credit::new(description, left)
        } else {
            Credit::new(left, description)
        };
        credits.extend(entry);
    }
}

fn involved_line<'a>(line: &'a str, credits: &mut Vec<Credit<'a>>) {
    if let Some((name, role)) = parenthesized_role(line) {
        credits.extend(Credit::new(name, role));
        return;
    }

    // NUL and alternating semicolon encodings store the role before the name.
    if let Some(separator) = ["\0", "0x00"]
        .into_iter()
        .find(|separator| line.contains(separator))
    {
        append_pairs(&line.split(separator).collect::<Vec<_>>(), credits);
    } else if line.contains(';') {
        let parts: Vec<_> = line
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if parts.iter().all(|part| part.contains(':')) {
            credits.extend(parts.into_iter().filter_map(|part| {
                let (role, name) = part.split_once(':')?;
                Credit::new(name, role)
            }));
        } else {
            append_pairs(&parts, credits);
        }
    }
}

fn append_pairs<'a>(parts: &[&'a str], credits: &mut Vec<Credit<'a>>) {
    let (pairs, remainder) = parts.as_chunks::<2>();
    if remainder.is_empty() {
        credits.extend(
            pairs
                .iter()
                .filter_map(|[role, name]| Credit::new(name, role)),
        );
    }
}

fn parenthesized_role(line: &str) -> Option<(&str, &str)> {
    let line = line.trim().strip_suffix(')')?;
    let mut depth = 1;
    // Walk back from the closing delimiter so nested parentheses stay part of the role.
    for (start, character) in line.char_indices().rev() {
        match character {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 && !line[..start].trim().is_empty() {
                    return Some((&line[..start], &line[start + 1..]));
                }
            }
            _ => (),
        }
    }
    None
}
