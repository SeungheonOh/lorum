use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use lorum_domain::{RuntimeEvent, SessionId};
use lorum_runtime::{RuntimeSubscriber, ToolCallDisplay};
use lorum_ui_core::{DefaultUiReducer, UiReducer};

use crate::render::{self, Skins};

struct StreamState {
    /// Number of visual rows the raw stream occupies (newlines + wraps).
    newline_count: usize,
    /// Current column position on the last line (for wrap detection).
    current_col: usize,
    /// Terminal width for wrap calculation.
    term_width: u16,
    /// Whether the "thinking..." indicator is currently shown.
    thinking_shown: bool,
    /// Whether any streaming text has been output for the current turn.
    has_stream_output: bool,
}

impl Default for StreamState {
    fn default() -> Self {
        Self {
            newline_count: 0,
            current_col: 0,
            term_width: 80,
            thinking_shown: false,
            has_stream_output: false,
        }
    }
}

pub struct CliSubscriber {
    reducer: Mutex<DefaultUiReducer>,
    skins: Skins,
    stream_state: Mutex<StreamState>,
    tool_display: Arc<dyn ToolCallDisplay>,
    /// Maps tool_call_id -> tool_name for use when ToolResultReceived arrives.
    tool_names: Mutex<HashMap<String, String>>,
    /// Child session IDs spawned by subagents; events for these are suppressed from the reducer.
    child_sessions: Mutex<HashSet<SessionId>>,
    /// Maps turn_id -> session_id for child sessions (to identify child events by turn_id).
    child_turn_sessions: Mutex<HashMap<lorum_domain::TurnId, SessionId>>,
}

impl CliSubscriber {
    pub fn new(skins: Skins, tool_display: Arc<dyn ToolCallDisplay>) -> Self {
        Self {
            reducer: Mutex::new(DefaultUiReducer::new()),
            skins,
            stream_state: Mutex::new(StreamState::default()),
            tool_display,
            tool_names: Mutex::new(HashMap::new()),
            child_sessions: Mutex::new(HashSet::new()),
            child_turn_sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Returns true if this event belongs to a child (subagent) session and should
    /// be suppressed from the main reducer and most rendering.
    fn is_child_session_event(&self, event: &RuntimeEvent) -> bool {
        match event {
            RuntimeEvent::TurnStarted {
                turn_id,
                session_id,
                ..
            } => {
                let is_child = self
                    .child_sessions
                    .lock()
                    .map(|s| s.contains(session_id))
                    .unwrap_or(false);
                if is_child {
                    if let Ok(mut map) = self.child_turn_sessions.lock() {
                        map.insert(turn_id.clone(), session_id.clone());
                    }
                }
                is_child
            }
            RuntimeEvent::UserMessageReceived { session_id, .. } => self
                .child_sessions
                .lock()
                .map(|s| s.contains(session_id))
                .unwrap_or(false),
            _ => {
                if let Some(turn_id) = event.turn_id() {
                    self.child_turn_sessions
                        .lock()
                        .map(|m| m.contains_key(turn_id))
                        .unwrap_or(false)
                } else {
                    false
                }
            }
        }
    }
}

impl RuntimeSubscriber for CliSubscriber {
    fn on_event(&self, event: &RuntimeEvent) {
        // Handle subagent lifecycle events first (they carry parent session_id)
        match event {
            RuntimeEvent::SubagentSpawned {
                child_session_id,
                agent_type,
                task_id,
                ..
            } => {
                if let Ok(mut children) = self.child_sessions.lock() {
                    children.insert(child_session_id.clone());
                }
                println!(
                    "{}",
                    render::render_subagent_spawned(agent_type, task_id, &self.skins)
                );
                return;
            }
            RuntimeEvent::SubagentCompleted {
                agent_type, status, ..
            } => {
                println!(
                    "{}",
                    render::render_subagent_completed(agent_type, status, &self.skins)
                );
                return;
            }
            _ => {}
        }

        // Suppress child session events from main reducer and rendering
        if self.is_child_session_event(event) {
            return;
        }

        if let Ok(mut reducer) = self.reducer.lock() {
            if let Err(err) = reducer.apply(event) {
                eprintln!("ui reducer rejected runtime event: {err}");
            }
        }

        match event {
            RuntimeEvent::TurnStarted { turn_id, .. } => {
                if let Ok(mut state) = self.stream_state.lock() {
                    state.thinking_shown = true;
                    state.has_stream_output = false;
                    state.newline_count = 0;
                    state.current_col = 0;
                    state.term_width = crossterm::terminal::size()
                        .map(|(w, _)| w)
                        .unwrap_or(80);
                }
                println!(
                    "{}",
                    render::render_turn_started(turn_id.as_str(), &self.skins)
                );
                let _ = io::stdout().flush();
            }
            RuntimeEvent::AssistantStreamDelta { delta, .. } => {
                let should_clear_thinking = {
                    let mut state = match self.stream_state.lock() {
                        Ok(s) => s,
                        Err(e) => e.into_inner(),
                    };

                    let clear_thinking = state.thinking_shown;
                    if state.thinking_shown {
                        state.thinking_shown = false;
                    }
                    state.has_stream_output = true;

                    let tw = state.term_width as usize;
                    for ch in delta.chars() {
                        if ch == '\n' {
                            state.newline_count += 1;
                            state.current_col = 0;
                        } else {
                            state.current_col += 1;
                            if tw > 0 && state.current_col >= tw {
                                state.newline_count += 1;
                                state.current_col = 0;
                            }
                        }
                    }

                    clear_thinking
                };

                if should_clear_thinking {
                    print!("{}", render::clear_last_n_lines(1));
                }
                print!("{delta}");
                let _ = io::stdout().flush();
            }
            RuntimeEvent::AssistantThinkingDelta { delta, .. } => {
                let should_clear_thinking = {
                    let mut state = match self.stream_state.lock() {
                        Ok(s) => s,
                        Err(e) => e.into_inner(),
                    };

                    let clear = state.thinking_shown;
                    if state.thinking_shown {
                        state.thinking_shown = false;
                    }
                    clear
                };

                if should_clear_thinking {
                    print!("{}", render::clear_last_n_lines(1));
                }
                render::print_thinking_delta(delta);
                let _ = io::stdout().flush();
            }
            RuntimeEvent::TurnFinished {
                turn_id,
                reason,
                message_id,
                ..
            } => {
                let (has_stream_output, thinking_shown, newline_count) = {
                    let state = self
                        .stream_state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    (
                        state.has_stream_output,
                        state.thinking_shown,
                        state.newline_count,
                    )
                };

                if has_stream_output {
                    let rendered = self.reducer.lock().ok().and_then(|reducer| {
                        reducer
                            .state()
                            .completed_turns
                            .iter()
                            .rev()
                            .find(|t| t.turn_id == *turn_id)
                            .map(|t| {
                                render::render_assistant_markdown(
                                    &t.assistant_text,
                                    &self.skins.assistant,
                                )
                            })
                    });

                    if let Some(rendered) = rendered {
                        if !rendered.is_empty() {
                            let clear = render::clear_last_n_lines(newline_count);
                            print!("{clear}{rendered}");
                        } else {
                            println!();
                        }
                    } else {
                        println!();
                    }
                } else if thinking_shown {
                    print!("{}", render::clear_last_n_lines(1));
                }

                println!(
                    "{}",
                    render::render_turn_finished(reason, message_id.as_ref(), &self.skins)
                );
            }
            RuntimeEvent::RuntimeError {
                code,
                message,
                turn_id,
                ..
            } => {
                let thinking_shown = self
                    .stream_state
                    .lock()
                    .map(|s| s.thinking_shown)
                    .unwrap_or(false);
                if thinking_shown {
                    print!("{}", render::clear_last_n_lines(1));
                }
                println!(
                    "\n{}",
                    render::render_error(
                        code,
                        &format!("[{}] {}", turn_id.as_str(), message),
                        &self.skins
                    )
                );
            }
            RuntimeEvent::ToolExecutionStart {
                tool_name,
                tool_call_id,
                arguments,
                ..
            } => {
                let summary = self.tool_display.format_call(tool_name, arguments);
                if let Ok(mut names) = self.tool_names.lock() {
                    names.insert(tool_call_id.clone(), tool_name.clone());
                }
                println!("{}", render::render_tool_start(&summary, &self.skins));
            }
            RuntimeEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } => {
                println!(
                    "{}",
                    render::render_tool_end(tool_name, *is_error, &self.skins)
                );
            }
            RuntimeEvent::ToolResultReceived {
                tool_call_id,
                is_error,
                result,
                ..
            } => {
                let tool_name = self
                    .tool_names
                    .lock()
                    .ok()
                    .and_then(|names| names.get(tool_call_id).cloned())
                    .unwrap_or_default();
                let summary = self
                    .tool_display
                    .format_result(&tool_name, *is_error, result);
                println!(
                    "{}",
                    render::render_tool_result(*is_error, &summary, &self.skins)
                );
            }
            RuntimeEvent::SessionSwitched { .. }
            | RuntimeEvent::UserMessageReceived { .. }
            | RuntimeEvent::SubagentSpawned { .. }
            | RuntimeEvent::SubagentCompleted { .. } => {}
        }
    }
}
