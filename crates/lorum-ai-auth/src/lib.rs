use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod bootstrap;
pub mod callback_listener;
mod credentials;
mod errors;
mod login;
mod oauth;
mod providers;
mod resolver;
mod store;

pub use bootstrap::{
    default_env_keys_for_provider, oauth_default_model_preset, oauth_provider_configuration_error,
    supported_oauth_providers, CurlOAuthHttpClient, OAuthProviderCatalog,
};
pub use credentials::{
    ApiKeyCredential, CredentialData, CredentialKind, CredentialRecord, CredentialUsage,
    OAuthCredential,
};
pub use errors::AuthError;
pub use oauth::{
    OAuthBeginContext, OAuthCallbackError, OAuthCallbackFlow, OAuthHttpClient, OAuthProvider,
    OAuthRefreshError, OAuthStart, OAuthToken,
};
pub use providers::OpenAiCodexOAuthProvider;
pub use resolver::{
    ApiKeyOptions, ApiKeyResolution, ApiKeySource, AuthResolver, EnvProvider,
    FallbackApiKeyResolver, StdEnvProvider,
};
pub use login::{
    oauth_await_callback, oauth_begin, oauth_complete, parse_manual_callback_input,
    persist_api_key, OAuthLoginRequest, OAuthLoginStart,
};
pub use store::{CredentialStore, SqliteCredentialStore};

pub(crate) const USAGE_KEY_PREFIX: &str = "usage";
pub(crate) const DEFAULT_TRANSIENT_BLOCK_SECS: i64 = 30;
pub(crate) const REFRESH_SKEW_SECS: i64 = 30;

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}
