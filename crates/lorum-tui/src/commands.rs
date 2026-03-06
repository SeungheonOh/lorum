use std::collections::HashMap;
use std::io::{self, Write};
use std::str::FromStr;

use tokio::runtime::Runtime;

use lorum_ai_auth::{
    callback_listener::CallbackResult, oauth_await_callback, oauth_begin, oauth_complete,
    oauth_default_model_preset, parse_manual_callback_input, persist_api_key,
    supported_oauth_providers, OAuthLoginRequest,
};
use lorum_ai_contract::{ApiKind, ModelRef};
use lorum_domain::{SessionId, TurnId};
use lorum_runtime::{ModelSelectRequest, RuntimeController, UserInputCommand};
use lorum_ui_print::{print_exit_code, render_text};

use crate::deps::{self, AppDeps};

pub const MODEL_COMMAND_USAGE: &str = "/model <preset> | /model <provider> <api-kind> <model>";

pub enum DispatchResult {
    Continue,
    Quit,
}

pub fn dispatch(
    input: &str,
    rt: &Runtime,
    deps: &AppDeps,
    session_id: &SessionId,
    current_model: &mut ModelRef,
    turn_counter: &mut u64,
) -> Result<DispatchResult, String> {
    if input == "/quit" {
        return Ok(DispatchResult::Quit);
    }

    if input == "/help" {
        let mut preset_names: Vec<String> = deps.model_presets.keys().cloned().collect();
        preset_names.sort();
        let mut oauth_providers = supported_oauth_providers();
        oauth_providers.sort();
        println!("/use <preset> ({})", preset_names.join(", "));
        println!("{MODEL_COMMAND_USAGE}");
        println!("/apikey <provider> <api_key>");
        println!("/login <provider> ({})", oauth_providers.join(", "));
        println!("/history");
        println!("/status");
        println!("/quit");
        return Ok(DispatchResult::Continue);
    }

    if input == "/status" {
        println!(
            "session={} provider={} api={} model={}",
            session_id.as_str(),
            current_model.provider,
            current_model.api,
            current_model.model
        );
        return Ok(DispatchResult::Continue);
    }

    if input == "/history" {
        match deps.session_store.replay(session_id) {
            Ok(events) => {
                if events.is_empty() {
                    println!("(no events)");
                } else {
                    let summary = render_text(&events);
                    let exit_code = print_exit_code(&events);
                    println!("{summary}");
                    println!("history_exit_code={exit_code}");
                }
            }
            Err(err) => eprintln!("history failed: {err}"),
        }
        return Ok(DispatchResult::Continue);
    }

    if let Some(rest) = input.strip_prefix("/use ") {
        let preset = rest.trim();
        let Some(model) = deps.model_presets.get(preset).cloned() else {
            eprintln!("unknown preset '{preset}'");
            return Ok(DispatchResult::Continue);
        };

        if let Err(err) = rt.block_on(deps.runtime.set_model(ModelSelectRequest {
            session_id: session_id.clone(),
            model: model.clone(),
        })) {
            eprintln!("set model failed: {err}");
            return Ok(DispatchResult::Continue);
        }

        *current_model = model;
        println!(
            "active model -> provider={} api={} model={}",
            current_model.provider, current_model.api, current_model.model
        );
        return Ok(DispatchResult::Continue);
    }

    if let Some(rest) = input.strip_prefix("/model ") {
        let model = match parse_model_selection(rest, &deps.model_presets) {
            Ok(model) => model,
            Err(err) => {
                eprintln!("{err}");
                return Ok(DispatchResult::Continue);
            }
        };

        if let Err(err) = rt.block_on(deps.runtime.set_model(ModelSelectRequest {
            session_id: session_id.clone(),
            model: model.clone(),
        })) {
            eprintln!("set model failed: {err}");
            return Ok(DispatchResult::Continue);
        }

        *current_model = model;
        println!(
            "active model -> provider={} api={} model={}",
            current_model.provider, current_model.api, current_model.model
        );
        return Ok(DispatchResult::Continue);
    }

    if let Some(rest) = input.strip_prefix("/apikey ") {
        let mut parts = rest.splitn(2, ' ');
        let Some(provider) = parts.next() else {
            eprintln!("usage: /apikey <provider> <api_key>");
            return Ok(DispatchResult::Continue);
        };
        let Some(api_key) = parts.next() else {
            eprintln!("usage: /apikey <provider> <api_key>");
            return Ok(DispatchResult::Continue);
        };

        if let Err(err) = rt.block_on(persist_api_key(
            deps.credential_store.as_ref(),
            provider,
            api_key.trim(),
        )) {
            eprintln!("persist api key failed: {err}");
            return Ok(DispatchResult::Continue);
        }

        println!("stored api key for provider '{provider}'");
        return Ok(DispatchResult::Continue);
    }

    if let Some(provider_id) = input.strip_prefix("/login ") {
        let provider_id = provider_id.trim();
        if provider_id.is_empty() {
            eprintln!("usage: /login <provider>");
            return Ok(DispatchResult::Continue);
        }

        match oauth_login(rt, provider_id, deps) {
            Ok(()) => {
                println!("oauth login succeeded for '{provider_id}'");
                if let Some(default_preset) = oauth_default_model_preset(provider_id) {
                    if let Some(model) = deps.model_presets.get(&default_preset).cloned() {
                        if let Err(err) = rt.block_on(deps.runtime.set_model(ModelSelectRequest {
                            session_id: session_id.clone(),
                            model: model.clone(),
                        })) {
                            eprintln!(
                                "oauth login succeeded, but setting model preset '{}' failed: {err}",
                                default_preset
                            );
                        } else {
                            *current_model = model;
                            println!(
                                "active model -> provider={} api={} model={}",
                                current_model.provider, current_model.api, current_model.model
                            );
                        }
                    }
                }
            }
            Err(err) => eprintln!("oauth login failed: {err}"),
        }
        return Ok(DispatchResult::Continue);
    }

    // Default: submit as user input
    let cmd = UserInputCommand {
        session_id: session_id.clone(),
        turn_id: TurnId::from(format!("turn-{}", *turn_counter)),
        prompt: input.to_string(),
        system_prompt: None,
    };
    *turn_counter += 1;

    if let Err(err) = rt.block_on(deps.runtime.submit_user_input(cmd)) {
        eprintln!("submit failed: {err}");
    }

    Ok(DispatchResult::Continue)
}

pub fn parse_model_selection(
    input: &str,
    model_presets: &HashMap<String, ModelRef>,
) -> Result<ModelRef, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!("usage: {MODEL_COMMAND_USAGE}"));
    }

    let mut parts = trimmed.split_whitespace();
    let Some(first) = parts.next() else {
        return Err(format!("usage: {MODEL_COMMAND_USAGE}"));
    };

    let Some(api_raw) = parts.next() else {
        return model_presets
            .get(first)
            .cloned()
            .ok_or_else(|| format!("unknown preset '{first}'"));
    };

    let model_name = parts.collect::<Vec<_>>().join(" ");
    if model_name.is_empty() {
        return Err(format!("usage: {MODEL_COMMAND_USAGE}"));
    }

    let api_normalized = api_raw.replace('_', "-");
    let api =
        ApiKind::from_str(&api_normalized).map_err(|_| format!("invalid api-kind: {api_raw}"))?;
    Ok(ModelRef {
        provider: first.to_string(),
        api,
        model: model_name,
    })
}

fn oauth_login(rt: &Runtime, provider_id: &str, deps: &AppDeps) -> Result<(), String> {
    let req = OAuthLoginRequest::new(
        provider_id,
        &deps.oauth_catalog,
        deps.credential_store.as_ref(),
    );

    let start = rt
        .block_on(oauth_begin(&req))
        .map_err(|err| err.to_string())?;

    println!(
        "open this url and complete auth for '{}':\n{}",
        provider_id, start.authorization_url
    );
    if !deps::try_open_browser(&start.authorization_url) {
        println!("browser auto-open unavailable; open URL manually");
    }

    if let Some((bind_host, port)) =
        lorum_ai_auth::callback_listener::LocalCallbackListener::bind_host_port(&start.redirect_uri)
    {
        println!(
            "waiting for oauth callback on http://{}:{} for up to {}s",
            bind_host,
            port,
            req.callback_timeout.as_secs()
        );
    }

    let code = match oauth_await_callback(&start, req.callback_timeout) {
        CallbackResult::Code(code) => code,
        CallbackResult::Error(err) => {
            if err.contains("oauth authorization failed:") {
                return Err(err);
            }
            eprintln!("local callback capture failed: {err}");
            let input = prompt_line("paste callback URL or auth code: ")?;
            parse_manual_callback_input(&input, &start.state, req.callback_timeout)?
        }
        _ => {
            let input = prompt_line("paste callback URL or auth code: ")?;
            parse_manual_callback_input(&input, &start.state, req.callback_timeout)?
        }
    };

    rt.block_on(oauth_complete(&req, &start, &code))
        .map_err(|err| err.to_string())
}

pub fn prompt_line(prompt: &str) -> Result<String, String> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|err| format!("flush prompt failed: {err}"))?;

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|err| format!("read input failed: {err}"))?;
    Ok(line)
}
