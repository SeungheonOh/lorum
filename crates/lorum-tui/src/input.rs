use std::borrow::Cow;
use std::path::Path;

use crossterm::event::{KeyCode, KeyModifiers};
use reedline::{
    default_emacs_keybindings, EditCommand, Emacs, FileBackedHistory, Prompt, PromptEditMode,
    PromptHistorySearch, PromptHistorySearchStatus, Reedline, ReedlineEvent, Signal,
};

pub enum InputSignal {
    Line(String),
    Eof,
    Interrupted,
}

struct LorumPrompt {
    model: String,
}

impl Prompt for LorumPrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        Cow::Owned(format!("lorum [{}]", self.model))
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_indicator(&self, _prompt_mode: PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("> ")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(".. ")
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };
        Cow::Owned(format!("({}reverse-search: {}) ", prefix, history_search.term))
    }
}

pub struct InputReader {
    editor: Reedline,
    prompt: LorumPrompt,
}

impl InputReader {
    pub fn new(history_path: &Path, model: &str) -> Result<Self, String> {
        if let Some(parent) = history_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let history = Box::new(
            FileBackedHistory::with_file(1000, history_path.to_path_buf())
                .map_err(|e| format!("failed to open history: {e}"))?,
        );

        let mut keybindings = default_emacs_keybindings();
        keybindings.add_binding(
            KeyModifiers::ALT,
            KeyCode::Enter,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );

        let edit_mode = Box::new(Emacs::new(keybindings));

        let editor = Reedline::create()
            .with_history(history)
            .with_edit_mode(edit_mode);

        let prompt = LorumPrompt {
            model: model.to_string(),
        };

        Ok(Self { editor, prompt })
    }

    pub fn read_line(&mut self) -> Result<InputSignal, String> {
        match self.editor.read_line(&self.prompt) {
            Ok(Signal::Success(line)) => Ok(InputSignal::Line(line)),
            Ok(Signal::CtrlC) => Ok(InputSignal::Interrupted),
            Ok(Signal::CtrlD) => Ok(InputSignal::Eof),
            Err(e) => Err(format!("input error: {e}")),
        }
    }

    pub fn update_prompt(&mut self, model: &str) {
        self.prompt.model = model.to_string();
    }
}
