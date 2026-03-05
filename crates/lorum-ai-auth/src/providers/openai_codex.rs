use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    AuthError, OAuthBeginContext, OAuthCallbackFlow, OAuthCredential, OAuthHttpClient,
    OAuthProvider, OAuthRefreshError, OAuthStart, OAuthToken,
};

pub struct OpenAiCodexOAuthProvider {
    client: Arc<dyn OAuthHttpClient>,
    client_id: String,
    redirect_uri: String,
    auth_url: String,
    token_url: String,
    default_scopes: Vec<String>,
}

impl OpenAiCodexOAuthProvider {
    pub fn new(
        client: Arc<dyn OAuthHttpClient>,
        client_id: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            client,
            client_id: client_id.into(),
            redirect_uri: redirect_uri.into(),
            auth_url: "https://auth.openai.com/oauth/authorize".to_string(),
            token_url: "https://auth.openai.com/oauth/token".to_string(),
            default_scopes: vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
                "offline_access".to_string(),
            ],
        }
    }

    fn generate_code_verifier(state: &str) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        let mut hasher = Sha256::new();
        hasher.update(state.as_bytes());
        hasher.update(counter.to_le_bytes());
        hasher.update(crate::unix_now().to_le_bytes());
        let digest = hasher.finalize();
        let mut verifier = URL_SAFE_NO_PAD.encode(digest);
        while verifier.len() < 43 {
            verifier.push('A');
        }
        verifier
    }

    fn pkce_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        URL_SAFE_NO_PAD.encode(digest)
    }

    fn resolve_scopes(&self, requested: &[String]) -> Vec<String> {
        let mut scopes = if requested.is_empty() {
            self.default_scopes.clone()
        } else {
            requested.to_vec()
        };
        for required in ["offline_access", "openid", "profile", "email"] {
            if !scopes.iter().any(|scope| scope == required) {
                scopes.push(required.to_string());
            }
        }
        scopes
    }

    fn parse_access_token_response(&self, response: &Value) -> Result<OAuthToken, AuthError> {
        if let Some(code) = response.get("error").and_then(Value::as_str) {
            let description = response
                .get("error_description")
                .and_then(Value::as_str)
                .unwrap_or("oauth exchange failed");
            return Err(AuthError::InvalidCredential(format!(
                "oauth exchange error {code}: {description}"
            )));
        }

        let access_token = response
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AuthError::InvalidCredential("oauth exchange missing access_token".to_string())
            })?
            .to_string();

        let refresh_token = response
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(ToString::to_string);

        let expires_in = response.get("expires_in").and_then(Value::as_i64);
        let expires_at_unix = expires_in.map(|seconds| crate::unix_now() + seconds.max(0));

        Ok(OAuthToken {
            credential: OAuthCredential {
                access_token,
                refresh_token,
                expires_at_unix,
                identity: None,
            },
        })
    }

    fn map_refresh_error(response: &Value) -> Option<OAuthRefreshError> {
        let code = response.get("error")?.as_str()?;
        let description = response
            .get("error_description")
            .and_then(Value::as_str)
            .unwrap_or("oauth refresh error");

        Some(match code {
            "invalid_grant" => OAuthRefreshError::InvalidGrant,
            "revoked" => OAuthRefreshError::Revoked,
            "unauthorized" | "invalid_client" => OAuthRefreshError::Unauthorized,
            "forbidden" => OAuthRefreshError::Forbidden,
            "temporarily_unavailable" | "timeout" => {
                OAuthRefreshError::Transient(description.to_string())
            }
            _ => OAuthRefreshError::Permanent(format!("{code}: {description}")),
        })
    }

    fn parse_refresh_response(
        &self,
        response: &Value,
        prior_credential: &OAuthCredential,
    ) -> Result<OAuthCredential, OAuthRefreshError> {
        if let Some(mapped) = Self::map_refresh_error(response) {
            return Err(mapped);
        }

        let access_token = response
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                OAuthRefreshError::Permanent("refresh missing access_token".to_string())
            })?
            .to_string();

        let refresh_token = response
            .get("refresh_token")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .or_else(|| prior_credential.refresh_token.clone());

        let expires_in = response.get("expires_in").and_then(Value::as_i64);
        let expires_at_unix = expires_in.map(|seconds| crate::unix_now() + seconds.max(0));

        Ok(OAuthCredential {
            access_token,
            refresh_token,
            expires_at_unix,
            identity: prior_credential.identity.clone(),
        })
    }
}

#[async_trait]
impl OAuthProvider for OpenAiCodexOAuthProvider {
    fn id(&self) -> &str {
        "openai"
    }

    async fn begin_flow(&self, ctx: OAuthBeginContext) -> Result<OAuthStart, AuthError> {
        let state = ctx.state.unwrap_or_else(|| {
            let flow = OAuthCallbackFlow::new(3000, 300);
            flow.generate_state()
        });
        let verifier = Self::generate_code_verifier(&state);
        let challenge = Self::pkce_challenge(&verifier);
        let scopes = self.resolve_scopes(&ctx.scopes).join(" ");
        let redirect = if ctx.redirect_uri.trim().is_empty() {
            self.redirect_uri.clone()
        } else {
            ctx.redirect_uri
        };

        let mut url = Url::parse(&self.auth_url)
            .map_err(|err| AuthError::InvalidCredential(format!("invalid auth url: {err}")))?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect)
            .append_pair("scope", &scopes)
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "opencode");

        Ok(OAuthStart {
            authorization_url: url.to_string(),
            state,
            code_verifier: Some(verifier),
        })
    }

    async fn exchange_code(
        &self,
        code: &str,
        verifier: Option<&str>,
    ) -> Result<OAuthToken, AuthError> {
        let Some(verifier) = verifier else {
            return Err(AuthError::InvalidCredential(
                "oauth code verifier is required".to_string(),
            ));
        };

        let response = self
            .client
            .post_form(
                &self.token_url,
                &[
                    ("grant_type".to_string(), "authorization_code".to_string()),
                    ("code".to_string(), code.to_string()),
                    ("client_id".to_string(), self.client_id.clone()),
                    ("redirect_uri".to_string(), self.redirect_uri.clone()),
                    ("code_verifier".to_string(), verifier.to_string()),
                ],
                &[("accept".to_string(), "application/json".to_string())],
            )
            .await?;

        self.parse_access_token_response(&response)
    }

    async fn refresh(
        &self,
        credential: &OAuthCredential,
    ) -> Result<OAuthCredential, OAuthRefreshError> {
        let refresh_token = credential
            .refresh_token
            .clone()
            .ok_or_else(|| OAuthRefreshError::Permanent("missing refresh token".to_string()))?;

        let response = self
            .client
            .post_form(
                &self.token_url,
                &[
                    ("grant_type".to_string(), "refresh_token".to_string()),
                    ("refresh_token".to_string(), refresh_token),
                    ("client_id".to_string(), self.client_id.clone()),
                ],
                &[("accept".to_string(), "application/json".to_string())],
            )
            .await
            .map_err(|err| OAuthRefreshError::Transient(err.to_string()))?;

        self.parse_refresh_response(&response, credential)
    }
}
