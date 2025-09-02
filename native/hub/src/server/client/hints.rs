use std::{
    borrow::Cow::{self, Borrowed, Owned},
    path::{Path, PathBuf},
    sync::Arc,
};

use rustyline::{
    completion::FilenameCompleter,
    highlight::{Highlighter, MatchingBracketHighlighter},
    hint::Hinter,
    history::SearchDirection,
    validate::MatchingBracketValidator,
};
use rustyline_derive::{Completer, Helper, Validator};
use tokio::sync::RwLock;

use crate::fs::VirtualFS;

#[derive(Helper, Completer, Validator)]
pub struct DIYHinter {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    colored_prompt: String,
    pub fs: Arc<RwLock<VirtualFS>>,
}

impl DIYHinter {
    pub fn new(fs: Arc<RwLock<VirtualFS>>) -> Self {
        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            validator: MatchingBracketValidator::new(),
            colored_prompt: String::new(),
            fs,
        }
    }

    pub fn set_colored_prompt(&mut self, prompt: String) {
        self.colored_prompt = format!("\x1b[1;32m{prompt}\x1b[0m");
    }

    // Helper function to find the common prefix between the current input and directory entry
    fn find_matching_entry(&self, current_path: &Path, partial_input: &str) -> Option<String> {
        // Get read lock on filesystem
        if let Ok(fs) = self.fs.try_read() {
            // Check if we have cache entry for current path
            if let Some(cache_entry) = fs.cache.get(current_path) {
                // Find first entry that starts with our partial input
                if let Some(entry) = cache_entry
                    .entries
                    .iter()
                    .find(|e| e.name.starts_with(partial_input))
                {
                    // Return remaining part of the matching entry name
                    return Some(entry.name[partial_input.len()..].to_string());
                }
            }
        }
        None
    }

    // Parse the input line to extract current directory path and partial input
    fn parse_input(&self, line: &str) -> Option<(PathBuf, String)> {
        // Get last component from input as partial text
        let parts: Vec<&str> = line.rsplitn(2, '/').collect();
        let (partial, path_str) = match parts.as_slice() {
            [partial] => (partial.to_string(), "/"),
            [partial, path] => (partial.to_string(), *path),
            _ => return None,
        };

        // Convert path string to PathBuf
        let path = if path_str.starts_with('/') {
            PathBuf::from(path_str)
        } else {
            // If relative path, combine with current directory
            if let Ok(fs) = self.fs.try_read() {
                fs.current_path.join(path_str)
            } else {
                return None;
            }
        };

        Some((path, partial))
    }
}

impl Hinter for DIYHinter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        // Return early if line is empty or cursor is not at end
        if line.is_empty() || pos < line.len() {
            return None;
        }

        // First try history-based completion
        let start = if ctx.history_index() == ctx.history().len() {
            ctx.history_index().saturating_sub(1)
        } else {
            ctx.history_index()
        };

        if let Some(sr) = ctx
            .history()
            .starts_with(line, start, SearchDirection::Reverse)
            .unwrap_or(None)
            && sr.entry != line
        {
            let char_pos = line.chars().take(pos).count();
            return Some(sr.entry.chars().skip(char_pos).collect());
        }

        // If no history match, try filesystem-based completion
        if let Some((current_path, partial)) = self.parse_input(line) {
            return self.find_matching_entry(&current_path, &partial);
        }

        None
    }
}

impl Highlighter for DIYHinter {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        default: bool,
    ) -> Cow<'b, str> {
        if default {
            Borrowed(&self.colored_prompt)
        } else {
            Borrowed(prompt)
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Owned("\x1b[1m".to_owned() + hint + "\x1b[m")
    }

    fn highlight<'l>(&self, line: &'l str, pos: usize) -> Cow<'l, str> {
        self.highlighter.highlight(line, pos)
    }

    fn highlight_char(&self, line: &str, pos: usize, kind: rustyline::highlight::CmdKind) -> bool {
        self.highlighter.highlight_char(line, pos, kind)
    }
}
