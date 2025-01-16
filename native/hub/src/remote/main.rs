use radix_trie::{Trie, TrieCommon};
use rustyline::hint::{Hint, Hinter};
use rustyline::history::DefaultHistory;
use rustyline::Context;
use rustyline::{Editor, Result};
use rustyline_derive::{Completer, Helper, Highlighter, Validator};
use tracing_subscriber::EnvFilter;

#[derive(Completer, Helper, Validator, Highlighter)]
struct DIYHinter {
    hints: Trie<String, CommandHint>,
}

#[derive(Hash, Debug, PartialEq, Eq, Clone)]
struct CommandHint {
    display: String,
    complete_up_to: usize,
}

impl Hint for CommandHint {
    fn display(&self) -> &str {
        &self.display
    }

    fn completion(&self) -> Option<&str> {
        if self.complete_up_to > 0 {
            Some(&self.display[..self.complete_up_to])
        } else {
            None
        }
    }
}

impl CommandHint {
    fn new(text: &str, complete_up_to: &str) -> Self {
        assert!(text.starts_with(complete_up_to));
        Self {
            display: text.into(),
            complete_up_to: complete_up_to.len(),
        }
    }

    fn suffix(&self, strip_chars: usize) -> Self {
        Self {
            display: self.display[strip_chars..].to_owned(),
            complete_up_to: self.complete_up_to.saturating_sub(strip_chars),
        }
    }
}

impl Hinter for DIYHinter {
    type Hint = CommandHint;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<CommandHint> {
        if line.is_empty() || pos < line.len() {
            return None;
        }

        self.hints
            .get_raw_descendant(line)
            .and_then(|node| node.value())
            .map(|hint| hint.suffix(pos))
    }
}

fn diy_hints() -> Trie<String, CommandHint> {
    let mut trie = Trie::new();
    let commands = [
        ("help", "help"),
        ("get key", "get "),
        ("set key value", "set "),
        ("hget key field", "hget "),
        ("hset key field value", "hset "),
    ];

    for (text, complete_up_to) in commands {
        trie.insert(text.to_string(), CommandHint::new(text, complete_up_to));
    }
    trie
}

fn main() -> Result<()> {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    println!("Welcome to the Rune Speaker Command Line Interface");
    let h = DIYHinter { hints: diy_hints() };

    let mut rl: Editor<DIYHinter, DefaultHistory> = Editor::new()?;
    rl.set_helper(Some(h));

    loop {
        let input = rl.readline("> ")?;
        println!("input: {input}");
    }
}
