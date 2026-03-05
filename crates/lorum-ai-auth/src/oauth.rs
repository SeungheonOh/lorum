use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::{AuthError, OAuthCredential};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthBeginContext {
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStart {
    pub authorization_url: String,
    pub state: String,
    pub code_verifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthToken {
    pub credential: OAuthCredential,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum OAuthRefreshError {
    #[error("oauth token invalid grant")]
    InvalidGrant,
    #[error("oauth token revoked")]
    Revoked,
    #[error("oauth unauthorized")]
    Unauthorized,
    #[error("oauth forbidden")]
    Forbidden,
    #[error("oauth transient error: {0}")]
    Transient(String),
    #[error("oauth permanent error: {0}")]
    Permanent(String),
}

impl OAuthRefreshError {
    pub(crate) fn is_definitive(&self) -> bool {
        matches!(
            self,
            OAuthRefreshError::InvalidGrant
                | OAuthRefreshError::Revoked
                | OAuthRefreshError::Unauthorized
                | OAuthRefreshError::Forbidden
                | OAuthRefreshError::Permanent(_)
        )
    }
}

#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &str;

    async fn begin_flow(&self, ctx: OAuthBeginContext) -> Result<OAuthStart, AuthError>;

    async fn exchange_code(
        &self,
        code: &str,
        verifier: Option<&str>,
    ) -> Result<OAuthToken, AuthError>;

    async fn refresh(
        &self,
        credential: &OAuthCredential,
    ) -> Result<OAuthCredential, OAuthRefreshError>;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OAuthCallbackError {
    #[error("invalid callback url")]
    InvalidUrl,
    #[error("oauth authorization failed: {error}: {description}")]
    AuthorizationFailed { error: String, description: String },
    #[error("callback missing authorization code")]
    MissingCode,
    #[error("callback missing state")]
    MissingState,
    #[error("callback state mismatch")]
    StateMismatch,
    #[error("manual input missing code")]
    MissingManualCode,
}

pub struct OAuthCallbackFlow {
    pub preferred_port: u16,
    pub timeout_seconds: u64,
}

impl OAuthCallbackFlow {
    pub fn new(preferred_port: u16, timeout_seconds: u64) -> Self {
        Self {
            preferred_port,
            timeout_seconds,
        }
    }

    pub fn generate_state(&self) -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
        format!("st-{:x}-{:x}", crate::unix_now().max(0), counter)
    }

    pub fn choose_callback_port(&self) -> Result<u16, AuthError> {
        if self.preferred_port != 0 {
            if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", self.preferred_port)) {
                let port = listener
                    .local_addr()
                    .map_err(|err| {
                        AuthError::Database(format!("read preferred port failed: {err}"))
                    })?
                    .port();
                return Ok(port);
            }
        }

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|err| AuthError::Database(format!("bind callback port failed: {err}")))?;
        let port = listener
            .local_addr()
            .map_err(|err| AuthError::Database(format!("read callback port failed: {err}")))?
            .port();
        Ok(port)
    }

    pub fn parse_callback_url(
        &self,
        callback_url: &str,
        expected_state: &str,
    ) -> Result<String, OAuthCallbackError> {
        let parsed = Url::parse(callback_url).map_err(|_| OAuthCallbackError::InvalidUrl)?;
        let query: HashMap<String, String> = parsed.query_pairs().into_owned().collect();

        if let Some(error) = query.get("error").cloned() {
            let description = query
                .get("error_description")
                .cloned()
                .unwrap_or_else(|| "authorization failed".to_string());
            return Err(OAuthCallbackError::AuthorizationFailed { error, description });
        }
        let code = query
            .get("code")
            .cloned()
            .ok_or(OAuthCallbackError::MissingCode)?;
        let state = query
            .get("state")
            .cloned()
            .ok_or(OAuthCallbackError::MissingState)?;

        if state != expected_state {
            return Err(OAuthCallbackError::StateMismatch);
        }

        Ok(code)
    }

    pub fn parse_callback_or_manual_input(
        &self,
        input: &str,
        expected_state: &str,
    ) -> Result<String, OAuthCallbackError> {
        let trimmed = input.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return self.parse_callback_url(trimmed, expected_state);
        }
        self.parse_manual_code(trimmed)
    }

    pub fn parse_manual_code(&self, input: &str) -> Result<String, OAuthCallbackError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(OAuthCallbackError::MissingManualCode);
        }

        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            return Err(OAuthCallbackError::InvalidUrl);
        }

        Ok(trimmed.to_string())
    }
}

#[async_trait]
pub trait OAuthHttpClient: Send + Sync {
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
        headers: &[(String, String)],
    ) -> Result<Value, AuthError>;
}
