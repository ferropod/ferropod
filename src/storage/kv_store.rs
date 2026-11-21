use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use tracing::info;
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};

use crate::core::model::{KvKeyMeta, KvValue, Result};
use crate::core::traits::KvStore;
use crate::storage::core_store::{escape_like, opt_i64, opt_text, opt_text_param};
use crate::storage::tx::{
    checkpoint_db, connect_db, copy_db_with_sidecars, map_turso_err, open_turso_db, with_retry,
};

const KV_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS kv_entries (
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    metadata TEXT,
    expires_at INTEGER,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (namespace, key)
);
CREATE INDEX IF NOT EXISTS idx_kv_expires ON kv_entries (expires_at);
"#;

/// Rows copied per transaction when migrating legacy `core.db` KV entries.
const LEGACY_MIGRATION_BATCH: i64 = 500;

#[derive(Debug, Clone)]
pub(crate) struct EphemeralEntry {
    pub(crate) value: Vec<u8>,
    pub(crate) metadata: Option<String>,
    pub(crate) expires_at: Option<i64>,
    pub(crate) updated_at: i64,
}

/// Dedicated Turso store for user KV data (`kv.db`), isolated from control-plane metadata in
/// `core.db` so high-frequency user writes never contend with routing/token/deployment writes.
/// Namespace ownership (`kv_namespaces`) stays in `core.db` via `RegistryStore`.
#[derive(Clone)]
pub struct TursoKvStore {
    db: Database,
    db_path: PathBuf,
    ephemeral_kv: Arc<tokio::sync::RwLock<HashMap<(String, String), EphemeralEntry>>>,
    max_ephemeral_entries: usize,
}

impl TursoKvStore {
    pub async fn open(path: &Path, max_ephemeral_entries: usize) -> Result<Self> {
        let db = open_turso_db(path).await?;
        let conn = connect_db(&db)?;
        conn.execute_batch(KV_SCHEMA).await.map_err(map_turso_err)?;
        Ok(Self {
            db,
            db_path: path.to_path_buf(),
            ephemeral_kv: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            max_ephemeral_entries: max_ephemeral_entries.max(64),
        })
    }

    /// One-time migration of KV entries that older versions stored inside `core.db`.
    ///
    /// Copies rows in batches (upsert, so it is idempotent and crash-safe), deletes each copied
    /// batch from `core.db`, and finally drops the legacy table. Returns the number of rows moved.
    pub async fn migrate_from_legacy_core(&self, core: &Database) -> Result<u64> {
        let core_conn = connect_db(core)?;
        let mut exists = core_conn
            .query(
                "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = 'kv_entries'",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        if exists.next().await.map_err(map_turso_err)?.is_none() {
            return Ok(0);
        }
        drop(exists);

        let mut moved = 0u64;
        loop {
            let mut rows = core_conn
                .query(
                    "SELECT namespace, key, value, metadata, expires_at, updated_at
                     FROM kv_entries ORDER BY namespace, key LIMIT ?1",
                    [TursoValue::Integer(LEGACY_MIGRATION_BATCH)],
                )
                .await
                .map_err(map_turso_err)?;
            let mut batch: Vec<Vec<TursoValue>> = Vec::new();
            while let Some(row) = rows.next().await.map_err(map_turso_err)? {
                let mut vals = Vec::with_capacity(6);
                for i in 0..6 {
                    vals.push(row.get_value(i).map_err(map_turso_err)?);
                }
                batch.push(vals);
            }
            drop(rows);
            if batch.is_empty() {
                break;
            }

            let kv_conn = connect_db(&self.db)?;
            kv_conn.execute("BEGIN", ()).await.map_err(map_turso_err)?;
            for vals in &batch {
                if let Err(e) = kv_conn
                    .execute(
                        "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(namespace, key) DO UPDATE SET
                            value = excluded.value,
                            metadata = excluded.metadata,
                            expires_at = excluded.expires_at,
                            updated_at = excluded.updated_at
                         WHERE excluded.updated_at >= kv_entries.updated_at",
                        vals.clone(),
                    )
                    .await
                {
                    let _ = kv_conn.execute("ROLLBACK", ()).await;
                    return Err(map_turso_err(e));
                }
            }
            kv_conn.execute("COMMIT", ()).await.map_err(map_turso_err)?;

            core_conn.execute("BEGIN", ()).await.map_err(map_turso_err)?;
            for vals in &batch {
                if let Err(e) = core_conn
                    .execute(
                        "DELETE FROM kv_entries WHERE namespace = ?1 AND key = ?2",
                        vec![vals[0].clone(), vals[1].clone()],
                    )
                    .await
                {
                    let _ = core_conn.execute("ROLLBACK", ()).await;
                    return Err(map_turso_err(e));
                }
            }
            core_conn.execute("COMMIT", ()).await.map_err(map_turso_err)?;
            moved += batch.len() as u64;
        }

        core_conn
            .execute("DROP INDEX IF EXISTS idx_kv_expires", ())
            .await
            .map_err(map_turso_err)?;
        core_conn
            .execute("DROP TABLE IF EXISTS kv_entries", ())
            .await
            .map_err(map_turso_err)?;
        if moved > 0 {
            info!(rows = moved, "migrated legacy KV entries from core.db to kv.db");
        }
        Ok(moved)
    }
}

#[async_trait]
impl KvStore for TursoKvStore {
    async fn get(&self, ns: &str, key: &str) -> Result<Option<KvValue>> {
        let now = Utc::now().timestamp();
        {
            let map = self.ephemeral_kv.read().await;
            if let Some(entry) = map.get(&(ns.to_string(), key.to_string())) {
                if entry.expires_at.map(|e| e > now).unwrap_or(true) {
                    return Ok(Some(KvValue {
                        value: entry.value.clone(),
                        metadata: entry.metadata.clone(),
                        expires_at: entry.expires_at,
                        updated_at: entry.updated_at,
                    }));
                }
            }
        }

        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT value, metadata, expires_at, updated_at FROM kv_entries
                 WHERE namespace = ?1 AND key = ?2 AND (expires_at IS NULL OR expires_at > ?3)",
                (ns, key, now),
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(KvValue {
                value: row.get(0).map_err(map_turso_err)?,
                metadata: opt_text(&row, 1)?,
                expires_at: opt_i64(&row, 2)?,
                updated_at: row.get(3).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn put(
        &self,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
        ephemeral: bool,
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        let expires_at = ttl_secs.map(|t| now + t as i64);

        if ephemeral {
            let mut map = self.ephemeral_kv.write().await;
            let k = (ns.to_string(), key.to_string());
            if !map.contains_key(&k) && map.len() >= self.max_ephemeral_entries {
                map.retain(|_, v| v.expires_at.map(|e| e > now).unwrap_or(true));
                if map.len() >= self.max_ephemeral_entries {
                    if let Some(oldest_key) = map
                        .iter()
                        .min_by_key(|(_, v)| v.updated_at)
                        .map(|(k, _)| k.clone())
                    {
                        map.remove(&oldest_key);
                    }
                }
            }
            map.insert(
                k,
                EphemeralEntry {
                    value: val.to_vec(),
                    metadata,
                    expires_at,
                    updated_at: now,
                },
            );
            return Ok(());
        }

        // Remove any matching ephemeral key so persistent writes are never shadowed.
        {
            let mut map = self.ephemeral_kv.write().await;
            map.remove(&(ns.to_string(), key.to_string()));
        }

        let ns_s = ns.to_string();
        let key_s = key.to_string();
        let val_v = val.to_vec();
        with_retry(|| {
            let db = self.db.clone();
            let ns_s = ns_s.clone();
            let key_s = key_s.clone();
            let val_v = val_v.clone();
            let metadata = metadata.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(namespace, key) DO UPDATE SET
                        value = excluded.value,
                        metadata = excluded.metadata,
                        expires_at = excluded.expires_at,
                        updated_at = excluded.updated_at",
                    vec![
                        TursoValue::Text(ns_s),
                        TursoValue::Text(key_s),
                        TursoValue::Blob(val_v),
                        opt_text_param(metadata),
                        expires_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(now),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn put_if_absent(
        &self,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
    ) -> Result<bool> {
        let now = Utc::now().timestamp();
        {
            let map = self.ephemeral_kv.read().await;
            if let Some(entry) = map.get(&(ns.to_string(), key.to_string())) {
                if entry.expires_at.map(|e| e > now).unwrap_or(true) {
                    return Ok(false);
                }
            }
        }

        let expires_at = ttl_secs.map(|t| now + t as i64);
        let ns_s = ns.to_string();
        let key_s = key.to_string();
        let val_v = val.to_vec();
        let inserted = with_retry(|| {
            let db = self.db.clone();
            let ns_s = ns_s.clone();
            let key_s = key_s.clone();
            let val_v = val_v.clone();
            let metadata = metadata.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT expires_at FROM kv_entries WHERE namespace = ?1 AND key = ?2",
                        (ns_s.as_str(), key_s.as_str()),
                    )
                    .await?;
                let exists_valid = if let Some(row) = rows.next().await? {
                    match row.get_value(0)? {
                        TursoValue::Null => true,
                        TursoValue::Integer(exp) => exp > now,
                        _ => true,
                    }
                } else {
                    false
                };
                drop(rows);

                if exists_valid {
                    tx.commit().await?;
                    return Ok(false);
                }

                tx.execute(
                    "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT(namespace, key) DO UPDATE SET
                        value = excluded.value,
                        metadata = excluded.metadata,
                        expires_at = excluded.expires_at,
                        updated_at = excluded.updated_at",
                    vec![
                        TursoValue::Text(ns_s),
                        TursoValue::Text(key_s),
                        TursoValue::Blob(val_v),
                        opt_text_param(metadata),
                        expires_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(now),
                    ],
                )
                .await?;
                tx.commit().await?;
                Ok(true)
            }
        })
        .await?;

        if inserted {
            let mut map = self.ephemeral_kv.write().await;
            map.remove(&(ns.to_string(), key.to_string()));
        }
        Ok(inserted)
    }

    async fn incr(&self, ns: &str, key: &str, delta: i64, ttl_secs: Option<u64>) -> Result<i64> {
        let now = Utc::now().timestamp();
        let expires_at = ttl_secs.map(|t| now + t as i64);

        // Remove any matching ephemeral key so persistent writes are never shadowed.
        {
            let mut map = self.ephemeral_kv.write().await;
            map.remove(&(ns.to_string(), key.to_string()));
        }

        let ns_s = ns.to_string();
        let key_s = key.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let ns_s = ns_s.clone();
            let key_s = key_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT value, expires_at FROM kv_entries WHERE namespace = ?1 AND key = ?2",
                        (ns_s.as_str(), key_s.as_str()),
                    )
                    .await?;
                let (current, reset_expiry) = if let Some(row) = rows.next().await? {
                    let expired = match row.get_value(1)? {
                        TursoValue::Integer(exp) => exp <= now,
                        _ => false,
                    };
                    if expired {
                        (0, true)
                    } else {
                        let bytes: Vec<u8> = row.get(0)?;
                        let s = std::str::from_utf8(&bytes).map_err(|_| {
                            turso::Error::ConversionFailure("kv value is not utf8 integer".into())
                        })?;
                        let v = s.trim().parse::<i64>().map_err(|_| {
                            turso::Error::ConversionFailure("kv value is not a valid i64".into())
                        })?;
                        (v, false)
                    }
                } else {
                    (0, true)
                };
                drop(rows);

                let next = current.saturating_add(delta);
                let next_bytes = next.to_string().into_bytes();
                let upsert_sql = if reset_expiry {
                    "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                     VALUES (?1, ?2, ?3, NULL, ?4, ?5)
                     ON CONFLICT(namespace, key) DO UPDATE SET
                        value = excluded.value,
                        metadata = NULL,
                        expires_at = excluded.expires_at,
                        updated_at = excluded.updated_at"
                } else {
                    "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                     VALUES (?1, ?2, ?3, NULL, ?4, ?5)
                     ON CONFLICT(namespace, key) DO UPDATE SET
                        value = excluded.value,
                        expires_at = COALESCE(excluded.expires_at, kv_entries.expires_at),
                        updated_at = excluded.updated_at"
                };

                tx.execute(
                    upsert_sql,
                    vec![
                        TursoValue::Text(ns_s),
                        TursoValue::Text(key_s),
                        TursoValue::Blob(next_bytes),
                        expires_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(now),
                    ],
                )
                .await?;
                tx.commit().await?;
                Ok(next)
            }
        })
        .await
    }

    async fn delete(&self, ns: &str, key: &str) -> Result<()> {
        {
            let mut map = self.ephemeral_kv.write().await;
            map.remove(&(ns.to_string(), key.to_string()));
        }
        let ns_s = ns.to_string();
        let key_s = key.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let ns_s = ns_s.clone();
            let key_s = key_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "DELETE FROM kv_entries WHERE namespace = ?1 AND key = ?2",
                    (ns_s, key_s),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn delete_namespace(&self, ns: &str) -> Result<()> {
        {
            let mut map = self.ephemeral_kv.write().await;
            map.retain(|(n, _), _| n != ns);
        }
        let ns_s = ns.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let ns_s = ns_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM kv_entries WHERE namespace = ?1", [ns_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn list(&self, ns: &str, prefix: Option<&str>, limit: usize) -> Result<Vec<KvKeyMeta>> {
        let (items, _) = self.list_page(ns, prefix, None, limit).await?;
        Ok(items)
    }

    async fn list_page(
        &self,
        ns: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<KvKeyMeta>, Option<String>)> {
        let now = Utc::now().timestamp();
        let conn = connect_db(&self.db)?;
        let page_limit = limit.clamp(1, 1000);
        let fetch_limit = (page_limit + 1) as i64;
        let cursor_opt = cursor.filter(|c| !c.is_empty());

        let mut rows = match (prefix, cursor_opt) {
            (Some(pfx), Some(cur)) => {
                let pattern = format!("{}%", escape_like(pfx));
                conn.query(
                    "SELECT key, LENGTH(value), metadata, expires_at, updated_at FROM kv_entries
                     WHERE namespace = ?1 AND (expires_at IS NULL OR expires_at > ?2)
                       AND key LIKE ?3 ESCAPE '\\' AND key > ?4
                     ORDER BY key ASC LIMIT ?5",
                    (ns, now, pattern, cur, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (Some(pfx), None) => {
                let pattern = format!("{}%", escape_like(pfx));
                conn.query(
                    "SELECT key, LENGTH(value), metadata, expires_at, updated_at FROM kv_entries
                     WHERE namespace = ?1 AND (expires_at IS NULL OR expires_at > ?2)
                       AND key LIKE ?3 ESCAPE '\\'
                     ORDER BY key ASC LIMIT ?4",
                    (ns, now, pattern, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (None, Some(cur)) => {
                conn.query(
                    "SELECT key, LENGTH(value), metadata, expires_at, updated_at FROM kv_entries
                     WHERE namespace = ?1 AND (expires_at IS NULL OR expires_at > ?2)
                       AND key > ?3
                     ORDER BY key ASC LIMIT ?4",
                    (ns, now, cur, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (None, None) => {
                conn.query(
                    "SELECT key, LENGTH(value), metadata, expires_at, updated_at FROM kv_entries
                     WHERE namespace = ?1 AND (expires_at IS NULL OR expires_at > ?2)
                     ORDER BY key ASC LIMIT ?3",
                    (ns, now, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
        };

        let mut merged: HashMap<String, KvKeyMeta> = HashMap::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let key: String = row.get(0).map_err(map_turso_err)?;
            merged.insert(
                key.clone(),
                KvKeyMeta {
                    key,
                    size: row.get::<i64>(1).map_err(map_turso_err)? as usize,
                    metadata: opt_text(&row, 2)?,
                    expires_at: opt_i64(&row, 3)?,
                    updated_at: row.get(4).map_err(map_turso_err)?,
                },
            );
        }

        {
            let map = self.ephemeral_kv.read().await;
            for ((entry_ns, entry_key), entry) in map.iter() {
                if entry_ns != ns {
                    continue;
                }
                if !entry.expires_at.map(|e| e > now).unwrap_or(true) {
                    continue;
                }
                if let Some(pfx) = prefix {
                    if !entry_key.starts_with(pfx) {
                        continue;
                    }
                }
                if let Some(cur) = cursor_opt {
                    if entry_key.as_str() <= cur {
                        continue;
                    }
                }
                merged.insert(
                    entry_key.clone(),
                    KvKeyMeta {
                        key: entry_key.clone(),
                        size: entry.value.len(),
                        metadata: entry.metadata.clone(),
                        expires_at: entry.expires_at,
                        updated_at: entry.updated_at,
                    },
                );
            }
        }

        let mut out: Vec<_> = merged.into_values().collect();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        let has_more = out.len() > page_limit;
        if has_more {
            out.truncate(page_limit);
        }
        let next_cursor = if has_more {
            out.last().map(|k| k.key.clone())
        } else {
            None
        };
        Ok((out, next_cursor))
    }

    async fn cleanup_expired_batch(&self, batch_limit: usize) -> Result<u64> {
        let now = Utc::now().timestamp();
        {
            let mut map = self.ephemeral_kv.write().await;
            map.retain(|_, v| v.expires_at.map(|e| e > now).unwrap_or(true));
        }

        with_retry(|| {
            let db = self.db.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT namespace, key FROM kv_entries WHERE expires_at IS NOT NULL AND expires_at <= ?1 LIMIT ?2",
                        (now, batch_limit as i64),
                    )
                    .await?;
                let mut targets = Vec::new();
                while let Some(row) = rows.next().await? {
                    let ns: String = row.get(0)?;
                    let k: String = row.get(1)?;
                    targets.push((ns, k));
                }
                drop(rows);

                let mut deleted = 0u64;
                for (ns, k) in targets {
                    deleted += tx
                        .execute(
                            "DELETE FROM kv_entries WHERE namespace = ?1 AND key = ?2 AND expires_at <= ?3",
                            (ns, k, now),
                        )
                        .await?;
                }
                tx.commit().await?;
                Ok(deleted)
            }
        })
        .await
    }

    async fn checkpoint(&self) -> Result<()> {
        checkpoint_db(&self.db).await;
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        checkpoint_db(&self.db).await;
        copy_db_with_sidecars(&self.db_path, dest_path).await
    }
}
