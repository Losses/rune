//! The thumbnailer programs registered in `thumbnailers/*.thumbnailer` under
//! the XDG data folders, the registry GNOME and tumbler read: each entry names
//! a program, its command line, and the file types it draws.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::mime::MimeDatabase;
use super::sandbox::{Sandbox, find_program};

/// How long a thumbnailer may take for one file.
const DEADLINE: Duration = Duration::from_secs(30);

/// One registered thumbnailer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    program: PathBuf,
    /// The command line after the program, with its field codes.
    arguments: Vec<String>,
}

/// The registered thumbnailers by the canonical type they draw.
#[derive(Debug, Default)]
pub(super) struct Thumbnailers {
    by_type: HashMap<String, Entry>,
}

impl Thumbnailers {
    /// Every registration in the XDG data folders whose program is installed.
    /// A type registered twice keeps the entry from the more important
    /// folder, and within a folder the first file by name.
    pub(super) fn load(data_dirs: &[PathBuf], mimes: &MimeDatabase) -> Self {
        let mut thumbnailers = Self::default();
        for dir in data_dirs {
            let Ok(entries) = fs::read_dir(dir.join("thumbnailers")) else {
                continue;
            };
            let mut files: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "thumbnailer"))
                .collect();
            files.sort();
            for file in files {
                if let Ok(text) = fs::read_to_string(&file) {
                    thumbnailers.add(&text, mimes);
                }
            }
        }
        thumbnailers
    }

    fn add(&mut self, text: &str, mimes: &MimeDatabase) {
        let Some(registration) = parse_registration(text) else {
            return;
        };
        let Some(program) = find_program(registration.try_exec.as_ref()) else {
            return;
        };
        let entry = Entry {
            program,
            arguments: registration.command[1..].to_vec(),
        };
        for mime in registration.types {
            self.by_type
                .entry(mimes.canonical(&mime))
                .or_insert_with(|| entry.clone());
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.by_type.is_empty()
    }

    pub(super) fn handles(&self, mime: &str) -> bool {
        self.by_type.contains_key(mime)
    }

    /// The PNG the thumbnailer for `mime` draws of `input` at `edge` pixels,
    /// run inside `sandbox`. `None` when none is registered or it fails.
    pub(super) fn draw(
        &self,
        sandbox: &Sandbox,
        mime: &str,
        input: &Path,
        uri: &str,
        edge: u32,
    ) -> Option<Vec<u8>> {
        let entry = self.by_type.get(mime)?;
        let folder = tempfile::Builder::new()
            .prefix("rune-thumbnail-")
            .tempdir()
            .ok()?;
        let output = folder.path().join("thumbnail.png");
        let arguments = expand(&entry.arguments, edge, uri, input, &output);
        match sandbox.run(&entry.program, &arguments, input, folder.path(), DEADLINE) {
            Ok(true) => fs::read(&output).ok(),
            Ok(false) => None,
            Err(error) => {
                log::warn!("Could not run {:?}: {error}", entry.program);
                None
            }
        }
    }
}

/// What one `.thumbnailer` file registers.
#[derive(Debug, PartialEq, Eq)]
struct Registration {
    try_exec: String,
    command: Vec<String>,
    types: Vec<String>,
}

fn parse_registration(text: &str) -> Option<Registration> {
    let mut in_entry = false;
    let mut values: HashMap<&str, &str> = HashMap::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Thumbnailer Entry]";
            continue;
        }
        if !in_entry || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            values.insert(key.trim(), value.trim());
        }
    }
    let command = split_command(values.get("Exec")?)?;
    let try_exec = values
        .get("TryExec")
        .map(|value| (*value).to_owned())
        .or_else(|| command.first().cloned())?;
    let types = values
        .get("MimeType")?
        .split(';')
        .filter(|mime| !mime.is_empty())
        .map(str::to_owned)
        .collect();
    Some(Registration {
        try_exec,
        command,
        types,
    })
}

/// An `Exec` value split into arguments: spaces separate them, double
/// quotes group them, and a backslash inside quotes escapes the next
/// character, as in desktop entries.
fn split_command(exec: &str) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => current.push(chars.next()?),
            ' ' | '\t' if !quoted => {
                if started {
                    arguments.push(mem::take(&mut current));
                    started = false;
                }
            }
            other => {
                current.push(other);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    if started {
        arguments.push(current);
    }
    (!arguments.is_empty()).then_some(arguments)
}

/// The arguments with their field codes filled in: `%s` the size, `%u` the
/// URI, `%i` the input path, `%o` the output path, `%%` a percent sign.
fn expand(
    arguments: &[String],
    edge: u32,
    uri: &str,
    input: &Path,
    output: &Path,
) -> Vec<OsString> {
    arguments
        .iter()
        .map(|argument| {
            let mut expanded = OsString::new();
            let mut chars = argument.chars();
            while let Some(ch) = chars.next() {
                if ch != '%' {
                    expanded.push(ch.encode_utf8(&mut [0; 4]));
                    continue;
                }
                match chars.next() {
                    Some('s') => expanded.push(edge.to_string()),
                    Some('u') => expanded.push(uri),
                    Some('i') => expanded.push(input),
                    Some('o') => expanded.push(output),
                    Some('%') => expanded.push("%"),
                    _ => {}
                }
            }
            expanded
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registrations_name_the_program_command_and_types() {
        let text = "[Thumbnailer Entry]\n\
                    TryExec=ffmpegthumbnailer\n\
                    Exec=ffmpegthumbnailer -s %s -i %i -o %o -c png -a\n\
                    MimeType=audio/flac;audio/x-flac;\n\
                    \n\
                    [Other]\n\
                    Exec=ignored\n";
        assert_eq!(
            parse_registration(text),
            Some(Registration {
                try_exec: "ffmpegthumbnailer".into(),
                command: [
                    "ffmpegthumbnailer",
                    "-s",
                    "%s",
                    "-i",
                    "%i",
                    "-o",
                    "%o",
                    "-c",
                    "png",
                    "-a"
                ]
                .map(String::from)
                .to_vec(),
                types: vec!["audio/flac".into(), "audio/x-flac".into()],
            })
        );
        assert_eq!(
            parse_registration("[Thumbnailer Entry]\nMimeType=a/b;\n"),
            None
        );
    }

    #[test]
    fn commands_split_on_spaces_outside_quotes() {
        assert_eq!(
            split_command(r#"thumb "--title=Live \"2019\"" %u  %o"#),
            Some(vec![
                "thumb".into(),
                r#"--title=Live "2019""#.into(),
                "%u".into(),
                "%o".into()
            ])
        );
        assert_eq!(split_command(r#"thumb "unclosed"#), None);
        assert_eq!(split_command("   "), None);
    }

    #[test]
    fn field_codes_expand_inside_arguments() {
        let arguments = [
            "-s".to_string(),
            "%s".into(),
            "--in=%i".into(),
            "%u".into(),
            "%o".into(),
            "100%%".into(),
            "%x".into(),
        ];
        let expanded = expand(
            &arguments,
            256,
            "file:///music/a%20b.flac",
            Path::new("/music/a b.flac"),
            Path::new("/tmp/out.png"),
        );
        assert_eq!(
            expanded,
            [
                "-s",
                "256",
                "--in=/music/a b.flac",
                "file:///music/a%20b.flac",
                "/tmp/out.png",
                "100%",
                ""
            ]
            .map(OsString::from)
            .to_vec()
        );
    }
}
