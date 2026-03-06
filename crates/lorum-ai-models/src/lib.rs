use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lorum_ai_contract::ApiKind;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const CACHE_VERSION: i64 = 1;
const DEFAULT_NON_AUTHORITATIVE_RETRY_BACKOFF_SECS: i64 = 60;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ModelError {
    #[error("database error: {0}")]
    Database(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("model source error: {0}")]
    Source(String),
    #[error("internal synchronization error")]
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub api: ApiKind,
    pub model_id: String,
    pub display_name: Option<String>,
    pub context_window: Option<u64>,
    pub supports_tools: Option<bool>,
    pub stale: Option<bool>,
}

impl ModelInfo {
    pub fn validate(&self) -> bool {
        !self.provider.trim().is_empty() && !self.model_id.trim().is_empty()
    }

    pub fn merge_over(&self, base: &ModelInfo) -> ModelInfo {
        ModelInfo {
            provider: self.provider.clone(),
            api: self.api,
            model_id: self.model_id.clone(),
            display_name: self
                .display_name
                .clone()
                .or_else(|| base.display_name.clone()),
            context_window: self.context_window.or(base.context_window),
            supports_tools: self.supports_tools.or(base.supports_tools),
            stale: self.stale.or(base.stale),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceModels {
    pub models: Vec<Value>,
    pub authoritative: bool,
}

#[async_trait]
pub trait ModelSource: Send + Sync {
    async fn fetch_models(&self, provider_id: &str) -> Result<SourceModels, ModelError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCacheEntry {
    pub provider_id: String,
    pub version: i64,
    pub updated_at_unix: i64,
    pub authoritative: bool,
    pub models: Vec<ModelInfo>,
}

#[async_trait]
pub trait ModelCacheStore: Send + Sync {
    async fn get(&self, provider_id: &str) -> Result<Option<ModelCacheEntry>, ModelError>;
    async fn put(&self, entry: &ModelCacheEntry) -> Result<(), ModelError>;
}

pub struct SqliteModelCache {
    conn: Mutex<Connection>,
}

impl SqliteModelCache {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ModelError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| ModelError::Database(format!("create cache dir failed: {err}")))?;
        }

        let conn = Connection::open(path)
            .map_err(|err| ModelError::Database(format!("open cache db failed: {err}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| ModelError::Database(format!("set wal failed: {err}")))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|err| ModelError::Database(format!("set busy timeout failed: {err}")))?;

        let cache = Self {
            conn: Mutex::new(conn),
        };
        cache.init_schema()?;

        Ok(cache)
    }

    fn init_schema(&self) -> Result<(), ModelError> {
        let conn = self.conn.lock().map_err(|_| ModelError::Internal)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS model_cache (
                provider_id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                updated_at_unix INTEGER NOT NULL,
                authoritative INTEGER NOT NULL,
                models_json TEXT NOT NULL
            );
            "#,
        )
        .map_err(|err| ModelError::Database(format!("init model cache schema failed: {err}")))?;
        Ok(())
    }
}

#[async_trait]
impl ModelCacheStore for SqliteModelCache {
    async fn get(&self, provider_id: &str) -> Result<Option<ModelCacheEntry>, ModelError> {
        let conn = self.conn.lock().map_err(|_| ModelError::Internal)?;
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, version, updated_at_unix, authoritative, models_json \
                 FROM model_cache WHERE provider_id = ?1",
            )
            .map_err(|err| ModelError::Database(format!("prepare get cache failed: {err}")))?;

        let mut rows = stmt
            .query(params![provider_id])
            .map_err(|err| ModelError::Database(format!("query get cache failed: {err}")))?;

        let Some(row) = rows
            .next()
            .map_err(|err| ModelError::Database(format!("cache next failed: {err}")))?
        else {
            return Ok(None);
        };

        let models_raw: String = row
            .get(4)
            .map_err(|err| ModelError::Database(format!("decode models json failed: {err}")))?;
        let models: Vec<ModelInfo> = serde_json::from_str(&models_raw).map_err(|err| {
            ModelError::Serialization(format!("parse cache models failed: {err}"))
        })?;

        Ok(Some(ModelCacheEntry {
            provider_id: row
                .get(0)
                .map_err(|err| ModelError::Database(format!("decode provider id failed: {err}")))?,
            version: row
                .get(1)
                .map_err(|err| ModelError::Database(format!("decode version failed: {err}")))?,
            updated_at_unix: row
                .get(2)
                .map_err(|err| ModelError::Database(format!("decode updated failed: {err}")))?,
            authoritative: row.get::<_, i64>(3).map_err(|err| {
                ModelError::Database(format!("decode authoritative failed: {err}"))
            })? != 0,
            models,
        }))
    }

    async fn put(&self, entry: &ModelCacheEntry) -> Result<(), ModelError> {
        let conn = self.conn.lock().map_err(|_| ModelError::Internal)?;
        let models_json = serde_json::to_string(&entry.models)
            .map_err(|err| ModelError::Serialization(format!("encode models failed: {err}")))?;

        conn.execute(
            "INSERT INTO model_cache(provider_id, version, updated_at_unix, authoritative, models_json) \
             VALUES(?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(provider_id) DO UPDATE SET \
                version = excluded.version, \
                updated_at_unix = excluded.updated_at_unix, \
                authoritative = excluded.authoritative, \
                models_json = excluded.models_json",
            params![
                entry.provider_id,
                entry.version,
                entry.updated_at_unix,
                if entry.authoritative { 1 } else { 0 },
                models_json,
            ],
        )
        .map_err(|err| ModelError::Database(format!("upsert model cache failed: {err}")))?;

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderDescriptor {
    pub provider_id: String,
    pub default_model: String,
    pub default_api: ApiKind,
    pub allow_unauthenticated_discovery: bool,
    pub catalog_metadata: Option<String>,
}

#[derive(Default)]
pub struct ProviderDescriptorRegistry {
    descriptors: Mutex<HashMap<String, ProviderDescriptor>>,
}

impl ProviderDescriptorRegistry {
    pub fn register(&self, descriptor: ProviderDescriptor) -> Result<(), ModelError> {
        let mut guard = self.descriptors.lock().map_err(|_| ModelError::Internal)?;
        guard.insert(descriptor.provider_id.clone(), descriptor);
        Ok(())
    }

    pub fn get(&self, provider_id: &str) -> Result<Option<ProviderDescriptor>, ModelError> {
        let guard = self.descriptors.lock().map_err(|_| ModelError::Internal)?;
        Ok(guard.get(provider_id).cloned())
    }

    pub fn default_models(&self) -> Result<HashMap<String, String>, ModelError> {
        let guard = self.descriptors.lock().map_err(|_| ModelError::Internal)?;
        let mut map = HashMap::new();
        for (provider_id, descriptor) in guard.iter() {
            map.insert(provider_id.clone(), descriptor.default_model.clone());
        }
        Ok(map)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    pub static_models: Vec<ModelInfo>,
    pub use_models_dev: bool,
    pub use_dynamic: bool,
    pub now_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelResolution {
    pub provider_id: String,
    pub models: Vec<ModelInfo>,
    pub cache_authoritative: Option<bool>,
    pub warnings: Vec<String>,
}

pub struct ModelManager {
    cache: Arc<dyn ModelCacheStore>,
    models_dev_source: Option<Arc<dyn ModelSource>>,
    dynamic_source: Option<Arc<dyn ModelSource>>,
    retry_backoff_secs: i64,
    last_non_authoritative_retry_unix: Mutex<HashMap<String, i64>>,
}

impl ModelManager {
    pub fn new(cache: Arc<dyn ModelCacheStore>) -> Self {
        Self {
            cache,
            models_dev_source: None,
            dynamic_source: None,
            retry_backoff_secs: DEFAULT_NON_AUTHORITATIVE_RETRY_BACKOFF_SECS,
            last_non_authoritative_retry_unix: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_models_dev_source(mut self, source: Arc<dyn ModelSource>) -> Self {
        self.models_dev_source = Some(source);
        self
    }

    pub fn with_dynamic_source(mut self, source: Arc<dyn ModelSource>) -> Self {
        self.dynamic_source = Some(source);
        self
    }

    pub fn set_non_authoritative_retry_backoff_secs(&mut self, seconds: i64) {
        self.retry_backoff_secs = seconds.max(1);
    }

    pub async fn resolve_provider_models(
        &self,
        provider_id: &str,
        options: ResolveOptions,
    ) -> Result<ModelResolution, ModelError> {
        let now_unix = options.now_unix.unwrap_or_else(unix_now);
        let mut warnings = Vec::new();
        let mut map = HashMap::<String, ModelInfo>::new();

        merge_models(&mut map, options.static_models);

        if options.use_models_dev {
            if let Some(source) = &self.models_dev_source {
                match source.fetch_models(provider_id).await {
                    Ok(source_models) => {
                        let parsed = parse_source_models(
                            source_models.models,
                            provider_id,
                            "models.dev",
                            &mut warnings,
                        );
                        merge_models(&mut map, parsed);
                    }
                    Err(err) => warnings.push(format!("models.dev fetch failed: {err}")),
                }
            }
        }

        let cache_entry = match self.cache.get(provider_id).await {
            Ok(entry) => entry,
            Err(err) => {
                warnings.push(format!("cache read failed: {err}"));
                None
            }
        };

        if let Some(entry) = &cache_entry {
            merge_models(&mut map, entry.models.clone());
        }

        if options.use_dynamic {
            if let Some(source) = &self.dynamic_source {
                let dynamic_allowed =
                    self.can_attempt_dynamic(provider_id, now_unix, cache_entry.as_ref())?;

                if dynamic_allowed {
                    match source.fetch_models(provider_id).await {
                        Ok(source_models) => {
                            let parsed = parse_source_models(
                                source_models.models.clone(),
                                provider_id,
                                "dynamic",
                                &mut warnings,
                            );
                            merge_models(&mut map, parsed.clone());

                            let cache_models = map.values().cloned().collect::<Vec<_>>();
                            let new_cache = ModelCacheEntry {
                                provider_id: provider_id.to_string(),
                                version: CACHE_VERSION,
                                updated_at_unix: now_unix,
                                authoritative: source_models.authoritative,
                                models: cache_models,
                            };
                            if let Err(err) = self.cache.put(&new_cache).await {
                                warnings.push(format!("cache write failed: {err}"));
                            }
                        }
                        Err(err) => {
                            warnings.push(format!("dynamic fetch failed: {err}"));
                            if cache_entry
                                .as_ref()
                                .is_some_and(|entry| !entry.authoritative)
                            {
                                let mut guard = self
                                    .last_non_authoritative_retry_unix
                                    .lock()
                                    .map_err(|_| ModelError::Internal)?;
                                guard.insert(provider_id.to_string(), now_unix);
                            }
                        }
                    }
                } else {
                    warnings.push(
                        "dynamic fetch skipped due to non-authoritative retry backoff".to_string(),
                    );
                }
            }
        }

        let mut models: Vec<ModelInfo> = map.into_values().collect();
        models.sort_by(|a, b| a.model_id.cmp(&b.model_id));

        Ok(ModelResolution {
            provider_id: provider_id.to_string(),
            models,
            cache_authoritative: cache_entry.map(|entry| entry.authoritative),
            warnings,
        })
    }

    fn can_attempt_dynamic(
        &self,
        provider_id: &str,
        now_unix: i64,
        cache_entry: Option<&ModelCacheEntry>,
    ) -> Result<bool, ModelError> {
        if cache_entry.is_none_or(|entry| entry.authoritative) {
            return Ok(true);
        }

        let guard = self
            .last_non_authoritative_retry_unix
            .lock()
            .map_err(|_| ModelError::Internal)?;
        let last_retry = guard.get(provider_id).copied().unwrap_or(0);
        Ok(now_unix - last_retry >= self.retry_backoff_secs)
    }
}

fn merge_models(map: &mut HashMap<String, ModelInfo>, incoming: Vec<ModelInfo>) {
    for model in incoming {
        if !model.validate() {
            continue;
        }
        match map.get(&model.model_id) {
            Some(existing) => {
                map.insert(model.model_id.clone(), model.merge_over(existing));
            }
            None => {
                map.insert(model.model_id.clone(), model);
            }
        }
    }
}

fn parse_source_models(
    source_models: Vec<Value>,
    provider_id: &str,
    source_label: &str,
    warnings: &mut Vec<String>,
) -> Vec<ModelInfo> {
    let mut parsed = Vec::new();

    for value in source_models {
        match parse_model_like(&value, provider_id) {
            Some(model) => parsed.push(model),
            None => warnings.push(format!("{source_label} dropped malformed model entry")),
        }
    }

    parsed
}

pub fn parse_model_like(value: &Value, fallback_provider: &str) -> Option<ModelInfo> {
    let object = value.as_object()?;

    let provider = object
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or(fallback_provider)
        .to_string();

    let api_raw = object.get("api")?.as_str()?;
    let api = ApiKind::from_str(api_raw).ok()?;

    let model_id = object
        .get("model_id")
        .and_then(Value::as_str)
        .or_else(|| object.get("model").and_then(Value::as_str))
        .or_else(|| object.get("id").and_then(Value::as_str))?
        .to_string();

    let display_name = object
        .get("display_name")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            object
                .get("name")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        });

    let context_window = object
        .get("context_window")
        .and_then(Value::as_u64)
        .or_else(|| object.get("max_tokens").and_then(Value::as_u64));

    let supports_tools = object
        .get("supports_tools")
        .and_then(Value::as_bool)
        .or_else(|| object.get("tool_use").and_then(Value::as_bool));

    let stale = object.get("stale").and_then(Value::as_bool);

    Some(ModelInfo {
        provider,
        api,
        model_id,
        display_name,
        context_window,
        supports_tools,
        stale,
    })
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs() as i64
}
