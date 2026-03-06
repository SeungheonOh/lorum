use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use tempfile::tempdir;

use lorum_ai_contract::ApiKind;
use lorum_ai_models::{
    parse_model_like, ModelCacheEntry, ModelCacheStore, ModelError, ModelInfo, ModelManager,
    ModelSource, ProviderDescriptor, ProviderDescriptorRegistry, ResolveOptions,
    SourceModels, SqliteModelCache, CACHE_VERSION,
};

struct MockSource {
    responses: Mutex<HashMap<String, Result<SourceModels, ModelError>>>,
}

impl MockSource {
    fn new() -> Self {
        Self {
            responses: Mutex::new(HashMap::new()),
        }
    }

    fn set(&self, provider: &str, response: Result<SourceModels, ModelError>) {
        let mut guard = self.responses.lock().expect("lock source responses");
        guard.insert(provider.to_string(), response);
    }
}

#[async_trait]
impl ModelSource for MockSource {
    async fn fetch_models(&self, provider_id: &str) -> Result<SourceModels, ModelError> {
        let guard = self.responses.lock().expect("lock source responses");
        guard.get(provider_id).cloned().unwrap_or_else(|| {
            Err(ModelError::Source(
                "missing mock source response".to_string(),
            ))
        })
    }
}

struct FailingCache;

#[async_trait]
impl ModelCacheStore for FailingCache {
    async fn get(&self, _provider_id: &str) -> Result<Option<ModelCacheEntry>, ModelError> {
        Err(ModelError::Database("cache read explode".to_string()))
    }

    async fn put(&self, _entry: &ModelCacheEntry) -> Result<(), ModelError> {
        Err(ModelError::Database("cache write explode".to_string()))
    }
}

fn parse_model(value: serde_json::Value, provider: &str) -> ModelInfo {
    parse_model_like(&value, provider).expect("parse model")
}

fn model_json(
    api: &str,
    model_id: &str,
    display_name: &str,
    context_window: u64,
) -> serde_json::Value {
    serde_json::json!({
        "provider": "openai",
        "api": api,
        "model_id": model_id,
        "display_name": display_name,
        "context_window": context_window,
        "supports_tools": true,
        "stale": false
    })
}

fn sqlite_cache() -> Arc<dyn ModelCacheStore> {
    let dir = tempdir().expect("create temp dir");
    let path = dir.path().join("models.db");
    let cache = SqliteModelCache::open(path).expect("open sqlite model cache");
    std::mem::forget(dir);
    Arc::new(cache)
}

#[test]
fn parse_model_like_accepts_model_id_and_api() {
    let value = model_json("openai-responses", "gpt-5.2", "GPT 5.2", 200000);
    let model = parse_model_like(&value, "openai").expect("parse model");

    assert_eq!(model.model_id, "gpt-5.2");
    assert_eq!(model.api, ApiKind::OpenAiResponses);
    assert_eq!(model.display_name.as_deref(), Some("GPT 5.2"));
}

#[test]
fn parse_model_like_rejects_missing_api() {
    let value = serde_json::json!({"model_id":"x"});
    assert!(parse_model_like(&value, "openai").is_none());
}

#[test]
fn model_merge_over_prefers_new_non_null_fields() {
    let base = ModelInfo {
        provider: "openai".to_string(),
        api: ApiKind::OpenAiResponses,
        model_id: "gpt-5.2".to_string(),
        display_name: Some("Base".to_string()),
        context_window: Some(1000),
        supports_tools: Some(false),
        stale: Some(false),
    };
    let patch = ModelInfo {
        provider: "openai".to_string(),
        api: ApiKind::OpenAiResponses,
        model_id: "gpt-5.2".to_string(),
        display_name: Some("Patch".to_string()),
        context_window: None,
        supports_tools: Some(true),
        stale: None,
    };

    let merged = patch.merge_over(&base);
    assert_eq!(merged.display_name.as_deref(), Some("Patch"));
    assert_eq!(merged.context_window, Some(1000));
    assert_eq!(merged.supports_tools, Some(true));
    assert_eq!(merged.stale, Some(false));
}

#[test]
fn sqlite_cache_put_get_roundtrip() {
    block_on(async {
        let cache = sqlite_cache();
        let entry = ModelCacheEntry {
            provider_id: "openai".to_string(),
            version: CACHE_VERSION,
            updated_at_unix: 100,
            authoritative: true,
            models: vec![parse_model(
                model_json("openai-responses", "gpt-5.2", "GPT", 1),
                "openai",
            )],
        };

        cache.put(&entry).await.expect("put cache entry");
        let loaded = cache
            .get("openai")
            .await
            .expect("get cache entry")
            .expect("cache entry exists");
        assert_eq!(loaded, entry);
    });
}

#[test]
fn descriptor_registry_stores_and_lists_default_models() {
    let registry = ProviderDescriptorRegistry::default();
    registry
        .register(ProviderDescriptor {
            provider_id: "openai".to_string(),
            default_model: "gpt-5.2".to_string(),
            default_api: ApiKind::OpenAiResponses,
            allow_unauthenticated_discovery: false,
            catalog_metadata: None,
        })
        .expect("register descriptor");

    let map = registry.default_models().expect("default models map");
    assert_eq!(map.get("openai"), Some(&"gpt-5.2".to_string()));
}

#[test]
fn resolve_models_merges_in_precedence_order() {
    block_on(async {
        let cache = sqlite_cache();
        cache
            .put(&ModelCacheEntry {
                provider_id: "openai".to_string(),
                version: CACHE_VERSION,
                updated_at_unix: 10,
                authoritative: true,
                models: vec![parse_model(
                    serde_json::json!({
                        "provider":"openai",
                        "api":"openai-responses",
                        "model_id":"gpt-5.2",
                        "display_name":"Cache Name"
                    }),
                    "openai",
                )],
            })
            .await
            .expect("seed cache");

        let models_dev = Arc::new(MockSource::new());
        models_dev.set(
            "openai",
            Ok(SourceModels {
                models: vec![serde_json::json!({
                    "provider":"openai",
                    "api":"openai-responses",
                    "model_id":"gpt-5.2",
                    "display_name":"ModelsDev Name"
                })],
                authoritative: false,
            }),
        );

        let dynamic = Arc::new(MockSource::new());
        dynamic.set(
            "openai",
            Ok(SourceModels {
                models: vec![serde_json::json!({
                    "provider":"openai",
                    "api":"openai-responses",
                    "model_id":"gpt-5.2",
                    "display_name":"Dynamic Name",
                    "context_window": 9999
                })],
                authoritative: true,
            }),
        );

        let manager = ModelManager::new(cache)
            .with_models_dev_source(models_dev)
            .with_dynamic_source(dynamic);

        let resolution = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    static_models: vec![parse_model(
                        serde_json::json!({
                            "provider":"openai",
                            "api":"openai-responses",
                            "model_id":"gpt-5.2",
                            "display_name":"Static Name"
                        }),
                        "openai",
                    )],
                    use_models_dev: true,
                    use_dynamic: true,
                    now_unix: Some(1000),
                },
            )
            .await
            .expect("resolve models");

        let model = resolution
            .models
            .iter()
            .find(|model| model.model_id == "gpt-5.2")
            .expect("model exists");
        assert_eq!(model.display_name.as_deref(), Some("Dynamic Name"));
        assert_eq!(model.context_window, Some(9999));
    });
}

#[test]
fn resolve_models_drops_malformed_entries() {
    block_on(async {
        let cache = sqlite_cache();
        let dynamic = Arc::new(MockSource::new());
        dynamic.set(
            "openai",
            Ok(SourceModels {
                models: vec![
                    serde_json::json!({"api":"openai-responses","model_id":"ok"}),
                    serde_json::json!({"model_id":"broken"}),
                ],
                authoritative: true,
            }),
        );

        let manager = ModelManager::new(cache).with_dynamic_source(dynamic);
        let resolution = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    use_dynamic: true,
                    ..ResolveOptions::default()
                },
            )
            .await
            .expect("resolve models");

        assert_eq!(resolution.models.len(), 1);
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("dropped malformed model entry")));
    });
}

#[test]
fn resolve_models_dynamic_retry_backoff_for_non_authoritative_cache() {
    block_on(async {
        let cache = sqlite_cache();
        cache
            .put(&ModelCacheEntry {
                provider_id: "openai".to_string(),
                version: CACHE_VERSION,
                updated_at_unix: 10,
                authoritative: false,
                models: vec![parse_model(
                    model_json("openai-responses", "gpt-5.2", "Cached", 10),
                    "openai",
                )],
            })
            .await
            .expect("seed cache");

        let dynamic = Arc::new(MockSource::new());
        dynamic.set(
            "openai",
            Err(ModelError::Source("dynamic down".to_string())),
        );

        let mut manager = ModelManager::new(cache).with_dynamic_source(dynamic);
        manager.set_non_authoritative_retry_backoff_secs(50);

        let first = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    use_dynamic: true,
                    now_unix: Some(1000),
                    ..ResolveOptions::default()
                },
            )
            .await
            .expect("first resolve");
        assert!(first
            .warnings
            .iter()
            .any(|warning| warning.contains("dynamic fetch failed")));

        let second = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    use_dynamic: true,
                    now_unix: Some(1020),
                    ..ResolveOptions::default()
                },
            )
            .await
            .expect("second resolve");
        assert!(second.warnings.iter().any(|warning| {
            warning.contains("dynamic fetch skipped due to non-authoritative retry backoff")
        }));
    });
}

#[test]
fn resolve_models_tolerates_cache_failures() {
    block_on(async {
        let dynamic = Arc::new(MockSource::new());
        dynamic.set(
            "openai",
            Ok(SourceModels {
                models: vec![model_json("openai-responses", "gpt-5.2", "Dynamic", 100)],
                authoritative: true,
            }),
        );

        let manager = ModelManager::new(Arc::new(FailingCache)).with_dynamic_source(dynamic);
        let resolution = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    use_dynamic: true,
                    ..ResolveOptions::default()
                },
            )
            .await
            .expect("resolve despite cache failure");

        assert_eq!(resolution.models.len(), 1);
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("cache read failed")));
        assert!(resolution
            .warnings
            .iter()
            .any(|warning| warning.contains("cache write failed")));
    });
}

#[test]
fn resolve_models_sorts_deterministically_by_model_id() {
    block_on(async {
        let cache = sqlite_cache();
        let manager = ModelManager::new(cache);
        let resolution = manager
            .resolve_provider_models(
                "openai",
                ResolveOptions {
                    static_models: vec![
                        parse_model(model_json("openai-responses", "zeta", "z", 1), "openai"),
                        parse_model(model_json("openai-responses", "alpha", "a", 1), "openai"),
                    ],
                    ..ResolveOptions::default()
                },
            )
            .await
            .expect("resolve models");

        let ids: Vec<String> = resolution
            .models
            .iter()
            .map(|model| model.model_id.clone())
            .collect();
        assert_eq!(ids, vec!["alpha".to_string(), "zeta".to_string()]);
    });
}
