use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use rusqlite::{params, Connection};

use crate::{
    unix_now, AuthError, CredentialData, CredentialKind, CredentialRecord, CredentialUsage,
    USAGE_KEY_PREFIX,
};

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn list_credentials(&self, provider: &str) -> Result<Vec<CredentialRecord>, AuthError>;

    async fn upsert(&self, record: &CredentialRecord) -> Result<(), AuthError>;

    async fn disable(&self, credential_id: &str) -> Result<(), AuthError>;

    async fn get_credential(
        &self,
        credential_id: &str,
    ) -> Result<Option<CredentialRecord>, AuthError>;

    async fn put_usage(
        &self,
        provider: &str,
        credential_id: &str,
        usage: &CredentialUsage,
    ) -> Result<(), AuthError>;

    async fn list_usage(
        &self,
        provider: &str,
    ) -> Result<HashMap<String, CredentialUsage>, AuthError>;
}

pub struct SqliteCredentialStore {
    conn: Mutex<Connection>,
}

impl SqliteCredentialStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AuthError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| AuthError::Database(format!("create dir failed: {err}")))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
            }
        }

        let conn = Connection::open(path)
            .map_err(|err| AuthError::Database(format!("open sqlite failed: {err}")))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| AuthError::Database(format!("set wal failed: {err}")))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|err| AuthError::Database(format!("set busy timeout failed: {err}")))?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.init_schema()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
        }

        Ok(store)
    }

    fn init_schema(&self) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS auth_credentials (
                credential_id TEXT PRIMARY KEY,
                provider TEXT NOT NULL,
                kind TEXT NOT NULL,
                disabled INTEGER NOT NULL DEFAULT 0,
                data TEXT NOT NULL,
                created_at_unix INTEGER NOT NULL,
                updated_at_unix INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_auth_credentials_provider
            ON auth_credentials(provider);

            CREATE TABLE IF NOT EXISTS cache (
                cache_key TEXT PRIMARY KEY,
                cache_value TEXT NOT NULL,
                updated_at_unix INTEGER NOT NULL
            );
            "#,
        )
        .map_err(|err| AuthError::Database(format!("init schema failed: {err}")))?;
        Ok(())
    }

    fn usage_key(provider: &str, credential_id: &str) -> String {
        format!("{USAGE_KEY_PREFIX}::{provider}::{credential_id}")
    }
}

#[async_trait]
impl CredentialStore for SqliteCredentialStore {
    async fn list_credentials(&self, provider: &str) -> Result<Vec<CredentialRecord>, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        let mut stmt = conn
            .prepare(
                "SELECT credential_id, provider, kind, disabled, data, created_at_unix, updated_at_unix \
                 FROM auth_credentials WHERE provider = ?1 ORDER BY created_at_unix ASC",
            )
            .map_err(|err| AuthError::Database(format!("prepare list credentials failed: {err}")))?;

        let rows = stmt
            .query_map(params![provider], |row| {
                let kind_raw: String = row.get(2)?;
                let kind = CredentialKind::parse(&kind_raw).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(std::fmt::Error),
                    )
                })?;
                let data_raw: String = row.get(4)?;
                let data: CredentialData = serde_json::from_str(&data_raw).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(std::fmt::Error),
                    )
                })?;

                Ok(CredentialRecord {
                    credential_id: row.get(0)?,
                    provider: row.get(1)?,
                    kind,
                    disabled: row.get::<_, i64>(3)? != 0,
                    data,
                    created_at_unix: row.get(5)?,
                    updated_at_unix: row.get(6)?,
                })
            })
            .map_err(|err| AuthError::Database(format!("query list credentials failed: {err}")))?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|err| AuthError::Database(format!("row decode failed: {err}")))?);
        }
        Ok(out)
    }

    async fn upsert(&self, record: &CredentialRecord) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        let data = serde_json::to_string(&record.data).map_err(|err| {
            AuthError::Serialization(format!("encode credential data failed: {err}"))
        })?;

        conn.execute(
            "INSERT INTO auth_credentials \
             (credential_id, provider, kind, disabled, data, created_at_unix, updated_at_unix) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT(credential_id) DO UPDATE SET \
               provider = excluded.provider, \
               kind = excluded.kind, \
               disabled = excluded.disabled, \
               data = excluded.data, \
               updated_at_unix = excluded.updated_at_unix",
            params![
                record.credential_id,
                record.provider,
                record.kind.as_str(),
                if record.disabled { 1 } else { 0 },
                data,
                record.created_at_unix,
                record.updated_at_unix
            ],
        )
        .map_err(|err| AuthError::Database(format!("upsert credential failed: {err}")))?;

        Ok(())
    }

    async fn disable(&self, credential_id: &str) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        conn.execute(
            "UPDATE auth_credentials SET disabled = 1, updated_at_unix = ?2 WHERE credential_id = ?1",
            params![credential_id, unix_now()],
        )
        .map_err(|err| AuthError::Database(format!("disable credential failed: {err}")))?;
        Ok(())
    }

    async fn get_credential(
        &self,
        credential_id: &str,
    ) -> Result<Option<CredentialRecord>, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        let mut stmt = conn
            .prepare(
                "SELECT credential_id, provider, kind, disabled, data, created_at_unix, updated_at_unix \
                 FROM auth_credentials WHERE credential_id = ?1",
            )
            .map_err(|err| AuthError::Database(format!("prepare get credential failed: {err}")))?;

        let mut rows = stmt
            .query(params![credential_id])
            .map_err(|err| AuthError::Database(format!("query get credential failed: {err}")))?;

        let Some(row) = rows
            .next()
            .map_err(|err| AuthError::Database(format!("next get credential failed: {err}")))?
        else {
            return Ok(None);
        };

        let kind_raw: String = row
            .get(2)
            .map_err(|err| AuthError::Database(format!("kind decode failed: {err}")))?;
        let kind = CredentialKind::parse(&kind_raw)?;
        let data_raw: String = row
            .get(4)
            .map_err(|err| AuthError::Database(format!("data decode failed: {err}")))?;
        let data: CredentialData = serde_json::from_str(&data_raw).map_err(|err| {
            AuthError::Serialization(format!("decode credential data failed: {err}"))
        })?;

        Ok(Some(CredentialRecord {
            credential_id: row
                .get(0)
                .map_err(|err| AuthError::Database(format!("id decode failed: {err}")))?,
            provider: row
                .get(1)
                .map_err(|err| AuthError::Database(format!("provider decode failed: {err}")))?,
            kind,
            disabled: row
                .get::<_, i64>(3)
                .map_err(|err| AuthError::Database(format!("disabled decode failed: {err}")))?
                != 0,
            data,
            created_at_unix: row
                .get(5)
                .map_err(|err| AuthError::Database(format!("created decode failed: {err}")))?,
            updated_at_unix: row
                .get(6)
                .map_err(|err| AuthError::Database(format!("updated decode failed: {err}")))?,
        }))
    }

    async fn put_usage(
        &self,
        provider: &str,
        credential_id: &str,
        usage: &CredentialUsage,
    ) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        let cache_key = Self::usage_key(provider, credential_id);
        let cache_value = serde_json::to_string(usage)
            .map_err(|err| AuthError::Serialization(format!("encode usage failed: {err}")))?;

        conn.execute(
            "INSERT INTO cache (cache_key, cache_value, updated_at_unix) VALUES (?1, ?2, ?3) \
             ON CONFLICT(cache_key) DO UPDATE SET cache_value = excluded.cache_value, updated_at_unix = excluded.updated_at_unix",
            params![cache_key, cache_value, unix_now()],
        )
        .map_err(|err| AuthError::Database(format!("put usage failed: {err}")))?;

        Ok(())
    }

    async fn list_usage(
        &self,
        provider: &str,
    ) -> Result<HashMap<String, CredentialUsage>, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::Internal)?;
        let pattern = format!("{USAGE_KEY_PREFIX}::{provider}::%");
        let mut stmt = conn
            .prepare("SELECT cache_key, cache_value FROM cache WHERE cache_key LIKE ?1")
            .map_err(|err| AuthError::Database(format!("prepare list usage failed: {err}")))?;

        let rows = stmt
            .query_map(params![pattern], |row| {
                let key: String = row.get(0)?;
                let value_raw: String = row.get(1)?;
                Ok((key, value_raw))
            })
            .map_err(|err| AuthError::Database(format!("query list usage failed: {err}")))?;

        let mut out = HashMap::new();
        for row in rows {
            let (key, value_raw) =
                row.map_err(|err| AuthError::Database(format!("row usage decode failed: {err}")))?;
            let Some(credential_id) = key.rsplit("::").next() else {
                continue;
            };
            let usage: CredentialUsage = serde_json::from_str(&value_raw)
                .map_err(|err| AuthError::Serialization(format!("decode usage failed: {err}")))?;
            out.insert(credential_id.to_string(), usage);
        }
        Ok(out)
    }
}
