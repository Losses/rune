use std::borrow::Cow::{self, Borrowed, Owned};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use radix_trie::{Trie, TrieCommon};
use rustyline::completion::FilenameCompleter;
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter, MatchingBracketHighlighter};
use rustyline::hint::{Hint, Hinter, HistoryHinter};
use rustyline::sqlite_history::SQLiteHistory;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{Cmd, CompletionType, Config, Context, EditMode, Editor, KeyEvent, Result};
use rustyline_derive::{Completer, Helper, Validator};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Debug)]
pub struct VirtualEntry {
    pub name: String,
    pub id: Option<i32>,
    pub is_directory: bool,
}

pub struct VirtualFS {
    pub current_path: PathBuf,
    pub root_dirs: Vec<String>,
    pub subdirs: HashMap<String, Vec<VirtualEntry>>,
}

impl VirtualFS {
    fn new() -> Self {
        let root_dirs = vec![
            "Artists".to_string(),
            "Playlists".to_string(),
            "Tracks".to_string(),
            "Albums".to_string(),
            "Mixes".to_string(),
        ];

        Self {
            current_path: PathBuf::from("/"),
            root_dirs,
            subdirs: HashMap::new(),
        }
    }

    fn current_dir(&self) -> String {
        self.current_path.to_string_lossy().to_string()
    }

    async fn list_current_dir(&self) -> Vec<VirtualEntry> {
        if self.current_path == Path::new("/") {
            return self
                .root_dirs
                .iter()
                .map(|name| VirtualEntry {
                    name: name.clone(),
                    id: None,
                    is_directory: true,
                })
                .collect();
        }

        let current_dir = self
            .current_path
            .components()
            .last()
            .unwrap()
            .as_os_str()
            .to_string_lossy()
            .to_string();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        self.subdirs
            .get(&current_dir)
            .cloned()
            .unwrap_or_else(std::vec::Vec::new)
    }
}

#[derive(Helper, Completer, Validator)]
pub struct DIYHinter {
    #[rustyline(Completer)]
    completer: FilenameCompleter,
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
    hints: Trie<String, CommandHint>,
    colored_prompt: String,
    history_hinter: HistoryHinter,
    pub fs: Arc<RwLock<VirtualFS>>,
}

#[derive(Hash, Debug, PartialEq, Eq, Clone)]
pub struct CommandHint {
    pub display: String,
    pub complete_up_to: usize,
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

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<CommandHint> {
        if line.is_empty() || pos < line.len() {
            return None;
        }

        let command_hint = self
            .hints
            .get_raw_descendant(line)
            .and_then(|node| node.value())
            .map(|hint| hint.suffix(pos));

        if command_hint.is_none() {
            if let Some(history_hint) = self.history_hinter.hint(line, pos, ctx) {
                return Some(CommandHint {
                    display: history_hint,
                    complete_up_to: 0,
                });
            }
        }

        command_hint
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

    fn highlight_char(&self, line: &str, pos: usize, kind: CmdKind) -> bool {
        self.highlighter.highlight_char(line, pos, kind)
    }
}

fn diy_hints() -> Trie<String, CommandHint> {
    let mut trie = Trie::new();
    let commands = [
        ("ls", "ls"),
        ("cwd", "cwd"),
        ("cd ..", "cd .."),
        ("cd /", "cd /"),
    ];

    for (text, complete_up_to) in commands {
        trie.insert(text.to_string(), CommandHint::new(text, complete_up_to));
    }
    trie
}

#[tokio::main]
async fn main() -> Result<()> {
    let filter = EnvFilter::new(
        "symphonia_format_ogg=off,symphonia_core=off,symphonia_bundle_mp3::demuxer=off,tantivy::directory=off,tantivy::indexer=off,sea_orm_migration::migrator=off,info",
    );

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_test_writer()
        .init();

    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(EditMode::Emacs)
        .auto_add_history(true)
        .build();

    let history = SQLiteHistory::with_config(config)?;

    let fs = Arc::new(RwLock::new(VirtualFS::new()));

    let h = DIYHinter {
        completer: FilenameCompleter::new(),
        highlighter: MatchingBracketHighlighter::new(),
        hints: diy_hints(),
        colored_prompt: "".to_owned(),
        validator: MatchingBracketValidator::new(),
        history_hinter: HistoryHinter::new(),
        fs: fs.clone(),
    };

    let mut rl: Editor<DIYHinter, _> = Editor::with_history(config, history)?;
    rl.set_helper(Some(h));
    rl.bind_sequence(KeyEvent::alt('n'), Cmd::HistorySearchForward);
    rl.bind_sequence(KeyEvent::alt('p'), Cmd::HistorySearchBackward);

    println!("Welcome to the Rune Speaker Command Line Interface");

    loop {
        let current_dir = {
            let fs = fs.read().await;
            fs.current_dir()
        };
        let p = format!("{}> ", current_dir);
        rl.helper_mut().expect("No helper").colored_prompt = format!("\x1b[1;32m{p}\x1b[0m");

        let readline = rl.readline(&p);
        match readline {
            Ok(line) => {
                let command = line.trim();
                match command {
                    "ls" => {
                        let fs = fs.read().await;
                        let entries = fs.list_current_dir().await;
                        for entry in entries {
                            let entry_type = if entry.is_directory { "DIR" } else { "FILE" };
                            let id_str =
                                entry.id.map(|id| format!(" [{}]", id)).unwrap_or_default();
                            println!("{:<4} {}{}", entry_type, entry.name, id_str);
                        }
                    }
                    "cwd" => {
                        let fs = fs.read().await;
                        println!("Current directory: {}", fs.current_dir());
                    }
                    cmd if cmd.starts_with("cd ") => {
                        let path = cmd[3..].trim();
                        let mut fs = fs.write().await;
                        match path {
                            ".." => {
                                if fs.current_path != Path::new("/") {
                                    fs.current_path.pop();
                                }
                            }
                            "/" => {
                                fs.current_path = PathBuf::from("/");
                            }
                            _ => {
                                // 这里可以添加进入子目录的逻辑
                                if fs.root_dirs.contains(&path.to_string()) {
                                    fs.current_path = PathBuf::from("/").join(path);
                                } else {
                                    println!("Directory not found: {}", path);
                                }
                            }
                        }
                    }
                    "" => {}
                    _ => println!("Unknown command: {}", command),
                }
            }
            Err(ReadlineError::Interrupted) => {
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("Encountered Eof");
                break;
            }
            Err(err) => {
                println!("Error: {err:?}");
                break;
            }
        }
    }

    Ok(())
}
