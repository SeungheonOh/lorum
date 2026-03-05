use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    unix_now, AuthError, CredentialData, CredentialKind, CredentialRecord, CredentialStore,
    CredentialUsage, OAuthProvider,
};

#[derive(Debug, Clone)]
pub struct ApiKeyOptions {
    pub runtime_override: Option<String>,
    pub env_keys: Vec<String>,
    pub allow_oauth: bool,
    pub now_unix: Option<i64>,
}

impl Default for ApiKeyOptions {
    fn default() -> Self {
        Self {
            runtime_override: None,
            env_keys: Vec::new(),
            allow_oauth: true,
            now_unix: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeySource {
    RuntimeOverride,
    PersistedApiKey { credential_id: String },
    OAuthCredential { credential_id: String },
    Environment { key: String },
    FallbackResolver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyResolution {
    pub api_key: String,
    pub source: ApiKeySource,
}

#[async_trait]
pub trait FallbackApiKeyResolver: Send + Sync {
    async fn resolve(&self, provider: &str, session_id: &str) -> Result<Option<String>, AuthError>;
}

pub trait EnvProvider: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
}

pub struct StdEnvProvider;

impl EnvProvider for StdEnvProvider {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok()
    }
}

pub struct AuthResolver {
    store: Arc<dyn CredentialStore>,
    oauth_providers: HashMap<String, Arc<dyn OAuthProvider>>,
    fallback_resolver: Option<Arc<dyn FallbackApiKeyResolver>>,
    env_provider: Arc<dyn EnvProvider>,
    blocked_until_unix: Mutex<HashMap<String, i64>>,
    session_affinity: Mutex<HashMap<(String, String), String>>,
    round_robin_cursor: Mutex<HashMap<String, usize>>,
    transient_block_secs: i64,
}

impl AuthResolver {
    pub fn new(store: Arc<dyn CredentialStore>) -> Self {
        Self {
            store,
            oauth_providers: HashMap::new(),
            fallback_resolver: None,
            env_provider: Arc::new(StdEnvProvider),
            blocked_until_unix: Mutex::new(HashMap::new()),
            session_affinity: Mutex::new(HashMap::new()),
            round_robin_cursor: Mutex::new(HashMap::new()),
            transient_block_secs: crate::DEFAULT_TRANSIENT_BLOCK_SECS,
        }
    }

    pub fn register_oauth_provider(&mut self, provider: Arc<dyn OAuthProvider>) {
        self.oauth_providers
            .insert(provider.id().to_string(), provider);
    }

    pub fn set_fallback_resolver(&mut self, resolver: Arc<dyn FallbackApiKeyResolver>) {
        self.fallback_resolver = Some(resolver);
    }

    pub fn set_env_provider(&mut self, provider: Arc<dyn EnvProvider>) {
        self.env_provider = provider;
    }

    pub fn set_transient_block_seconds(&mut self, seconds: i64) {
        self.transient_block_secs = seconds.max(1);
    }

    pub fn report_transient_failure(
        &self,
        credential_id: &str,
        now_unix: i64,
    ) -> Result<(), AuthError> {
        let mut guard = self
            .blocked_until_unix
            .lock()
            .map_err(|_| AuthError::Internal)?;
        guard.insert(
            credential_id.to_string(),
            now_unix + self.transient_block_secs.max(1),
        );
        Ok(())
    }

    pub fn clear_block(&self, credential_id: &str) -> Result<(), AuthError> {
        let mut guard = self
            .blocked_until_unix
            .lock()
            .map_err(|_| AuthError::Internal)?;
        guard.remove(credential_id);
        Ok(())
    }

    pub fn is_blocked(&self, credential_id: &str, now_unix: i64) -> Result<bool, AuthError> {
        let mut guard = self
            .blocked_until_unix
            .lock()
            .map_err(|_| AuthError::Internal)?;
        let blocked = guard
            .get(credential_id)
            .copied()
            .is_some_and(|blocked_until| blocked_until > now_unix);
        if !blocked {
            guard.remove(credential_id);
        }
        Ok(blocked)
    }

    pub async fn get_api_key(
        &self,
        provider: &str,
        session_id: &str,
        options: ApiKeyOptions,
    ) -> Result<Option<ApiKeyResolution>, AuthError> {
        let now_unix = options.now_unix.unwrap_or_else(unix_now);

        if let Some(value) = options.runtime_override {
            if !value.trim().is_empty() {
                return Ok(Some(ApiKeyResolution {
                    api_key: value,
                    source: ApiKeySource::RuntimeOverride,
                }));
            }
        }

        let mut credentials = self.store.list_credentials(provider).await?;
        credentials.retain(|record| !record.disabled);

        if let Some(record) = credentials
            .iter()
            .filter(|record| record.kind == CredentialKind::ApiKey)
            .max_by_key(|record| record.updated_at_unix)
        {
            if let CredentialData::ApiKey(data) = &record.data {
                if !data.api_key.trim().is_empty() {
                    return Ok(Some(ApiKeyResolution {
                        api_key: data.api_key.clone(),
                        source: ApiKeySource::PersistedApiKey {
                            credential_id: record.credential_id.clone(),
                        },
                    }));
                }
            }
        }

        if options.allow_oauth {
            if let Some(result) = self
                .resolve_oauth_key(provider, session_id, now_unix, &credentials)
                .await?
            {
                return Ok(Some(result));
            }
        }

        for env_key in options.env_keys {
            if let Some(value) = self.env_provider.get(&env_key) {
                if !value.trim().is_empty() {
                    return Ok(Some(ApiKeyResolution {
                        api_key: value,
                        source: ApiKeySource::Environment { key: env_key },
                    }));
                }
            }
        }

        if let Some(resolver) = &self.fallback_resolver {
            if let Some(value) = resolver.resolve(provider, session_id).await? {
                if !value.trim().is_empty() {
                    return Ok(Some(ApiKeyResolution {
                        api_key: value,
                        source: ApiKeySource::FallbackResolver,
                    }));
                }
            }
        }

        Ok(None)
    }

    async fn resolve_oauth_key(
        &self,
        provider: &str,
        session_id: &str,
        now_unix: i64,
        credentials: &[CredentialRecord],
    ) -> Result<Option<ApiKeyResolution>, AuthError> {
        let oauth_records: Vec<CredentialRecord> = credentials
            .iter()
            .filter(|record| record.kind == CredentialKind::OAuth)
            .cloned()
            .collect();

        if oauth_records.is_empty() {
            return Ok(None);
        }

        let usage = self.store.list_usage(provider).await?;
        let ordered = self.order_oauth_records(provider, session_id, oauth_records, &usage)?;

        for mut record in ordered {
            if self.is_blocked(&record.credential_id, now_unix)? {
                continue;
            }

            let mut oauth = match record.data.clone() {
                CredentialData::OAuth(v) => v,
                _ => continue,
            };

            if oauth.is_expired(now_unix) {
                let Some(oauth_provider) = self.oauth_providers.get(provider) else {
                    return Err(AuthError::MissingOAuthProvider(provider.to_string()));
                };

                match oauth_provider.refresh(&oauth).await {
                    Ok(refreshed) => {
                        oauth = refreshed.clone();
                        record.data = CredentialData::OAuth(refreshed);
                        record.updated_at_unix = now_unix;
                        self.store.upsert(&record).await?;
                    }
                    Err(err) if err.is_definitive() => {
                        self.store.disable(&record.credential_id).await?;
                        self.clear_block(&record.credential_id)?;
                        continue;
                    }
                    Err(_) => {
                        self.report_transient_failure(&record.credential_id, now_unix)?;
                        continue;
                    }
                }
            }

            if oauth.access_token.trim().is_empty() {
                continue;
            }

            let mut affinity = self
                .session_affinity
                .lock()
                .map_err(|_| AuthError::Internal)?;
            affinity.insert(
                (provider.to_string(), session_id.to_string()),
                record.credential_id.clone(),
            );

            return Ok(Some(ApiKeyResolution {
                api_key: oauth.access_token,
                source: ApiKeySource::OAuthCredential {
                    credential_id: record.credential_id,
                },
            }));
        }

        Ok(None)
    }

    fn order_oauth_records(
        &self,
        provider: &str,
        session_id: &str,
        mut records: Vec<CredentialRecord>,
        usage: &HashMap<String, CredentialUsage>,
    ) -> Result<Vec<CredentialRecord>, AuthError> {
        records.sort_by(|a, b| {
            let score_a = usage
                .get(&a.credential_id)
                .map(|v| v.remaining_ratio)
                .unwrap_or(1.0);
            let score_b = usage
                .get(&b.credential_id)
                .map(|v| v.remaining_ratio)
                .unwrap_or(1.0);

            score_b
                .partial_cmp(&score_a)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.credential_id.cmp(&b.credential_id))
        });

        let key = (provider.to_string(), session_id.to_string());
        let affinity = self
            .session_affinity
            .lock()
            .map_err(|_| AuthError::Internal)?;
        if let Some(preferred) = affinity.get(&key) {
            if let Some(idx) = records
                .iter()
                .position(|record| &record.credential_id == preferred)
            {
                records.rotate_left(idx);
                return Ok(records);
            }
        }
        drop(affinity);

        if records.len() > 1 {
            let mut rr = self
                .round_robin_cursor
                .lock()
                .map_err(|_| AuthError::Internal)?;
            let start = rr.get(provider).copied().unwrap_or(0) % records.len();
            records.rotate_left(start);
            rr.insert(provider.to_string(), (start + 1) % records.len());
        }

        Ok(records)
    }
}
