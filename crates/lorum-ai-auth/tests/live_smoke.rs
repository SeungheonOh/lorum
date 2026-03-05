use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_auth::{
    AuthError, OAuthBeginContext, OAuthCredential, OAuthHttpClient, OAuthProvider,
    OpenAiCodexOAuthProvider,
};
use serde_json::Value;

fn smoke_enabled() -> bool {
    matches!(std::env::var("OMP_LIVE_SMOKE").as_deref(), Ok("1"))
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required env var: {name}"))
}

#[derive(Debug, Clone)]
struct RecordedCall {
    url: String,
    form: Vec<(String, String)>,
    headers: Vec<(String, String)>,
}

struct StaticHttpClient {
    responses: Mutex<Vec<Result<Value, AuthError>>>,
    calls: Mutex<Vec<RecordedCall>>,
}

impl StaticHttpClient {
    fn with_responses(responses: Vec<Result<Value, AuthError>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().rev().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl OAuthHttpClient for StaticHttpClient {
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
        headers: &[(String, String)],
    ) -> Result<Value, AuthError> {
        self.calls.lock().expect("lock calls").push(RecordedCall {
            url: url.to_string(),
            form: form.to_vec(),
            headers: headers.to_vec(),
        });
        self.responses
            .lock()
            .expect("lock responses")
            .pop()
            .unwrap_or_else(|| {
                Err(AuthError::InvalidCredential(
                    "missing smoke response fixture".to_string(),
                ))
            })
    }
}

#[test]
#[ignore = "requires OMP_LIVE_SMOKE=1 and provider secrets"]
fn codex_oauth_live_smoke_scaffold_begin_exchange_refresh() {
    if !smoke_enabled() {
        return;
    }

    block_on(async {
        let client_id = require_env("OMP_SMOKE_OPENAI_CLIENT_ID");
        let redirect_uri = require_env("OMP_SMOKE_OPENAI_REDIRECT_URI");

        let client = Arc::new(StaticHttpClient::with_responses(vec![
            Ok(serde_json::json!({
                "access_token": "smoke-access-1",
                "refresh_token": "smoke-refresh-1",
                "expires_in": 3600
            })),
            Ok(serde_json::json!({
                "access_token": "smoke-access-2",
                "expires_in": 3600
            })),
        ]));

        let provider = OpenAiCodexOAuthProvider::new(client.clone(), &client_id, &redirect_uri);

        let begin = provider
            .begin_flow(OAuthBeginContext {
                redirect_uri: redirect_uri.clone(),
                scopes: vec!["offline_access".to_string()],
                state: Some("smoke-state".to_string()),
            })
            .await
            .expect("begin flow");

        assert!(begin.authorization_url.contains("smoke-state"));
        assert!(begin.code_verifier.as_deref().is_some());

        let exchanged = provider
            .exchange_code("smoke-code", begin.code_verifier.as_deref())
            .await
            .expect("exchange code");
        assert_eq!(exchanged.credential.access_token, "smoke-access-1");
        assert_eq!(
            exchanged.credential.refresh_token.as_deref(),
            Some("smoke-refresh-1")
        );

        let refreshed = provider
            .refresh(&OAuthCredential {
                access_token: exchanged.credential.access_token,
                refresh_token: exchanged.credential.refresh_token,
                expires_at_unix: exchanged.credential.expires_at_unix,
                identity: exchanged.credential.identity,
            })
            .await
            .expect("refresh token");

        assert_eq!(refreshed.access_token, "smoke-access-2");
        assert_eq!(refreshed.refresh_token.as_deref(), Some("smoke-refresh-1"));

        let calls = client.calls.lock().expect("calls lock");
        assert_eq!(calls.len(), 2);
        assert!(calls
            .iter()
            .all(|call| call.url == "https://auth.openai.com/oauth/token"));
        assert!(calls.iter().all(|call| call.headers.is_empty()));
        assert!(calls
            .iter()
            .flat_map(|call| call.form.iter())
            .any(|(k, v)| k == "client_id" && v == &client_id));
    });
}
