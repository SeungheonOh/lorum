use crossterm::style::{Color, Attribute};
use lorum_domain::{MessageId, TurnTerminalReason};
use lorum_runtime::{ToolCallSummary, ToolResultSummary};
use termimad::{CompoundStyle, MadSkin};

/// Pre-configured styles for all TUI rendering contexts.
pub struct Skins {
    pub assistant: MadSkin,
    dim: CompoundStyle,
    error: CompoundStyle,
    success: CompoundStyle,
    warning: CompoundStyle,
    diff_add: CompoundStyle,
    diff_remove: CompoundStyle,
    diff_hunk: CompoundStyle,
}

impl Skins {
    pub fn new() -> Self {
        Self {
            assistant: make_assistant_skin(),
            dim: CompoundStyle::with_fg(Color::DarkGrey),
            error: CompoundStyle::with_fg(Color::Red),
            success: CompoundStyle::with_fg(Color::Green),
            warning: CompoundStyle::with_fg(Color::Yellow),
            diff_add: CompoundStyle::with_fg(Color::Green),
            diff_remove: CompoundStyle::with_fg(Color::Red),
            diff_hunk: CompoundStyle::with_fg(Color::DarkCyan),
        }
    }
}

fn make_assistant_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    skin.headers[0].compound_style.set_fg(Color::Cyan);
    skin.headers[0].compound_style.add_attr(Attribute::Bold);
    skin.headers[1].compound_style.set_fg(Color::Cyan);
    skin.headers[2].compound_style.set_fg(Color::DarkCyan);
    skin.bold.set_fg(Color::White);
    skin.bold.add_attr(Attribute::Bold);
    skin.italic.add_attr(Attribute::Italic);
    skin.inline_code.set_bg(Color::AnsiValue(236));
    skin.code_block.compound_style.set_bg(Color::AnsiValue(236));
    skin
}

pub fn render_assistant_markdown(text: &str, skin: &MadSkin) -> String {
    if text.trim().is_empty() {
        return String::new();
    }
    format!("{}", skin.term_text(text))
}

/// Print a thinking delta inline with dim styling. Uses a thread-local to
/// track whether the `[thinking] ` label has already been printed for the
/// current thinking block.
pub fn print_thinking_delta(delta: &str) {
    use std::cell::Cell;
    thread_local! {
        static LABEL_PRINTED: Cell<bool> = const { Cell::new(false) };
    }

    let style = CompoundStyle::with_fg(Color::DarkGrey);

    LABEL_PRINTED.with(|printed| {
        if !printed.get() {
            print!("{} ", style.apply_to("[thinking]"));
            printed.set(true);
        }
    });

    // Print styled text; detect end-of-block if delta ends with newline
    for ch in delta.chars() {
        if ch == '\n' {
            println!();
            LABEL_PRINTED.with(|p| p.set(false));
        } else {
            print!("{}", style.apply_to(ch));
        }
    }
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

pub fn render_tool_start(summary: &ToolCallSummary, skins: &Skins) -> String {
    let header = match &summary.detail {
        Some(detail) => format!("  [tool] {} {}", summary.headline, detail),
        None => format!("  [tool] {}", summary.headline),
    };
    let mut out = format!("{}", skins.dim.apply_to(&header));
    if let Some(body) = &summary.body {
        out.push('\n');
        for line in body.lines() {
            let styled = render_diff_line(line, skins);
            out.push_str(&format!("  {styled}\n"));
        }
    }
    out
}

pub fn render_tool_end(tool_name: &str, is_error: bool, skins: &Skins) -> String {
    if is_error {
        format!(
            "  {} {}",
            skins.error.apply_to("\u{2717}"),
            skins.error.apply_to(tool_name)
        )
    } else {
        format!(
            "  {} {}",
            skins.success.apply_to("\u{2713}"),
            skins.success.apply_to(tool_name)
        )
    }
}

pub fn render_tool_result(is_error: bool, summary: &ToolResultSummary, skins: &Skins) -> String {
    let mut out = if is_error {
        format!(
            "{}",
            skins
                .error
                .apply_to(format!("  [result] ERR {}", summary.headline))
        )
    } else {
        format!(
            "{}",
            skins
                .dim
                .apply_to(format!("  [result] {}", summary.headline))
        )
    };
    if let Some(body) = &summary.body {
        out.push('\n');
        for line in body.lines() {
            let styled = if is_error {
                format!("  {}", skins.error.apply_to(line))
            } else {
                render_content_line(line, skins)
            };
            out.push_str(&format!("{styled}\n"));
        }
    }
    out
}

pub fn render_error(code: &str, message: &str, skins: &Skins) -> String {
    format!(
        "{}",
        skins.error.apply_to(format!("[error] {code}: {message}"))
    )
}

pub fn render_turn_started(_turn_id: &str, skins: &Skins) -> String {
    format!("{}", skins.dim.apply_to("thinking..."))
}

pub fn render_turn_finished(
    reason: &TurnTerminalReason,
    message_id: Option<&MessageId>,
    skins: &Skins,
) -> String {
    let msg_id = message_id.map(|id| id.as_str()).unwrap_or("<none>");

    match reason {
        TurnTerminalReason::Done => {
            format!(
                "{}",
                skins
                    .success
                    .apply_to(format!("[done] message_id={msg_id}"))
            )
        }
        TurnTerminalReason::ToolUse => {
            format!("{}", skins.dim.apply_to("[tool_use]"))
        }
        TurnTerminalReason::Aborted => {
            format!(
                "{}",
                skins
                    .warning
                    .apply_to(format!("[aborted] message_id={msg_id}"))
            )
        }
        TurnTerminalReason::Error => {
            format!(
                "{}",
                skins
                    .error
                    .apply_to(format!("[error] message_id={msg_id}"))
            )
        }
    }
}

/// Style a diff line: green for additions, red for removals, dim for context.
fn render_diff_line(line: &str, skins: &Skins) -> String {
    if line.starts_with('+') {
        format!("{}", skins.diff_add.apply_to(line))
    } else if line.starts_with('-') {
        format!("{}", skins.diff_remove.apply_to(line))
    } else if line.starts_with("@@") {
        format!("{}", skins.diff_hunk.apply_to(line))
    } else {
        format!("{}", skins.dim.apply_to(line))
    }
}

/// Style a content line: CID-prefixed lines get dim line numbers, grep matches get highlighted.
fn render_content_line(line: &str, skins: &Skins) -> String {
    // CID format: "LINE#ID\tcontent" or grep format: "path:line:content"
    if let Some(tab_pos) = line.find('\t') {
        let tag = &line[..tab_pos];
        let content = &line[tab_pos + 1..];
        format!("  {}{}", skins.dim.apply_to(format!("{tag}\t")), content)
    } else if let Some(colon_pos) = line.find(':') {
        // grep-style: path:line:content
        let prefix = &line[..colon_pos + 1];
        let rest = &line[colon_pos + 1..];
        format!("  {}{}", skins.dim.apply_to(prefix), rest)
    } else {
        format!("  {}", skins.dim.apply_to(line))
    }
}

/// Emit ANSI escape sequences to move cursor up `n` lines (visual rows)
/// and clear from there to end of screen.
pub fn clear_last_n_lines(n: usize) -> String {
    if n == 0 {
        return "\r\x1b[0J".to_string();
    }
    format!("\x1b[{}F\x1b[0J", n)
}
