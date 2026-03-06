use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use lorum_ai_auth::{
    default_env_keys_for_provider, ApiKeyOptions, CredentialStore, CurlOAuthHttpClient,
    OAuthProviderCatalog, SqliteCredentialStore,
};
use lorum_ai_connectors::build_curl_provider_catalog;
use lorum_ai_contract::{ModelRef, ProviderAdapter};
use lorum_domain::SessionId;
use lorum_runtime::{
    agents::builtin_agents,
    subagent::{SubagentExecutor, SubagentHandler, SubmitResultHandler},
    ChatOnlyRuntime, RuntimeAuthResolver, RuntimeConfig, RuntimeModelResolver,
    RuntimeProviderRegistry, ToolCallDisplay, ToolDispatcher,
};
use lorum_session::{InMemorySessionStore, SessionStore};

pub const DEFAULT_SESSION_ID: &str = "default";

pub struct AppDeps {
    pub runtime: ChatOnlyRuntime,
    pub session_store: Arc<dyn SessionStore>,
    pub credential_store: Arc<dyn CredentialStore>,
    pub oauth_catalog: OAuthProviderCatalog,
    pub model_presets: HashMap<String, ModelRef>,
    pub default_model: ModelRef,
    pub tool_display: Arc<dyn ToolCallDisplay>,
}

struct CliAuthResolver {
    resolver: Arc<lorum_ai_auth::AuthResolver>,
}

#[async_trait]
impl RuntimeAuthResolver for CliAuthResolver {
    async fn get_api_key(
        &self,
        provider: &str,
        session_id: &SessionId,
    ) -> Result<Option<String>, String> {
        let resolution = self
            .resolver
            .get_api_key(
                provider,
                session_id.as_str(),
                ApiKeyOptions {
                    runtime_override: None,
                    env_keys: default_env_keys_for_provider(provider),
                    allow_oauth: true,
                    now_unix: None,
                },
            )
            .await
            .map_err(|err| err.to_string())?;

        Ok(resolution.map(|resolved| resolved.api_key))
    }
}

struct StaticModelResolver {
    default_model: ModelRef,
}

#[async_trait]
impl RuntimeModelResolver for StaticModelResolver {
    async fn resolve_model(
        &self,
        _session_id: &SessionId,
        override_model: Option<&ModelRef>,
    ) -> Result<ModelRef, String> {
        Ok(override_model
            .cloned()
            .unwrap_or_else(|| self.default_model.clone()))
    }
}

struct StaticProviderRegistry {
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl RuntimeProviderRegistry for StaticProviderRegistry {
    fn get_provider(&self, provider_id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(provider_id).cloned()
    }
}

pub fn build_app_deps() -> Result<AppDeps, String> {
    let auth_db_path = resolve_auth_db_path();
    let credential_store: Arc<dyn CredentialStore> =
        Arc::new(SqliteCredentialStore::open(&auth_db_path).map_err(|err| {
            format!(
                "open auth store failed at {}: {err}",
                auth_db_path.display()
            )
        })?);

    let oauth_catalog = OAuthProviderCatalog::from_env(Arc::new(CurlOAuthHttpClient));

    let mut auth_resolver = lorum_ai_auth::AuthResolver::new(Arc::clone(&credential_store));
    oauth_catalog.register_into_resolver(&mut auth_resolver);

    let auth_resolver: Arc<dyn RuntimeAuthResolver> = Arc::new(CliAuthResolver {
        resolver: Arc::new(auth_resolver),
    });

    let provider_catalog = build_curl_provider_catalog();
    let model_presets = provider_catalog.model_presets().clone();
    let default_model = provider_catalog
        .default_model()
        .ok_or_else(|| "provider catalog has no default model".to_string())?;

    let model_resolver: Arc<dyn RuntimeModelResolver> = Arc::new(StaticModelResolver {
        default_model: default_model.clone(),
    });

    let provider_registry: Arc<dyn RuntimeProviderRegistry> = Arc::new(StaticProviderRegistry {
        providers: provider_catalog.into_providers(),
    });
    let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());

    let cwd = env::current_dir().map_err(|e| format!("failed to get current directory: {e}"))?;
    let tool_registry = Arc::new(lorum_tools::ToolRegistry::new(
        cwd,
        std::time::Duration::from_secs(120),
    ));
    let tool_executor: Arc<dyn lorum_runtime::ToolExecutor> = Arc::clone(&tool_registry) as _;
    let tool_display: Arc<dyn ToolCallDisplay> = tool_registry;

    let config = RuntimeConfig {
        max_tool_turns: 25,
        timeout_ms: 120_000,
        max_output_bytes: lorum_runtime::subagent::DEFAULT_MAX_OUTPUT_BYTES,
        max_output_lines: lorum_runtime::subagent::DEFAULT_MAX_OUTPUT_LINES,
    };

    let dispatcher = Arc::new(ToolDispatcher::new(tool_executor));

    let subagent_executor = Arc::new(SubagentExecutor::new(
        Arc::clone(&auth_resolver),
        Arc::clone(&model_resolver),
        Arc::clone(&provider_registry),
        Arc::clone(&session_store),
        config,
    ));

    let max_recursion_depth = 2;
    dispatcher.register(Arc::new(SubagentHandler::new(
        subagent_executor,
        builtin_agents(),
        max_recursion_depth,
        Arc::clone(&dispatcher),
        lorum_tools::task_definition(),
    )));
    dispatcher.register(Arc::new(SubmitResultHandler));

    let runtime = ChatOnlyRuntime::new(
        config,
        auth_resolver,
        model_resolver,
        provider_registry,
        Arc::clone(&session_store),
        Some(dispatcher),
    );

    Ok(AppDeps {
        runtime,
        session_store,
        credential_store,
        oauth_catalog,
        model_presets,
        default_model,
        tool_display,
    })
}

pub fn resolve_auth_db_path() -> PathBuf {
    if let Some(value) = env::var("OMP_AUTH_DB")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        return PathBuf::from(value);
    }

    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".oh-my-pi").join("auth.db")
}

pub fn resolve_history_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".oh-my-pi").join("history.txt")
}

pub fn try_open_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(url)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(url)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", url])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        false
    }
}
