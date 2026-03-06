use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::sync::Arc;

use serde_json::Value;

use crate::{AuthError, AuthResolver, OAuthHttpClient, OAuthProvider, OpenAiCodexOAuthProvider};

const DEFAULT_OPENAI_REDIRECT_URI: &str = "http://127.0.0.1:1455/callback";

pub struct CurlOAuthHttpClient;

#[async_trait::async_trait]
impl OAuthHttpClient for CurlOAuthHttpClient {
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
        headers: &[(String, String)],
    ) -> Result<Value, AuthError> {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-X")
            .arg("POST")
            .arg(url)
            .arg("-H")
            .arg("accept: application/json")
            .arg("-H")
            .arg("content-type: application/x-www-form-urlencoded");

        for (key, value) in headers {
            cmd.arg("-H").arg(format!("{key}: {value}"));
        }

        for (key, value) in form {
            cmd.arg("--data-urlencode").arg(format!("{key}={value}"));
        }

        let output = cmd.output().map_err(|err| {
            AuthError::InvalidCredential(format!("failed to execute curl: {err}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AuthError::InvalidCredential(format!(
                "oauth request failed with status {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        let body = String::from_utf8(output.stdout)
            .map_err(|err| AuthError::Serialization(format!("oauth response not utf8: {err}")))?;

        serde_json::from_str(&body)
            .map_err(|err| AuthError::Serialization(format!("oauth response parse failed: {err}")))
    }
}

pub struct OAuthProviderCatalog {
    providers: HashMap<String, Arc<dyn OAuthProvider>>,
    redirect_uris: HashMap<String, String>,
}

impl OAuthProviderCatalog {
    pub fn from_env(client: Arc<dyn OAuthHttpClient>) -> Self {
        let mut providers: HashMap<String, Arc<dyn OAuthProvider>> = HashMap::new();
        let mut redirect_uris: HashMap<String, String> = HashMap::new();

        let openai_client_id = env_first_non_empty(&[
            "OPENAI_OAUTH_CLIENT_ID",
            "OMP_OPENAI_CLIENT_ID",
            "OMP_SMOKE_OPENAI_CLIENT_ID",
        ]);
        let openai_redirect_uri = env_first_non_empty(&[
            "OPENAI_OAUTH_REDIRECT_URI",
            "OMP_OPENAI_REDIRECT_URI",
            "OMP_SMOKE_OPENAI_REDIRECT_URI",
        ])
        .unwrap_or_else(|| DEFAULT_OPENAI_REDIRECT_URI.to_string());

        if let Some(client_id) = openai_client_id {
            let provider: Arc<dyn OAuthProvider> = Arc::new(OpenAiCodexOAuthProvider::new(
                Arc::clone(&client),
                client_id,
                openai_redirect_uri.clone(),
            ));
            providers.insert("openai".to_string(), provider);
            redirect_uris.insert("openai".to_string(), openai_redirect_uri);
        }

        Self {
            providers,
            redirect_uris,
        }
    }

    pub fn register_into_resolver(&self, resolver: &mut AuthResolver) {
        for provider in self.providers.values() {
            resolver.register_oauth_provider(Arc::clone(provider));
        }
    }

    pub fn provider(&self, provider_id: &str) -> Option<Arc<dyn OAuthProvider>> {
        self.providers.get(provider_id).cloned()
    }

    pub fn redirect_uri(&self, provider_id: &str) -> Option<&str> {
        self.redirect_uris.get(provider_id).map(String::as_str)
    }

    pub fn provider_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.providers.keys().cloned().collect();
        ids.sort();
        ids
    }
}

pub fn default_env_keys_for_provider(provider: &str) -> Vec<String> {
    match provider {
        "openai" => vec![
            "OPENAI_API_KEY".to_string(),
            "OMP_OPENAI_API_KEY".to_string(),
        ],
        "anthropic" => vec![
            "ANTHROPIC_API_KEY".to_string(),
            "OMP_ANTHROPIC_API_KEY".to_string(),
        ],
        _ => Vec::new(),
    }
}

pub fn supported_oauth_providers() -> Vec<String> {
    vec!["openai".to_string()]
}

pub fn oauth_provider_configuration_error(provider: &str) -> Option<String> {
    match provider {
        "openai" => Some(
            "OPENAI_OAUTH_CLIENT_ID (or OMP_OPENAI_CLIENT_ID) is required for /login openai"
                .to_string(),
        ),
        _ => None,
    }
}

pub fn oauth_default_model_preset(provider: &str) -> Option<String> {
    match provider {
        "openai" => Some("codex".to_string()),
        _ => None,
    }
}

fn env_first_non_empty(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}
