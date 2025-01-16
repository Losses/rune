use std::borrow::Cow::{self, Borrowed, Owned};

use rustyline::completion::FilenameCompleter;
use rustyline::highlight::{Highlighter, MatchingBracketHighlighter};
use rustyline::hint::Hinter;
use rustyline::history::SearchDirection;
use rustyline::validate::MatchingBracketValidator;
use rustyline_derive::{Completer, Helper, Validator};

#[derive(Helper, Completer, Validator)]
pub struct DIYHinter {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    colored_prompt: String,
    pub fs: std::sync::Arc<tokio::sync::RwLock<crate::fs::VirtualFS>>,
}

impl DIYHinter {
    pub fn new(fs: std::sync::Arc<tokio::sync::RwLock<crate::fs::VirtualFS>>) -> Self {
        Self {
            completer: FilenameCompleter::new(),
            highlighter: MatchingBracketHighlighter::new(),
            validator: MatchingBracketValidator::new(),
            colored_prompt: String::new(),
            fs,
        }
    }

    pub fn set_colored_prompt(&mut self, prompt: String) {
        self.colored_prompt = format!("\x1b[1;32m{}\x1b[0m", prompt);
    }
}

impl Hinter for DIYHinter {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &rustyline::Context<'_>) -> Option<String> {
        if line.is_empty() || pos < line.len() {
            return None;
        }

        let start = if ctx.history_index() == ctx.history().len() {
            ctx.history_index().saturating_sub(1)
        } else {
            ctx.history_index()
        };

        if let Some(sr) = ctx
            .history()
            .starts_with(line, start, SearchDirection::Reverse)
            .unwrap_or(None)
        {
            if sr.entry == line {
                return None;
            }

            // Convert byte index to character index
            let char_pos = line.chars().take(pos).count();
            let hint = sr.entry.chars().skip(char_pos).collect::<String>();
            return Some(hint);
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
