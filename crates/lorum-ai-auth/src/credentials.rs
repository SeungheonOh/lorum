use serde::{Deserialize, Serialize};

use crate::{unix_now, AuthError, REFRESH_SKEW_SECS};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    ApiKey,
    OAuth,
}

impl CredentialKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            CredentialKind::ApiKey => "api_key",
            CredentialKind::OAuth => "oauth",
        }
    }

    pub(crate) fn parse(value: &str) -> Result<Self, AuthError> {
        match value {
            "api_key" => Ok(CredentialKind::ApiKey),
            "oauth" => Ok(CredentialKind::OAuth),
            _ => Err(AuthError::InvalidCredential(format!(
                "unknown credential kind: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthCredential {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at_unix: Option<i64>,
    pub identity: Option<String>,
}

impl OAuthCredential {
    pub fn is_expired(&self, now_unix: i64) -> bool {
        match self.expires_at_unix {
            Some(expiry) => now_unix + REFRESH_SKEW_SECS >= expiry,
            None => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialData {
    ApiKey(ApiKeyCredential),
    OAuth(OAuthCredential),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRecord {
    pub credential_id: String,
    pub provider: String,
    pub kind: CredentialKind,
    pub disabled: bool,
    pub data: CredentialData,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialUsage {
    pub remaining_ratio: f64,
    pub reset_at_unix: Option<i64>,
    pub updated_at_unix: i64,
}

impl Default for CredentialUsage {
    fn default() -> Self {
        Self {
            remaining_ratio: 1.0,
            reset_at_unix: None,
            updated_at_unix: unix_now(),
        }
    }
}
