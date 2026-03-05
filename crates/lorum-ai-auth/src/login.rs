use std::time::Duration;

use crate::{
    callback_listener::{CallbackResult, LocalCallbackListener},
    unix_now, ApiKeyCredential, AuthError, CredentialData, CredentialKind, CredentialRecord,
    CredentialStore, OAuthBeginContext, OAuthCallbackFlow, OAuthProviderCatalog,
    oauth_provider_configuration_error,
};

const DEFAULT_CALLBACK_TIMEOUT_SECS: u64 = 180;

pub struct OAuthLoginRequest<'a> {
    pub provider_id: &'a str,
    pub catalog: &'a OAuthProviderCatalog,
    pub credential_store: &'a dyn CredentialStore,
    pub callback_timeout: Duration,
}

impl<'a> OAuthLoginRequest<'a> {
    pub fn new(
        provider_id: &'a str,
        catalog: &'a OAuthProviderCatalog,
        credential_store: &'a dyn CredentialStore,
    ) -> Self {
        Self {
            provider_id,
            catalog,
            credential_store,
            callback_timeout: Duration::from_secs(DEFAULT_CALLBACK_TIMEOUT_SECS),
        }
    }
}

pub struct OAuthLoginStart {
    pub authorization_url: String,
    pub state: String,
    pub code_verifier: Option<String>,
    pub redirect_uri: String,
}

pub async fn oauth_begin(req: &OAuthLoginRequest<'_>) -> Result<OAuthLoginStart, AuthError> {
    let provider = req.catalog.provider(req.provider_id).ok_or_else(|| {
        let msg = oauth_provider_configuration_error(req.provider_id)
            .unwrap_or_else(|| format!("oauth provider not configured: {}", req.provider_id));
        AuthError::MissingOAuthProvider(msg)
    })?;

    let redirect_uri = req
        .catalog
        .redirect_uri(req.provider_id)
        .ok_or_else(|| {
            AuthError::InvalidCredential(format!(
                "missing redirect URI for oauth provider: {}",
                req.provider_id
            ))
        })?
        .to_string();

    let start = provider
        .begin_flow(OAuthBeginContext {
            redirect_uri: redirect_uri.clone(),
            scopes: Vec::new(),
            state: None,
        })
        .await?;

    Ok(OAuthLoginStart {
        authorization_url: start.authorization_url,
        state: start.state,
        code_verifier: start.code_verifier,
        redirect_uri,
    })
}

pub fn oauth_await_callback(start: &OAuthLoginStart, timeout: Duration) -> CallbackResult {
    let listener = LocalCallbackListener::new(timeout);
    listener.wait_for_code(&start.redirect_uri, &start.state)
}

pub fn parse_manual_callback_input(
    input: &str,
    expected_state: &str,
    timeout: Duration,
) -> Result<String, String> {
    let callback_flow = OAuthCallbackFlow::new(0, timeout.as_secs());
    callback_flow
        .parse_callback_or_manual_input(input.trim(), expected_state)
        .map_err(|err| err.to_string())
}

pub async fn oauth_complete(
    req: &OAuthLoginRequest<'_>,
    start: &OAuthLoginStart,
    code: &str,
) -> Result<(), AuthError> {
    let provider = req.catalog.provider(req.provider_id).ok_or_else(|| {
        AuthError::MissingOAuthProvider(format!(
            "oauth provider not configured: {}",
            req.provider_id
        ))
    })?;

    let token = provider
        .exchange_code(code, start.code_verifier.as_deref())
        .await?;

    let now = unix_now();
    let record = CredentialRecord {
        credential_id: format!("oauth-{}-{now}", req.provider_id),
        provider: req.provider_id.to_string(),
        kind: CredentialKind::OAuth,
        disabled: false,
        data: CredentialData::OAuth(token.credential),
        created_at_unix: now,
        updated_at_unix: now,
    };

    req.credential_store.upsert(&record).await?;
    let _ = req
        .credential_store
        .disable(&format!("manual-api-key-{}", req.provider_id))
        .await;

    Ok(())
}

pub async fn persist_api_key(
    store: &dyn CredentialStore,
    provider: &str,
    api_key: &str,
) -> Result<(), AuthError> {
    if api_key.trim().is_empty() {
        return Err(AuthError::InvalidCredential(
            "api key must not be empty".to_string(),
        ));
    }

    let now = unix_now();
    let record = CredentialRecord {
        credential_id: format!("manual-api-key-{provider}"),
        provider: provider.to_string(),
        kind: CredentialKind::ApiKey,
        disabled: false,
        data: CredentialData::ApiKey(ApiKeyCredential {
            api_key: api_key.to_string(),
        }),
        created_at_unix: now,
        updated_at_unix: now,
    };

    store.upsert(&record).await
}
