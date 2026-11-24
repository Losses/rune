use crate::hints::DIYHinter;
use rustyline::Cmd;
use rustyline::CompletionType;
use rustyline::EditMode;
use rustyline::KeyEvent;
use rustyline::sqlite_history::SQLiteHistory;
use rustyline::{Config, Editor, Result};
use std::sync as std_sync;
use tokio::sync as tokio_sync;

pub struct EditorConfig {
    history_ignore_space: bool,
    completion_type: rustyline::CompletionType,
    edit_mode: rustyline::EditMode,
    auto_add_history: bool,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self {
            history_ignore_space: true,
            completion_type: CompletionType::List,
            edit_mode: EditMode::Emacs,
            auto_add_history: true,
        }
    }
}

pub fn create_editor(
    config: EditorConfig,
    fs: std_sync::Arc<tokio_sync::RwLock<crate::fs::VirtualFS>>,
) -> Result<Editor<DIYHinter, SQLiteHistory>> {
    let config = Config::builder()
        .history_ignore_space(config.history_ignore_space)
        .completion_type(config.completion_type)
        .edit_mode(config.edit_mode)
        .auto_add_history(config.auto_add_history)
        .build();

    let history = SQLiteHistory::with_config(config)?;
    let mut editor = Editor::with_history(config, history)?;

    let hinter = DIYHinter::new(fs);
    editor.set_helper(Some(hinter));

    // Key bindings
    editor.bind_sequence(KeyEvent::alt('n'), Cmd::HistorySearchForward);
    editor.bind_sequence(KeyEvent::alt('p'), Cmd::HistorySearchBackward);

    Ok(editor)
}
