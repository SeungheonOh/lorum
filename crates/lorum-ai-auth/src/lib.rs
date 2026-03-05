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

pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}
#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use futures::executor::block_on;
    use serde_json::Value;
    use std::collections::{BTreeMap, HashMap, VecDeque};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    use super::*;

    struct MapEnvProvider {
        values: BTreeMap<String, String>,
    }

    impl EnvProvider for MapEnvProvider {
        fn get(&self, key: &str) -> Option<String> {
            self.values.get(key).cloned()
        }
    }

    struct MockFallbackResolver {
        value: Option<String>,
    }

    #[async_trait]
    impl FallbackApiKeyResolver for MockFallbackResolver {
        async fn resolve(
            &self,
            _provider: &str,
            _session_id: &str,
        ) -> Result<Option<String>, AuthError> {
            Ok(self.value.clone())
        }
    }

    struct MockOAuthProvider {
        id: String,
        refresh_map: Mutex<HashMap<String, Result<OAuthCredential, OAuthRefreshError>>>,
    }

    impl MockOAuthProvider {
        fn new(id: &str) -> Self {
            Self {
                id: id.to_string(),
                refresh_map: Mutex::new(HashMap::new()),
            }
        }

        fn set_refresh_result(
            &self,
            refresh_token: &str,
            result: Result<OAuthCredential, OAuthRefreshError>,
        ) {
            let mut guard = self.refresh_map.lock().expect("lock refresh map");
            guard.insert(refresh_token.to_string(), result);
        }
    }

    #[async_trait]
    impl OAuthProvider for MockOAuthProvider {
        fn id(&self) -> &str {
            &self.id
        }

        async fn begin_flow(&self, ctx: OAuthBeginContext) -> Result<OAuthStart, AuthError> {
            let state = ctx.state.unwrap_or_else(|| "s".to_string());
            Ok(OAuthStart {
                authorization_url: format!("https://example.test/auth?state={state}"),
                state,
                code_verifier: Some("verifier".to_string()),
            })
        }

        async fn exchange_code(
            &self,
            code: &str,
            _verifier: Option<&str>,
        ) -> Result<OAuthToken, AuthError> {
            Ok(OAuthToken {
                credential: OAuthCredential {
                    access_token: format!("access-{code}"),
                    refresh_token: Some("refresh-token".to_string()),
                    expires_at_unix: Some(unix_now() + 3600),
                    identity: Some("user".to_string()),
                },
            })
        }

        async fn refresh(
            &self,
            credential: &OAuthCredential,
        ) -> Result<OAuthCredential, OAuthRefreshError> {
            let refresh = credential
                .refresh_token
                .as_deref()
                .ok_or_else(|| OAuthRefreshError::Permanent("missing refresh token".to_string()))?;
            let guard = self.refresh_map.lock().expect("lock refresh map");
            guard.get(refresh).cloned().unwrap_or_else(|| {
                Err(OAuthRefreshError::Transient(
                    "no result configured".to_string(),
                ))
            })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct HttpCall {
        url: String,
        form: Vec<(String, String)>,
        headers: Vec<(String, String)>,
    }

    struct MockOAuthHttpClient {
        responses: Mutex<VecDeque<Result<Value, AuthError>>>,
        calls: Mutex<Vec<HttpCall>>,
    }

    impl MockOAuthHttpClient {
        fn with_responses(responses: Vec<Result<Value, AuthError>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl OAuthHttpClient for MockOAuthHttpClient {
        async fn post_form(
            &self,
            url: &str,
            form: &[(String, String)],
            headers: &[(String, String)],
        ) -> Result<Value, AuthError> {
            self.calls.lock().expect("lock calls").push(HttpCall {
                url: url.to_string(),
                form: form.to_vec(),
                headers: headers.to_vec(),
            });

            self.responses
                .lock()
                .expect("lock responses")
                .pop_front()
                .unwrap_or_else(|| Err(AuthError::Database("missing mock response".to_string())))
        }
    }
    fn create_store() -> Arc<dyn CredentialStore> {
        let dir = tempdir().expect("create temp dir");
        let path = dir.path().join("agent.db");
        let store = SqliteCredentialStore::open(path).expect("open sqlite store");
        std::mem::forget(dir);
        Arc::new(store)
    }

    fn api_key_record(
        id: &str,
        provider: &str,
        api_key: &str,
        updated_at: i64,
    ) -> CredentialRecord {
        CredentialRecord {
            credential_id: id.to_string(),
            provider: provider.to_string(),
            kind: CredentialKind::ApiKey,
            disabled: false,
            data: CredentialData::ApiKey(ApiKeyCredential {
                api_key: api_key.to_string(),
            }),
            created_at_unix: updated_at - 1,
            updated_at_unix: updated_at,
        }
    }

    fn oauth_record(
        id: &str,
        provider: &str,
        access_token: &str,
        refresh_token: &str,
        expires_at_unix: Option<i64>,
        updated_at: i64,
    ) -> CredentialRecord {
        CredentialRecord {
            credential_id: id.to_string(),
            provider: provider.to_string(),
            kind: CredentialKind::OAuth,
            disabled: false,
            data: CredentialData::OAuth(OAuthCredential {
                access_token: access_token.to_string(),
                refresh_token: Some(refresh_token.to_string()),
                expires_at_unix,
                identity: None,
            }),
            created_at_unix: updated_at - 1,
            updated_at_unix: updated_at,
        }
    }

    #[test]
    fn sqlite_upsert_and_list_credentials_roundtrip() {
        block_on(async {
            let store = create_store();
            let record = api_key_record("cred-1", "openai", "k1", 10);
            store.upsert(&record).await.expect("upsert credential");

            let listed = store
                .list_credentials("openai")
                .await
                .expect("list credentials");
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0], record);
        });
    }

    #[test]
    fn sqlite_disable_marks_credential_disabled() {
        block_on(async {
            let store = create_store();
            let record = api_key_record("cred-1", "openai", "k1", 10);
            store.upsert(&record).await.expect("upsert credential");
            store.disable("cred-1").await.expect("disable credential");

            let loaded = store
                .get_credential("cred-1")
                .await
                .expect("get credential")
                .expect("credential exists");
            assert!(loaded.disabled);
        });
    }

    #[test]
    fn sqlite_usage_roundtrip() {
        block_on(async {
            let store = create_store();
            let usage = CredentialUsage {
                remaining_ratio: 0.42,
                reset_at_unix: Some(123),
                updated_at_unix: 100,
            };

            store
                .put_usage("openai", "cred-1", &usage)
                .await
                .expect("put usage");

            let listed = store.list_usage("openai").await.expect("list usage");
            assert_eq!(listed.get("cred-1"), Some(&usage));
        });
    }

    #[test]
    fn resolver_uses_runtime_override_first() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&api_key_record("cred-1", "openai", "stored", 10))
                .await
                .expect("upsert");

            let resolver = AuthResolver::new(store);
            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        runtime_override: Some("runtime".to_string()),
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "runtime");
            assert_eq!(result.source, ApiKeySource::RuntimeOverride);
        });
    }

    #[test]
    fn resolver_prefers_persisted_api_key_over_oauth() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "oauth-key",
                    "r1",
                    None,
                    10,
                ))
                .await
                .expect("upsert oauth");
            store
                .upsert(&api_key_record("api-1", "openai", "api-key", 11))
                .await
                .expect("upsert api key");

            let resolver = AuthResolver::new(store);
            let result = resolver
                .get_api_key("openai", "session", ApiKeyOptions::default())
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "api-key");
            assert_eq!(
                result.source,
                ApiKeySource::PersistedApiKey {
                    credential_id: "api-1".to_string()
                }
            );
        });
    }

    #[test]
    fn resolver_uses_oauth_when_api_key_missing() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "oauth-key",
                    "r1",
                    None,
                    10,
                ))
                .await
                .expect("upsert oauth");

            let resolver = AuthResolver::new(store);
            let result = resolver
                .get_api_key("openai", "session", ApiKeyOptions::default())
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "oauth-key");
            assert_eq!(
                result.source,
                ApiKeySource::OAuthCredential {
                    credential_id: "oauth-1".to_string()
                }
            );
        });
    }

    #[test]
    fn resolver_refreshes_expired_oauth_credential() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "old-token",
                    "rt-1",
                    Some(5),
                    10,
                ))
                .await
                .expect("upsert oauth");

            let provider = Arc::new(MockOAuthProvider::new("openai"));
            provider.set_refresh_result(
                "rt-1",
                Ok(OAuthCredential {
                    access_token: "new-token".to_string(),
                    refresh_token: Some("rt-1".to_string()),
                    expires_at_unix: Some(10_000),
                    identity: None,
                }),
            );

            let mut resolver = AuthResolver::new(store.clone());
            resolver.register_oauth_provider(provider);

            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        now_unix: Some(100),
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "new-token");
            let loaded = store
                .get_credential("oauth-1")
                .await
                .expect("load credential")
                .expect("credential exists");
            match loaded.data {
                CredentialData::OAuth(data) => assert_eq!(data.access_token, "new-token"),
                _ => panic!("unexpected credential kind"),
            }
        });
    }

    #[test]
    fn resolver_disables_credential_on_definitive_refresh_failure() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "old-token",
                    "rt-1",
                    Some(5),
                    10,
                ))
                .await
                .expect("upsert oauth");

            let provider = Arc::new(MockOAuthProvider::new("openai"));
            provider.set_refresh_result("rt-1", Err(OAuthRefreshError::InvalidGrant));

            let mut resolver = AuthResolver::new(store.clone());
            resolver.register_oauth_provider(provider);

            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        now_unix: Some(100),
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key");
            assert!(result.is_none());

            let loaded = store
                .get_credential("oauth-1")
                .await
                .expect("load credential")
                .expect("credential exists");
            assert!(loaded.disabled);
        });
    }

    #[test]
    fn resolver_blocks_credential_on_transient_refresh_failure() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "old-token",
                    "rt-1",
                    Some(5),
                    10,
                ))
                .await
                .expect("upsert oauth");

            let provider = Arc::new(MockOAuthProvider::new("openai"));
            provider.set_refresh_result(
                "rt-1",
                Err(OAuthRefreshError::Transient("timeout".to_string())),
            );

            let mut resolver = AuthResolver::new(store);
            resolver.register_oauth_provider(provider);
            resolver.set_transient_block_seconds(20);

            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        now_unix: Some(100),
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key");
            assert!(result.is_none());
            assert!(resolver.is_blocked("oauth-1", 105).expect("blocked check"));
            assert!(!resolver.is_blocked("oauth-1", 121).expect("blocked check"));
        });
    }

    #[test]
    fn resolver_uses_environment_after_credentials_exhausted() {
        block_on(async {
            let store = create_store();
            let mut resolver = AuthResolver::new(store);
            resolver.set_env_provider(Arc::new(MapEnvProvider {
                values: BTreeMap::from([("OPENAI_API_KEY".to_string(), "env-key".to_string())]),
            }));

            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        env_keys: vec!["OPENAI_API_KEY".to_string()],
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "env-key");
            assert_eq!(
                result.source,
                ApiKeySource::Environment {
                    key: "OPENAI_API_KEY".to_string()
                }
            );
        });
    }

    #[test]
    fn resolver_uses_fallback_as_last_resort() {
        block_on(async {
            let store = create_store();
            let mut resolver = AuthResolver::new(store);
            resolver.set_fallback_resolver(Arc::new(MockFallbackResolver {
                value: Some("fallback-key".to_string()),
            }));

            let result = resolver
                .get_api_key("openai", "session", ApiKeyOptions::default())
                .await
                .expect("resolve api key")
                .expect("must resolve");

            assert_eq!(result.api_key, "fallback-key");
            assert_eq!(result.source, ApiKeySource::FallbackResolver);
        });
    }

    #[test]
    fn resolver_can_disable_oauth_resolution() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-1",
                    "openai",
                    "oauth-key",
                    "r1",
                    None,
                    10,
                ))
                .await
                .expect("upsert oauth");

            let resolver = AuthResolver::new(store);
            let result = resolver
                .get_api_key(
                    "openai",
                    "session",
                    ApiKeyOptions {
                        allow_oauth: false,
                        ..ApiKeyOptions::default()
                    },
                )
                .await
                .expect("resolve api key");

            assert!(result.is_none());
        });
    }

    #[test]
    fn resolver_prefers_session_affinity_for_oauth() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-a", "openai", "token-a", "r-a", None, 10,
                ))
                .await
                .expect("upsert oauth-a");
            store
                .upsert(&oauth_record(
                    "oauth-b", "openai", "token-b", "r-b", None, 11,
                ))
                .await
                .expect("upsert oauth-b");

            let resolver = AuthResolver::new(store);

            let first = resolver
                .get_api_key("openai", "s1", ApiKeyOptions::default())
                .await
                .expect("resolve first")
                .expect("first exists");
            let second = resolver
                .get_api_key("openai", "s1", ApiKeyOptions::default())
                .await
                .expect("resolve second")
                .expect("second exists");

            assert_eq!(first.api_key, second.api_key);
        });
    }

    #[test]
    fn resolver_round_robin_for_new_sessions() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-a", "openai", "token-a", "r-a", None, 10,
                ))
                .await
                .expect("upsert oauth-a");
            store
                .upsert(&oauth_record(
                    "oauth-b", "openai", "token-b", "r-b", None, 11,
                ))
                .await
                .expect("upsert oauth-b");

            let resolver = AuthResolver::new(store);

            let s1 = resolver
                .get_api_key("openai", "s1", ApiKeyOptions::default())
                .await
                .expect("resolve s1")
                .expect("s1 exists");
            let s2 = resolver
                .get_api_key("openai", "s2", ApiKeyOptions::default())
                .await
                .expect("resolve s2")
                .expect("s2 exists");

            assert_ne!(s1.api_key, s2.api_key);
        });
    }

    #[test]
    fn resolver_usage_ranking_prefers_less_exhausted_credential() {
        block_on(async {
            let store = create_store();
            store
                .upsert(&oauth_record(
                    "oauth-a", "openai", "token-a", "r-a", None, 10,
                ))
                .await
                .expect("upsert oauth-a");
            store
                .upsert(&oauth_record(
                    "oauth-b", "openai", "token-b", "r-b", None, 11,
                ))
                .await
                .expect("upsert oauth-b");

            store
                .put_usage(
                    "openai",
                    "oauth-a",
                    &CredentialUsage {
                        remaining_ratio: 0.2,
                        reset_at_unix: None,
                        updated_at_unix: 1,
                    },
                )
                .await
                .expect("put usage a");

            store
                .put_usage(
                    "openai",
                    "oauth-b",
                    &CredentialUsage {
                        remaining_ratio: 0.9,
                        reset_at_unix: None,
                        updated_at_unix: 1,
                    },
                )
                .await
                .expect("put usage b");

            let resolver = AuthResolver::new(store);
            let result = resolver
                .get_api_key("openai", "session", ApiKeyOptions::default())
                .await
                .expect("resolve")
                .expect("must resolve");

            assert_eq!(result.api_key, "token-b");
        });
    }

    #[test]
    fn callback_flow_generates_unique_states() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let a = flow.generate_state();
        let b = flow.generate_state();
        assert_ne!(a, b);
    }

    #[test]
    fn callback_flow_parse_callback_url_success() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let code = flow
            .parse_callback_url("http://127.0.0.1/callback?code=abc&state=s1", "s1")
            .expect("parse callback");
        assert_eq!(code, "abc");
    }

    #[test]
    fn callback_flow_parse_callback_url_state_mismatch() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let err = flow
            .parse_callback_url("http://127.0.0.1/callback?code=abc&state=s2", "s1")
            .expect_err("must fail");
        assert_eq!(err, OAuthCallbackError::StateMismatch);
    }

    #[test]
    fn callback_flow_parse_callback_url_authorization_error() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let err = flow
            .parse_callback_url(
                "http://127.0.0.1/callback?error=invalid_scope&error_description=scope+denied&state=s1",
                "s1",
            )
            .expect_err("must fail");
        assert_eq!(
            err,
            OAuthCallbackError::AuthorizationFailed {
                error: "invalid_scope".to_string(),
                description: "scope denied".to_string(),
            }
        );
    }

    #[test]
    fn callback_flow_parse_manual_code_supports_plain_code() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let code = flow
            .parse_manual_code("  abc123  ")
            .expect("manual code parse");
        assert_eq!(code, "abc123");
    }

    #[test]
    fn callback_flow_parse_manual_code_rejects_empty() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let err = flow.parse_manual_code("   ").expect_err("must fail");
        assert_eq!(err, OAuthCallbackError::MissingManualCode);
    }

    #[test]
    fn callback_flow_choose_callback_port_succeeds() {
        let flow = OAuthCallbackFlow::new(0, 300);
        let port = flow.choose_callback_port().expect("choose port");
        assert!(port > 0);
    }

    #[test]
    fn oauth_begin_and_exchange_contract_surface() {
        block_on(async {
            let provider = MockOAuthProvider::new("openai");
            let start = provider
                .begin_flow(OAuthBeginContext {
                    redirect_uri: "http://localhost/callback".to_string(),
                    scopes: vec!["offline_access".to_string()],
                    state: Some("state-1".to_string()),
                })
                .await
                .expect("begin flow");
            assert!(start.authorization_url.contains("state=state-1"));

            let token = provider
                .exchange_code("auth-code", start.code_verifier.as_deref())
                .await
                .expect("exchange code");
            assert_eq!(token.credential.access_token, "access-auth-code");
        });
    }
    #[test]
    fn callback_flow_parse_callback_or_manual_url_input() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let code = flow
            .parse_callback_or_manual_input("http://127.0.0.1/callback?code=code-1&state=s1", "s1")
            .expect("parse callback input");
        assert_eq!(code, "code-1");
    }

    #[test]
    fn callback_flow_parse_callback_or_manual_plain_code() {
        let flow = OAuthCallbackFlow::new(3000, 300);
        let code = flow
            .parse_callback_or_manual_input("raw-code", "ignored")
            .expect("parse manual input");
        assert_eq!(code, "raw-code");
    }

    #[test]
    fn codex_begin_flow_emits_pkce_and_offline_scope() {
        block_on(async {
            let client = Arc::new(MockOAuthHttpClient::with_responses(vec![]));
            let provider =
                OpenAiCodexOAuthProvider::new(client, "client-1", "http://127.0.0.1/callback");

            let start = provider
                .begin_flow(OAuthBeginContext {
                    redirect_uri: "".to_string(),
                    scopes: vec!["openid".to_string()],
                    state: Some("state-x".to_string()),
                })
                .await
                .expect("begin flow");

            assert!(start
                .authorization_url
                .contains("code_challenge_method=S256"));
            assert!(start.authorization_url.contains("offline_access"));
            assert!(start.authorization_url.contains("profile"));
            assert!(start.authorization_url.contains("email"));
            assert!(!start.authorization_url.contains("api.responses.write"));
            assert!(start.code_verifier.as_deref().is_some());
        });
    }

    #[test]
    fn codex_exchange_code_posts_expected_form_and_parses_token() {
        block_on(async {
            let client = Arc::new(MockOAuthHttpClient::with_responses(vec![Ok(
                serde_json::json!({
                    "access_token": "acc-1",
                    "refresh_token": "ref-1",
                    "expires_in": 3600
                }),
            )]));
            let provider = OpenAiCodexOAuthProvider::new(
                client.clone(),
                "client-1",
                "http://127.0.0.1/callback",
            );

            let token = provider
                .exchange_code("auth-code", Some("verifier-1"))
                .await
                .expect("exchange code");

            assert_eq!(token.credential.access_token, "acc-1");
            assert_eq!(token.credential.refresh_token.as_deref(), Some("ref-1"));
            assert!(token.credential.expires_at_unix.is_some());

            let calls = client.calls.lock().expect("calls lock");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].url, "https://auth.openai.com/oauth/token");
            assert!(calls[0]
                .form
                .iter()
                .any(|(k, v)| k == "grant_type" && v == "authorization_code"));
            assert!(calls[0]
                .form
                .iter()
                .any(|(k, v)| k == "code_verifier" && v == "verifier-1"));
        });
    }

    #[test]
    fn codex_refresh_maps_invalid_grant_error() {
        block_on(async {
            let client = Arc::new(MockOAuthHttpClient::with_responses(vec![Ok(
                serde_json::json!({
                    "error": "invalid_grant",
                    "error_description": "revoked"
                }),
            )]));
            let provider =
                OpenAiCodexOAuthProvider::new(client, "client-1", "http://127.0.0.1/callback");

            let result = provider
                .refresh(&OAuthCredential {
                    access_token: "old".to_string(),
                    refresh_token: Some("ref-1".to_string()),
                    expires_at_unix: Some(1),
                    identity: None,
                })
                .await;

            assert!(matches!(result, Err(OAuthRefreshError::InvalidGrant)));
        });
    }

    #[test]
    fn codex_refresh_keeps_prior_refresh_token_when_omitted() {
        block_on(async {
            let client = Arc::new(MockOAuthHttpClient::with_responses(vec![Ok(
                serde_json::json!({
                    "access_token": "new-access",
                    "expires_in": 100
                }),
            )]));
            let provider =
                OpenAiCodexOAuthProvider::new(client, "client-1", "http://127.0.0.1/callback");

            let refreshed = provider
                .refresh(&OAuthCredential {
                    access_token: "old".to_string(),
                    refresh_token: Some("ref-keep".to_string()),
                    expires_at_unix: Some(1),
                    identity: Some("id".to_string()),
                })
                .await
                .expect("refresh token");

            assert_eq!(refreshed.access_token, "new-access");
            assert_eq!(refreshed.refresh_token.as_deref(), Some("ref-keep"));
            assert_eq!(refreshed.identity.as_deref(), Some("id"));
        });
    }
}
