use async_trait::async_trait;
use chrono::Utc;
use turso::{transaction::TransactionBehavior, Value as TursoValue};

use crate::core::model::{BucketMeta, BucketOpts, FerroError, ObjectMeta, Result};
use crate::core::traits::BlobMetaStore;
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{escape_like, opt_i64, opt_text, opt_text_param, row_to_object_meta, TursoCoreStore};

const SELECT_BUCKET: &str =
    "SELECT name, project_id, public_read, created_at, resource_group, description, max_object_size_mb, quota_mb, allowed_content_types, cache_control, cors_origins FROM blob_buckets";

fn row_to_bucket(row: &turso::Row) -> Result<BucketMeta> {
    Ok(BucketMeta {
        name: row.get(0).map_err(map_turso_err)?,
        project_id: opt_text(row, 1)?,
        public_read: row.get::<i64>(2).map_err(map_turso_err)? != 0,
        created_at: row.get(3).map_err(map_turso_err)?,
        group: opt_text(row, 4)?,
        description: opt_text(row, 5)?,
        max_object_size_mb: opt_i64(row, 6)?.and_then(|v| usize::try_from(v).ok()),
        quota_mb: opt_i64(row, 7)?.and_then(|v| usize::try_from(v).ok()),
        allowed_content_types: opt_text(row, 8)?,
        cache_control: opt_text(row, 9)?,
        cors_origins: opt_text(row, 10)?,
    })
}

#[async_trait]
impl BlobMetaStore for TursoCoreStore {
    async fn create_bucket_with_opts(&self, name: &str, opts: BucketOpts) -> Result<BucketMeta> {
        let now = Utc::now().timestamp();
        let public_read = opts.public_read.unwrap_or(false);
        let description = opts
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let max_object_size_mb = opts.max_object_size_mb.filter(|v| *v > 0);
        let quota_mb = opts.quota_mb.filter(|v| *v > 0);
        let allowed_content_types = opts
            .allowed_content_types
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let cache_control = opts
            .cache_control
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let cors_origins = opts
            .cors_origins
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let meta = BucketMeta {
            name: name.to_string(),
            project_id: opts.project_id,
            public_read,
            created_at: now,
            group: opts.group,
            description,
            max_object_size_mb,
            quota_mb,
            allowed_content_types,
            cache_control,
            cors_origins,
        };

        let m = meta.clone();
        with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO blob_buckets (name, project_id, public_read, created_at, resource_group, description, max_object_size_mb, quota_mb, allowed_content_types, cache_control, cors_origins)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    vec![
                        TursoValue::Text(m.name),
                        opt_text_param(m.project_id),
                        TursoValue::Integer(if m.public_read { 1 } else { 0 }),
                        TursoValue::Integer(m.created_at),
                        opt_text_param(m.group),
                        opt_text_param(m.description),
                        m.max_object_size_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.quota_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        opt_text_param(m.allowed_content_types),
                        opt_text_param(m.cache_control),
                        opt_text_param(m.cors_origins),
                    ],
                )
                .await
            }
        })
        .await?;

        Ok(meta)
    }

    async fn update_bucket(&self, meta: &BucketMeta) -> Result<()> {
        let m = meta.clone();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE blob_buckets SET public_read = ?1, resource_group = ?2, description = ?3,
                     max_object_size_mb = ?4, quota_mb = ?5, allowed_content_types = ?6, cache_control = ?7, cors_origins = ?8
                     WHERE name = ?9",
                    vec![
                        TursoValue::Integer(if m.public_read { 1 } else { 0 }),
                        opt_text_param(m.group),
                        opt_text_param(m.description),
                        m.max_object_size_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.quota_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        opt_text_param(m.allowed_content_types),
                        opt_text_param(m.cache_control),
                        opt_text_param(m.cors_origins),
                        TursoValue::Text(m.name),
                    ],
                )
                .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(format!("bucket '{}'", meta.name)));
        }
        Ok(())
    }

    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_BUCKET} WHERE name = ?1"), [name])
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_bucket(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_buckets(&self) -> Result<Vec<BucketMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_BUCKET} ORDER BY name ASC"), ())
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_bucket(&row)?);
        }
        Ok(out)
    }

    async fn total_bucket_bytes(&self, bucket: &str) -> Result<u64> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT COALESCE(SUM(size), 0) FROM blob_objects WHERE bucket = ?1",
                [bucket],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let sum: i64 = row.get(0).map_err(map_turso_err)?;
            Ok(sum.max(0) as u64)
        } else {
            Ok(0)
        }
    }

    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()> {
        let pub_int: i64 = if public_read { 1 } else { 0 };
        let name_s = name.to_string();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let name_s = name_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE blob_buckets SET public_read = ?1 WHERE name = ?2",
                    (pub_int, name_s),
                )
                .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(format!("bucket '{name}'")));
        }
        Ok(())
    }

    async fn delete_bucket(&self, name: &str) -> Result<()> {
        let name_s = name.to_string();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let name_s = name_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM blob_buckets WHERE name = ?1", [name_s])
                    .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(format!("bucket '{name}'")));
        }
        Ok(())
    }

    async fn upsert_object_meta(&self, meta: ObjectMeta, gc_grace_secs: i64) -> Result<()> {
        let now = meta.updated_at;
        let delete_after = now + gc_grace_secs;
        with_retry(|| {
            let db = self.db.clone();
            let meta = meta.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;

                let mut prev_rows = tx
                    .query(
                        "SELECT blob_path FROM blob_objects WHERE bucket = ?1 AND key = ?2",
                        (meta.bucket.as_str(), meta.key.as_str()),
                    )
                    .await?;
                let old_path = match prev_rows.next().await? {
                    Some(r) => Some(r.get::<String>(0)?),
                    None => None,
                };
                drop(prev_rows);

                tx.execute(
                    "INSERT INTO blob_objects (bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(bucket, key) DO UPDATE SET
                        sha256 = excluded.sha256,
                        blob_path = excluded.blob_path,
                        size = excluded.size,
                        content_type = excluded.content_type,
                        custom_meta = excluded.custom_meta,
                        updated_at = excluded.updated_at",
                    vec![
                        TursoValue::Text(meta.bucket.clone()),
                        TursoValue::Text(meta.key.clone()),
                        TursoValue::Text(meta.sha256.clone()),
                        TursoValue::Text(meta.blob_path.clone()),
                        TursoValue::Integer(meta.size as i64),
                        TursoValue::Text(meta.content_type.clone()),
                        opt_text_param(meta.custom_meta.clone()),
                        TursoValue::Integer(meta.updated_at),
                    ],
                )
                .await?;

                // Ensure newly written blob_path is removed from pending GC if deduplicated.
                tx.execute(
                    "DELETE FROM blob_gc WHERE blob_path = ?1",
                    [meta.blob_path.as_str()],
                )
                .await?;

                if let Some(old) = old_path {
                    if old != meta.blob_path {
                        let mut cnt_rows = tx
                            .query(
                                "SELECT COUNT(*) FROM blob_objects WHERE blob_path = ?1",
                                [old.as_str()],
                            )
                            .await?;
                        let cnt = match cnt_rows.next().await? {
                            Some(r) => r.get::<i64>(0)?,
                            None => 0,
                        };
                        drop(cnt_rows);
                        if cnt == 0 {
                            tx.execute(
                                "INSERT OR IGNORE INTO blob_gc (blob_path, delete_after) VALUES (?1, ?2)",
                                (old, delete_after),
                            )
                            .await?;
                        }
                    }
                }

                tx.commit().await
            }
        })
        .await
    }

    async fn get_object_meta(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at
                 FROM blob_objects WHERE bucket = ?1 AND key = ?2",
                (bucket, key),
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_object_meta(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn delete_object_meta(
        &self,
        bucket: &str,
        key: &str,
        gc_grace_secs: i64,
    ) -> Result<bool> {
        let delete_after = Utc::now().timestamp() + gc_grace_secs;
        let bucket_s = bucket.to_string();
        let key_s = key.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let bucket_s = bucket_s.clone();
            let key_s = key_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT blob_path FROM blob_objects WHERE bucket = ?1 AND key = ?2",
                        (bucket_s.as_str(), key_s.as_str()),
                    )
                    .await?;
                let path_opt = match rows.next().await? {
                    Some(r) => Some(r.get::<String>(0)?),
                    None => None,
                };
                drop(rows);
                let Some(blob_path) = path_opt else {
                    tx.commit().await?;
                    return Ok(false);
                };

                tx.execute(
                    "DELETE FROM blob_objects WHERE bucket = ?1 AND key = ?2",
                    (bucket_s.as_str(), key_s.as_str()),
                )
                .await?;

                let mut cnt_rows = tx
                    .query(
                        "SELECT COUNT(*) FROM blob_objects WHERE blob_path = ?1",
                        [blob_path.as_str()],
                    )
                    .await?;
                let cnt = match cnt_rows.next().await? {
                    Some(r) => r.get::<i64>(0)?,
                    None => 0,
                };
                drop(cnt_rows);
                if cnt == 0 {
                    tx.execute(
                        "INSERT OR IGNORE INTO blob_gc (blob_path, delete_after) VALUES (?1, ?2)",
                        (blob_path, delete_after),
                    )
                    .await?;
                }
                tx.commit().await?;
                Ok(true)
            }
        })
        .await
    }

    async fn list_objects_meta(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectMeta>> {
        let (items, _) = self
            .list_objects_meta_page(bucket, prefix, None, limit)
            .await?;
        Ok(items)
    }

    async fn list_objects_meta_page(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ObjectMeta>, Option<String>)> {
        let conn = connect_db(&self.db)?;
        let page_limit = limit.clamp(1, 1000);
        let fetch_limit = (page_limit + 1) as i64;
        let cursor_opt = cursor.filter(|c| !c.is_empty());

        let mut rows = match (prefix, cursor_opt) {
            (Some(pfx), Some(cur)) => {
                let pattern = format!("{}%", escape_like(pfx));
                conn.query(
                    "SELECT bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at
                     FROM blob_objects WHERE bucket = ?1 AND key LIKE ?2 ESCAPE '\\' AND key > ?3
                     ORDER BY key ASC LIMIT ?4",
                    (bucket, pattern, cur, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (Some(pfx), None) => {
                let pattern = format!("{}%", escape_like(pfx));
                conn.query(
                    "SELECT bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at
                     FROM blob_objects WHERE bucket = ?1 AND key LIKE ?2 ESCAPE '\\'
                     ORDER BY key ASC LIMIT ?3",
                    (bucket, pattern, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (None, Some(cur)) => {
                conn.query(
                    "SELECT bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at
                     FROM blob_objects WHERE bucket = ?1 AND key > ?2
                     ORDER BY key ASC LIMIT ?3",
                    (bucket, cur, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
            (None, None) => {
                conn.query(
                    "SELECT bucket, key, sha256, blob_path, size, content_type, custom_meta, updated_at
                     FROM blob_objects WHERE bucket = ?1 ORDER BY key ASC LIMIT ?2",
                    (bucket, fetch_limit),
                )
                .await
                .map_err(map_turso_err)?
            }
        };
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_object_meta(&row)?);
        }
        let has_more = out.len() > page_limit;
        if has_more {
            out.truncate(page_limit);
        }
        let next_cursor = if has_more {
            out.last().map(|o| o.key.clone())
        } else {
            None
        };
        Ok((out, next_cursor))
    }

    async fn list_due_blob_gc(&self, now_secs: i64, limit: usize) -> Result<Vec<String>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT blob_path FROM blob_gc WHERE delete_after <= ?1 LIMIT ?2",
                (now_secs, limit as i64),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row.get(0).map_err(map_turso_err)?);
        }
        Ok(out)
    }

    async fn claim_blob_gc_delete(&self, blob_path: &str) -> Result<bool> {
        let path_s = blob_path.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let path_s = path_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut cnt_rows = tx
                    .query(
                        "SELECT COUNT(*) FROM blob_objects WHERE blob_path = ?1",
                        [path_s.as_str()],
                    )
                    .await?;
                let cnt = match cnt_rows.next().await? {
                    Some(r) => r.get::<i64>(0)?,
                    None => 0,
                };
                drop(cnt_rows);
                tx.execute(
                    "DELETE FROM blob_gc WHERE blob_path = ?1",
                    [path_s.as_str()],
                )
                .await?;
                tx.commit().await?;
                Ok(cnt == 0)
            }
        })
        .await
    }

    async fn schedule_orphan_gc(&self, blob_path: &str, gc_grace_secs: i64) -> Result<()> {
        let delete_after = Utc::now().timestamp() + gc_grace_secs;
        let path_s = blob_path.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let path_s = path_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut cnt_rows = tx
                    .query(
                        "SELECT COUNT(*) FROM blob_objects WHERE blob_path = ?1",
                        [path_s.as_str()],
                    )
                    .await?;
                let cnt = match cnt_rows.next().await? {
                    Some(r) => r.get::<i64>(0)?,
                    None => 0,
                };
                drop(cnt_rows);
                if cnt == 0 {
                    tx.execute(
                        "INSERT OR IGNORE INTO blob_gc (blob_path, delete_after) VALUES (?1, ?2)",
                        (path_s, delete_after),
                    )
                    .await?;
                }
                tx.commit().await
            }
        })
        .await
    }
}
