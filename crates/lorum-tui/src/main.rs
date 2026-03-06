mod input;
mod render;
mod subscriber;

use std::sync::Arc;

use lorum_domain::SessionId;
use lorum_runtime::{ModelSelectRequest, RuntimeController};
use lorum_tui::{commands, deps};
use tokio::runtime::Runtime;

use crate::input::InputSignal;

fn main() {
    if let Err(e) = run() {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let rt = Runtime::new().map_err(|err| format!("failed to create async runtime: {err}"))?;
    let deps = deps::build_app_deps()?;
    let current_session_id = SessionId::from(deps::DEFAULT_SESSION_ID);
    let mut current_model = deps.default_model.clone();
    let mut turn_counter = 1_u64;

    let skins = render::Skins::new();
    let subscriber = Arc::new(subscriber::CliSubscriber::new(skins, Arc::clone(&deps.tool_display)));
    rt.block_on(deps.runtime.subscribe(subscriber))
        .map_err(|err| format!("subscribe failed: {err}"))?;

    rt.block_on(deps.runtime.set_model(ModelSelectRequest {
        session_id: current_session_id.clone(),
        model: current_model.clone(),
    }))
    .map_err(|err| format!("initial model set failed: {err}"))?;

    let history_path = deps::resolve_history_path();
    let mut input_reader =
        input::InputReader::new(&history_path, &current_model.model)?;

    println!("servus - AI coding agent");
    println!(
        "model: {} | session: {}",
        current_model.model,
        current_session_id.as_str()
    );
    println!(
        "commands: /help, /quit, /history, /status, /use, /model, /apikey, /login"
    );

    loop {
        match input_reader.read_line()? {
            InputSignal::Eof => break,
            InputSignal::Interrupted => {
                println!("use /quit to exit");
                continue;
            }
            InputSignal::Line(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match commands::dispatch(
                    trimmed,
                    &rt,
                    &deps,
                    &current_session_id,
                    &mut current_model,
                    &mut turn_counter,
                )? {
                    commands::DispatchResult::Quit => break,
                    commands::DispatchResult::Continue => {}
                }
                input_reader.update_prompt(&current_model.model);
            }
        }
    }

    println!("bye");
    Ok(())
}
