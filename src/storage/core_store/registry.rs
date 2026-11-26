use async_trait::async_trait;
use chrono::Utc;
use turso::Value as TursoValue;

use crate::core::model::{
    default_ai_timeout_secs, AiProvider, AiProviderRecord, AiUsageRecord, AlertWebhook,
    AlertWebhookRecord, CronJob, DbFtsIndexMeta, FerroError, KvNamespaceMeta, KvNamespaceOpts,
    Result, UserDbMeta, UserDbOpts, DEFAULT_KV_TTL_SECS,
};
use crate::core::traits::RegistryStore;
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{opt_blob, opt_blob_param, opt_i64, opt_text, opt_text_param, TursoCoreStore};

const SELECT_USER_DB: &str =
    "SELECT id, name, project_id, created_at, resource_group, description, max_rows, max_size_mb, foreign_keys, read_only, query_timeout_ms, rate_limit_rpm FROM user_databases";

const SELECT_KV_NAMESPACE: &str =
    "SELECT name, project_id, created_at, resource_group, description, default_ttl_secs, default_ephemeral, max_value_bytes, max_keys, rate_limit_rpm FROM kv_namespaces";

const SELECT_CRON_JOB: &str = "SELECT id, name, project_id, schedule, target, enabled, \
     last_run_at, last_status, created_at, updated_at, description, timezone, timeout_ms, max_retries \
     FROM cron_jobs";

fn row_to_user_db(row: &turso::Row) -> Result<UserDbMeta> {
    Ok(UserDbMeta {
        id: row.get(0).map_err(map_turso_err)?,
        name: row.get(1).map_err(map_turso_err)?,
        project_id: opt_text(row, 2)?,
        created_at: row.get(3).map_err(map_turso_err)?,
        group: opt_text(row, 4)?,
        description: opt_text(row, 5)?,
        max_rows: opt_i64(row, 6)?.and_then(|v| usize::try_from(v).ok()),
        max_size_mb: opt_i64(row, 7)?.and_then(|v| usize::try_from(v).ok()),
        foreign_keys: opt_i64(row, 8)?.unwrap_or(0) != 0,
        read_only: opt_i64(row, 9)?.unwrap_or(0) != 0,
        query_timeout_ms: opt_i64(row, 10)?
            .and_then(|v| u64::try_from(v).ok())
            .filter(|v| *v > 0),
        rate_limit_rpm: opt_i64(row, 11)?
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0),
    })
}

fn row_to_kv_namespace(row: &turso::Row) -> Result<KvNamespaceMeta> {
    let raw_ttl = opt_i64(row, 5)?
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_KV_TTL_SECS);
    Ok(KvNamespaceMeta {
        name: row.get(0).map_err(map_turso_err)?,
        project_id: opt_text(row, 1)?,
        created_at: row.get(2).map_err(map_turso_err)?,
        group: opt_text(row, 3)?,
        description: opt_text(row, 4)?,
        default_ttl_secs: raw_ttl,
        default_ephemeral: opt_i64(row, 6)?.unwrap_or(0) != 0,
        max_value_bytes: opt_i64(row, 7)?.and_then(|v| usize::try_from(v).ok()),
        max_keys: opt_i64(row, 8)?.and_then(|v| usize::try_from(v).ok()),
        rate_limit_rpm: opt_i64(row, 9)?
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0),
    })
}

fn row_to_cron_job(row: &turso::Row) -> Result<CronJob> {
    Ok(CronJob {
        id: row.get(0).map_err(map_turso_err)?,
        name: row.get(1).map_err(map_turso_err)?,
        project_id: opt_text(row, 2)?,
        schedule: row.get(3).map_err(map_turso_err)?,
        target: row.get(4).map_err(map_turso_err)?,
        enabled: row.get::<i64>(5).map_err(map_turso_err)? != 0,
        last_run_at: opt_i64(row, 6)?,
        last_status: opt_text(row, 7)?,
        created_at: row.get(8).map_err(map_turso_err)?,
        updated_at: row.get(9).map_err(map_turso_err)?,
        description: opt_text(row, 10)?,
        timezone: opt_text(row, 11)?.unwrap_or_else(|| "UTC".to_string()),
        timeout_ms: opt_i64(row, 12)?.and_then(|v| u64::try_from(v).ok()),
        max_retries: opt_i64(row, 13)?
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0),
    })
}

#[async_trait]
impl RegistryStore for TursoCoreStore {
    async fn register_user_db_with_opts(
        &self,
        id: &str,
        name: &str,
        opts: UserDbOpts,
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        let id_s = id.to_string();
        let name_s = name.to_string();
        let description = opts
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let max_rows = opts.max_rows.filter(|v| *v > 0);
        let max_size_mb = opts.max_size_mb.filter(|v| *v > 0);
        let foreign_keys = opts.foreign_keys.unwrap_or(false);
        let read_only = opts.read_only.unwrap_or(false);
        let query_timeout_ms = opts.query_timeout_ms.filter(|v| *v > 0);
        let rate_limit_rpm = opts.rate_limit_rpm.filter(|v| *v > 0);
        let proj_s = opts.project_id;
        let group = opts.group;
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            let name_s = name_s.clone();
            let proj_s = proj_s.clone();
            let group = group.clone();
            let description = description.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO user_databases (id, name, project_id, created_at, resource_group, description, max_rows, max_size_mb, foreign_keys, read_only, query_timeout_ms, rate_limit_rpm)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    vec![
                        TursoValue::Text(id_s),
                        TursoValue::Text(name_s),
                        opt_text_param(proj_s),
                        TursoValue::Integer(now),
                        opt_text_param(group),
                        opt_text_param(description),
                        max_rows
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        max_size_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(if foreign_keys { 1 } else { 0 }),
                        TursoValue::Integer(if read_only { 1 } else { 0 }),
                        query_timeout_ms
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        rate_limit_rpm
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn update_user_db(&self, meta: &UserDbMeta) -> Result<()> {
        let m = meta.clone();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE user_databases SET resource_group = ?1, description = ?2, max_rows = ?3, max_size_mb = ?4, foreign_keys = ?5, read_only = ?6, query_timeout_ms = ?7, rate_limit_rpm = ?8
                     WHERE id = ?9",
                    vec![
                        opt_text_param(m.group),
                        opt_text_param(m.description),
                        m.max_rows
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.max_size_mb
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(if m.foreign_keys { 1 } else { 0 }),
                        TursoValue::Integer(if m.read_only { 1 } else { 0 }),
                        m.query_timeout_ms
                            .filter(|v| *v > 0)
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.rate_limit_rpm
                            .filter(|v| *v > 0)
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Text(m.id),
                    ],
                )
                .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(format!("user db '{}'", meta.id)));
        }
        Ok(())
    }

    async fn delete_user_db(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                let _ = conn
                    .execute(
                        "DELETE FROM user_db_fts_indexes WHERE db_id = ?1",
                        [id_s.as_str()],
                    )
                    .await;
                conn.execute("DELETE FROM user_databases WHERE id = ?1", [id_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn get_user_db(&self, id_or_name: &str) -> Result<Option<UserDbMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                &format!("{SELECT_USER_DB} WHERE id = ?1 OR name = ?1 ORDER BY CASE WHEN id = ?1 THEN 0 ELSE 1 END LIMIT 1"),
                [id_or_name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            row_to_user_db(&row).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn list_user_dbs(&self) -> Result<Vec<UserDbMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_USER_DB} ORDER BY created_at DESC"), ())
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_user_db(&row)?);
        }
        Ok(out)
    }

    async fn create_kv_namespace_with_opts(
        &self,
        name: &str,
        opts: KvNamespaceOpts,
    ) -> Result<KvNamespaceMeta> {
        let now = Utc::now().timestamp();
        let default_ttl_secs = opts.default_ttl_secs.unwrap_or(DEFAULT_KV_TTL_SECS).max(1);
        let default_ephemeral = opts.default_ephemeral.unwrap_or(false);
        let max_value_bytes = opts.max_value_bytes.filter(|v| *v > 0);
        let max_keys = opts.max_keys.filter(|v| *v > 0);
        let rate_limit_rpm = opts.rate_limit_rpm.filter(|v| *v > 0);
        let description = opts
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let meta = KvNamespaceMeta {
            name: name.to_string(),
            project_id: opts.project_id,
            created_at: now,
            group: opts.group,
            description,
            default_ttl_secs,
            default_ephemeral,
            max_value_bytes,
            max_keys,
            rate_limit_rpm,
        };

        let m = meta.clone();
        with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO kv_namespaces (name, project_id, created_at, resource_group, description, default_ttl_secs, default_ephemeral, max_value_bytes, max_keys, rate_limit_rpm)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    vec![
                        TursoValue::Text(m.name),
                        opt_text_param(m.project_id),
                        TursoValue::Integer(m.created_at),
                        opt_text_param(m.group),
                        opt_text_param(m.description),
                        TursoValue::Integer(m.default_ttl_secs as i64),
                        TursoValue::Integer(if m.default_ephemeral { 1 } else { 0 }),
                        m.max_value_bytes
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.max_keys
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.rate_limit_rpm
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                    ],
                )
                .await
            }
        })
        .await?;

        Ok(meta)
    }

    async fn update_kv_namespace(&self, meta: &KvNamespaceMeta) -> Result<()> {
        let m = meta.clone();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE kv_namespaces SET resource_group = ?1, description = ?2, default_ttl_secs = ?3, default_ephemeral = ?4, max_value_bytes = ?5, max_keys = ?6, rate_limit_rpm = ?7
                     WHERE name = ?8",
                    vec![
                        opt_text_param(m.group),
                        opt_text_param(m.description),
                        TursoValue::Integer(m.default_ttl_secs.max(1) as i64),
                        TursoValue::Integer(if m.default_ephemeral { 1 } else { 0 }),
                        m.max_value_bytes
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.max_keys
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        m.rate_limit_rpm
                            .filter(|v| *v > 0)
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Text(m.name),
                    ],
                )
                .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(format!(
                "kv namespace '{}'",
                meta.name
            )));
        }
        Ok(())
    }

    async fn get_kv_namespace(&self, name: &str) -> Result<Option<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                &format!("{SELECT_KV_NAMESPACE} WHERE name = ?1 LIMIT 1"),
                [name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            row_to_kv_namespace(&row).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn list_kv_namespaces(&self) -> Result<Vec<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_KV_NAMESPACE} ORDER BY name ASC"), ())
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_kv_namespace(&row)?);
        }
        Ok(out)
    }

    async fn delete_kv_namespace(&self, name: &str) -> Result<()> {
        let name_s = name.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let name_s = name_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM kv_namespaces WHERE name = ?1", [name_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn create_cron_job(&self, job: CronJob) -> Result<CronJob> {
        let j = job.clone();
        with_retry(|| {
            let db = self.db.clone();
            let j = j.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO cron_jobs (id, name, project_id, schedule, target, enabled, last_run_at, last_status, created_at, updated_at, description, timezone, timeout_ms, max_retries)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    vec![
                        TursoValue::Text(j.id),
                        TursoValue::Text(j.name),
                        j.project_id.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        TursoValue::Text(j.schedule),
                        TursoValue::Text(j.target),
                        TursoValue::Integer(if j.enabled { 1 } else { 0 }),
                        j.last_run_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        j.last_status.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(j.created_at),
                        TursoValue::Integer(j.updated_at),
                        opt_text_param(j.description),
                        TursoValue::Text(j.timezone),
                        j.timeout_ms
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(j.max_retries as i64),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(job)
    }

    async fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_CRON_JOB} WHERE id = ?1 LIMIT 1"), [id])
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            row_to_cron_job(&row).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn list_cron_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(&format!("{SELECT_CRON_JOB} ORDER BY created_at DESC"), ())
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_cron_job(&row)?);
        }
        Ok(out)
    }

    async fn update_cron_job(&self, job: &CronJob) -> Result<()> {
        let j = job.clone();
        with_retry(|| {
            let db = self.db.clone();
            let j = j.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE cron_jobs SET name = ?1, schedule = ?2, target = ?3, enabled = ?4, last_run_at = ?5, last_status = ?6, updated_at = ?7,
                     description = ?8, timezone = ?9, timeout_ms = ?10, max_retries = ?11
                     WHERE id = ?12",
                    vec![
                        TursoValue::Text(j.name),
                        TursoValue::Text(j.schedule),
                        TursoValue::Text(j.target),
                        TursoValue::Integer(if j.enabled { 1 } else { 0 }),
                        j.last_run_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        j.last_status.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(j.updated_at),
                        opt_text_param(j.description),
                        TursoValue::Text(j.timezone),
                        j.timeout_ms
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(j.max_retries as i64),
                        TursoValue::Text(j.id),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn delete_cron_job(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM cron_jobs WHERE id = ?1", [id_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn create_db_fts_index(&self, meta: DbFtsIndexMeta) -> Result<DbFtsIndexMeta> {
        let cols_json = serde_json::to_string(&meta.columns)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("encode fts columns: {e}")))?;
        let m = meta.clone();
        with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            let cols_json = cols_json.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO user_db_fts_indexes (db_id, name, table_name, columns_json, tokenizer, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    (
                        m.db_id,
                        m.name,
                        m.table,
                        cols_json,
                        m.tokenizer,
                        m.created_at,
                    ),
                )
                .await
            }
        })
        .await?;
        Ok(meta)
    }

    async fn list_db_fts_indexes(&self, db_id: &str) -> Result<Vec<DbFtsIndexMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT db_id, name, table_name, columns_json, tokenizer, created_at
                 FROM user_db_fts_indexes WHERE db_id = ?1 ORDER BY created_at DESC",
                [db_id],
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let cols_raw: String = row.get(3).map_err(map_turso_err)?;
            let columns: Vec<String> = serde_json::from_str(&cols_raw).unwrap_or_default();
            out.push(DbFtsIndexMeta {
                db_id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                table: row.get(2).map_err(map_turso_err)?,
                columns,
                tokenizer: row.get(4).map_err(map_turso_err)?,
                rows_indexed: 0,
                size_bytes: 16384,
                created_at: row.get(5).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }

    async fn delete_db_fts_index(&self, db_id: &str, name: &str) -> Result<()> {
        let db_s = db_id.to_string();
        let name_s = name.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let db_s = db_s.clone();
            let name_s = name_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "DELETE FROM user_db_fts_indexes WHERE db_id = ?1 AND name = ?2",
                    (db_s, name_s),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn upsert_ai_provider(&self, rec: AiProviderRecord) -> Result<AiProviderRecord> {
        let r = rec.clone();
        with_retry(|| {
            let db = self.db.clone();
            let r = r.clone();
            async move {
                let conn = db.connect()?;
                let p = &r.provider;
                conn.execute(
                    "INSERT INTO ai_providers (id, name, base_url, api_key_nonce, api_key_ciphertext, default_model, fallback_provider, rate_limit_rpm, cache_ttl_secs, monthly_token_budget, timeout_secs, enabled, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                     ON CONFLICT(name) DO UPDATE SET
                        base_url = excluded.base_url,
                        api_key_nonce = COALESCE(excluded.api_key_nonce, ai_providers.api_key_nonce),
                        api_key_ciphertext = COALESCE(excluded.api_key_ciphertext, ai_providers.api_key_ciphertext),
                        default_model = excluded.default_model,
                        fallback_provider = excluded.fallback_provider,
                        rate_limit_rpm = excluded.rate_limit_rpm,
                        cache_ttl_secs = excluded.cache_ttl_secs,
                        monthly_token_budget = excluded.monthly_token_budget,
                        timeout_secs = excluded.timeout_secs,
                        enabled = excluded.enabled,
                        updated_at = excluded.updated_at",
                    vec![
                        TursoValue::Text(p.id.clone()),
                        TursoValue::Text(p.name.clone()),
                        TursoValue::Text(p.base_url.clone()),
                        opt_blob_param(r.api_key_nonce),
                        opt_blob_param(r.api_key_ciphertext),
                        opt_text_param(p.default_model.clone()),
                        opt_text_param(p.fallback_provider.clone()),
                        p.rate_limit_rpm
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(p.cache_ttl_secs as i64),
                        p.monthly_token_budget
                            .map(|v| TursoValue::Integer(v as i64))
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(p.timeout_secs as i64),
                        TursoValue::Integer(if p.enabled { 1 } else { 0 }),
                        TursoValue::Integer(p.created_at),
                        TursoValue::Integer(p.updated_at),
                    ],
                )
                .await
            }
        })
        .await?;
        self.get_ai_provider(&rec.provider.name)
            .await?
            .ok_or_else(|| FerroError::Internal(anyhow::anyhow!("failed to reload ai provider")))
    }

    async fn get_ai_provider(&self, name: &str) -> Result<Option<AiProviderRecord>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, base_url, api_key_nonce, api_key_ciphertext, default_model, fallback_provider, rate_limit_rpm, cache_ttl_secs, monthly_token_budget, enabled, created_at, updated_at, timeout_secs
                 FROM ai_providers WHERE name = ?1 OR id = ?1 LIMIT 1",
                [name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_ai_provider(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_ai_providers(&self) -> Result<Vec<AiProviderRecord>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, base_url, api_key_nonce, api_key_ciphertext, default_model, fallback_provider, rate_limit_rpm, cache_ttl_secs, monthly_token_budget, enabled, created_at, updated_at, timeout_secs
                 FROM ai_providers ORDER BY created_at ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_ai_provider(&row)?);
        }
        Ok(out)
    }

    async fn delete_ai_provider(&self, name: &str) -> Result<bool> {
        let n = name.to_string();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let n = n.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM ai_providers WHERE name = ?1 OR id = ?1", [n])
                    .await
            }
        })
        .await?;
        Ok(affected > 0)
    }

    async fn record_ai_usage(&self, rec: &AiUsageRecord) -> Result<()> {
        let r = rec.clone();
        with_retry(|| {
            let db = self.db.clone();
            let r = r.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO ai_usage (provider_name, project_id, model, prompt_tokens, completion_tokens, total_tokens, cached, status, latency_ms, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    vec![
                        TursoValue::Text(r.provider_name),
                        opt_text_param(r.project_id),
                        opt_text_param(r.model),
                        TursoValue::Integer(r.prompt_tokens as i64),
                        TursoValue::Integer(r.completion_tokens as i64),
                        TursoValue::Integer(r.total_tokens as i64),
                        TursoValue::Integer(if r.cached { 1 } else { 0 }),
                        TursoValue::Integer(r.status as i64),
                        TursoValue::Integer(r.latency_ms as i64),
                        TursoValue::Integer(r.created_at),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn list_ai_usage(
        &self,
        provider_name: Option<&str>,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AiUsageRecord>> {
        let conn = connect_db(&self.db)?;
        let limit_i = limit.clamp(1, 500) as i64;
        let mut conds: Vec<&'static str> = Vec::new();
        let mut params: Vec<TursoValue> = Vec::new();
        if let Some(p) = provider_name {
            params.push(TursoValue::Text(p.to_string()));
            conds.push(if params.len() == 1 {
                "provider_name = ?1"
            } else {
                "provider_name = ?2"
            });
        }
        if let Some(pid) = project_id {
            params.push(TursoValue::Text(pid.to_string()));
            conds.push(if params.len() == 1 {
                "project_id = ?1"
            } else {
                "project_id = ?2"
            });
        }
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let limit_idx = params.len() + 1;
        params.push(TursoValue::Integer(limit_i));
        let sql = format!(
            "SELECT id, provider_name, project_id, model, prompt_tokens, completion_tokens, total_tokens, cached, status, latency_ms, created_at
             FROM ai_usage {where_clause} ORDER BY id DESC LIMIT ?{limit_idx}"
        );
        let mut rows = conn.query(&sql, params).await.map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(AiUsageRecord {
                id: row.get(0).map_err(map_turso_err)?,
                provider_name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                model: opt_text(&row, 3)?,
                prompt_tokens: row.get::<i64>(4).map_err(map_turso_err)?.max(0) as u64,
                completion_tokens: row.get::<i64>(5).map_err(map_turso_err)?.max(0) as u64,
                total_tokens: row.get::<i64>(6).map_err(map_turso_err)?.max(0) as u64,
                cached: row.get::<i64>(7).map_err(map_turso_err)? != 0,
                status: row.get::<i64>(8).map_err(map_turso_err)?.clamp(0, 999) as u16,
                latency_ms: row.get::<i64>(9).map_err(map_turso_err)?.max(0) as u64,
                created_at: row.get(10).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }

    async fn aggregate_ai_usage(
        &self,
        provider_name: Option<&str>,
        project_id: Option<&str>,
        month_start_ms: i64,
    ) -> Result<(usize, usize, u64, u64, u64, u64)> {
        let conn = connect_db(&self.db)?;
        let mut conds: Vec<String> = Vec::new();
        let mut params: Vec<TursoValue> = vec![TursoValue::Integer(month_start_ms)];
        if let Some(p) = provider_name {
            params.push(TursoValue::Text(p.to_string()));
            conds.push(format!("provider_name = ?{}", params.len()));
        }
        if let Some(pid) = project_id {
            params.push(TursoValue::Text(pid.to_string()));
            conds.push(format!("project_id = ?{}", params.len()));
        }
        let where_clause = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let sql = format!(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN cached != 0 THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(total_tokens), 0),
                    COALESCE(SUM(CASE WHEN created_at >= ?1 AND cached = 0 THEN total_tokens ELSE 0 END), 0)
             FROM ai_usage {where_clause}"
        );
        let mut rows = conn.query(&sql, params).await.map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let total_req = row.get::<i64>(0).map_err(map_turso_err)?.max(0) as usize;
            let cached_req = row.get::<i64>(1).map_err(map_turso_err)?.max(0) as usize;
            let prompt_tok = row.get::<i64>(2).map_err(map_turso_err)?.max(0) as u64;
            let completion_tok = row.get::<i64>(3).map_err(map_turso_err)?.max(0) as u64;
            let total_tok = row.get::<i64>(4).map_err(map_turso_err)?.max(0) as u64;
            let month_tok = row.get::<i64>(5).map_err(map_turso_err)?.max(0) as u64;
            Ok((
                total_req,
                cached_req,
                prompt_tok,
                completion_tok,
                total_tok,
                month_tok,
            ))
        } else {
            Ok((0, 0, 0, 0, 0, 0))
        }
    }

    async fn cleanup_ai_usage_before(&self, cutoff_ms: i64) -> Result<u64> {
        let affected = with_retry(|| {
            let db = self.db.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM ai_usage WHERE created_at < ?1", [cutoff_ms])
                    .await
            }
        })
        .await?;
        Ok(affected)
    }

    async fn sum_ai_tokens_since(&self, provider_name: &str, since_ms: i64) -> Result<u64> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT COALESCE(SUM(total_tokens), 0) FROM ai_usage WHERE provider_name = ?1 AND created_at >= ?2 AND cached = 0",
                (provider_name, since_ms),
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(row.get::<i64>(0).map_err(map_turso_err)?.max(0) as u64)
        } else {
            Ok(0)
        }
    }

    async fn upsert_alert_webhook(&self, rec: AlertWebhookRecord) -> Result<AlertWebhookRecord> {
        let r = rec.clone();
        with_retry(|| {
            let db = self.db.clone();
            let r = r.clone();
            async move {
                let conn = db.connect()?;
                let w = &r.webhook;
                let events_json = serde_json::to_string(&w.events)
                    .unwrap_or_else(|_| "[\"*\"]".to_string());
                let clear_secret = !w.has_secret;
                conn.execute(
                    "INSERT INTO alert_webhooks (id, name, url, secret_nonce, secret_ciphertext, events_json, enabled, last_status, last_error, last_triggered_at, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                     ON CONFLICT(id) DO UPDATE SET
                        name = excluded.name,
                        url = excluded.url,
                        secret_nonce = CASE WHEN ?13 != 0 THEN NULL ELSE COALESCE(excluded.secret_nonce, alert_webhooks.secret_nonce) END,
                        secret_ciphertext = CASE WHEN ?13 != 0 THEN NULL ELSE COALESCE(excluded.secret_ciphertext, alert_webhooks.secret_ciphertext) END,
                        events_json = excluded.events_json,
                        enabled = excluded.enabled,
                        updated_at = excluded.updated_at",
                    vec![
                        TursoValue::Text(w.id.clone()),
                        TursoValue::Text(w.name.clone()),
                        TursoValue::Text(w.url.clone()),
                        opt_blob_param(r.secret_nonce),
                        opt_blob_param(r.secret_ciphertext),
                        TursoValue::Text(events_json),
                        TursoValue::Integer(if w.enabled { 1 } else { 0 }),
                        w.last_status
                            .map(|s| TursoValue::Integer(s as i64))
                            .unwrap_or(TursoValue::Null),
                        opt_text_param(w.last_error.clone()),
                        w.last_triggered_at
                            .map(TursoValue::Integer)
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(w.created_at),
                        TursoValue::Integer(w.updated_at),
                        TursoValue::Integer(if clear_secret { 1 } else { 0 }),
                    ],
                )
                .await
            }
        })
        .await?;
        self.get_alert_webhook(&rec.webhook.id)
            .await?
            .ok_or_else(|| FerroError::Internal(anyhow::anyhow!("failed to reload alert webhook")))
    }

    async fn get_alert_webhook(&self, id: &str) -> Result<Option<AlertWebhookRecord>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, url, secret_nonce, secret_ciphertext, events_json, enabled, last_status, last_error, last_triggered_at, created_at, updated_at
                 FROM alert_webhooks WHERE id = ?1 LIMIT 1",
                [id],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_alert_webhook(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_alert_webhooks(&self) -> Result<Vec<AlertWebhookRecord>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, url, secret_nonce, secret_ciphertext, events_json, enabled, last_status, last_error, last_triggered_at, created_at, updated_at
                 FROM alert_webhooks ORDER BY created_at ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_alert_webhook(&row)?);
        }
        Ok(out)
    }

    async fn delete_alert_webhook(&self, id: &str) -> Result<bool> {
        let id_s = id.to_string();
        let affected = with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM alert_webhooks WHERE id = ?1", [id_s])
                    .await
            }
        })
        .await?;
        Ok(affected > 0)
    }

    async fn record_alert_webhook_delivery(
        &self,
        id: &str,
        status: Option<u16>,
        error: Option<&str>,
        triggered_at: i64,
    ) -> Result<()> {
        let id_s = id.to_string();
        let err_s = error.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            let err_s = err_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE alert_webhooks
                     SET last_status = ?1, last_error = ?2, last_triggered_at = ?3
                     WHERE id = ?4",
                    vec![
                        status
                            .map(|s| TursoValue::Integer(s as i64))
                            .unwrap_or(TursoValue::Null),
                        opt_text_param(err_s),
                        TursoValue::Integer(triggered_at),
                        TursoValue::Text(id_s),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn get_platform_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT value_json FROM platform_settings WHERE key = ?1 LIMIT 1",
                [key],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let val: String = row.get(0).map_err(map_turso_err)?;
            Ok(Some(val))
        } else {
            Ok(None)
        }
    }

    async fn set_platform_setting(&self, key: &str, value_json: &str) -> Result<()> {
        let key_s = key.to_string();
        let val_s = value_json.to_string();
        let now = Utc::now().timestamp_millis();
        with_retry(|| {
            let db = self.db.clone();
            let key_s = key_s.clone();
            let val_s = val_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO platform_settings (key, value_json, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET
                        value_json = excluded.value_json,
                        updated_at = excluded.updated_at",
                    (key_s, val_s, now),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn delete_platform_setting(&self, key: &str) -> Result<()> {
        let key_s = key.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let key_s = key_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM platform_settings WHERE key = ?1", [key_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }
}

fn row_to_ai_provider(row: &turso::Row) -> Result<AiProviderRecord> {
    let nonce = opt_blob(row, 3)?;
    let ct = opt_blob(row, 4)?;
    let has_api_key = ct.as_ref().is_some_and(|c| !c.is_empty());
    let timeout_secs = opt_i64(row, 13)?
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or_else(default_ai_timeout_secs);
    Ok(AiProviderRecord {
        provider: AiProvider {
            id: row.get(0).map_err(map_turso_err)?,
            name: row.get(1).map_err(map_turso_err)?,
            base_url: row.get(2).map_err(map_turso_err)?,
            has_api_key,
            default_model: opt_text(row, 5)?,
            fallback_provider: opt_text(row, 6)?,
            rate_limit_rpm: opt_i64(row, 7)?.and_then(|v| u32::try_from(v).ok()),
            cache_ttl_secs: opt_i64(row, 8)?.unwrap_or(0).max(0) as u64,
            monthly_token_budget: opt_i64(row, 9)?.and_then(|v| u64::try_from(v).ok()),
            timeout_secs,
            enabled: row.get::<i64>(10).map_err(map_turso_err)? != 0,
            created_at: row.get(11).map_err(map_turso_err)?,
            updated_at: row.get(12).map_err(map_turso_err)?,
        },
        api_key_nonce: nonce,
        api_key_ciphertext: ct,
    })
}

fn row_to_alert_webhook(row: &turso::Row) -> Result<AlertWebhookRecord> {
    let nonce = opt_blob(row, 3)?;
    let ct = opt_blob(row, 4)?;
    let has_secret = ct.as_ref().is_some_and(|c| !c.is_empty());
    let events_raw = opt_text(row, 5)?.unwrap_or_else(|| "[\"*\"]".to_string());
    let events: Vec<String> =
        serde_json::from_str(&events_raw).unwrap_or_else(|_| vec!["*".into()]);
    Ok(AlertWebhookRecord {
        webhook: AlertWebhook {
            id: row.get(0).map_err(map_turso_err)?,
            name: row.get(1).map_err(map_turso_err)?,
            url: row.get(2).map_err(map_turso_err)?,
            has_secret,
            events,
            enabled: row.get::<i64>(6).map_err(map_turso_err)? != 0,
            last_status: opt_i64(row, 7)?.and_then(|v| u16::try_from(v).ok()),
            last_error: opt_text(row, 8)?,
            last_triggered_at: opt_i64(row, 9)?,
            created_at: row.get(10).map_err(map_turso_err)?,
            updated_at: row.get(11).map_err(map_turso_err)?,
        },
        secret_nonce: nonce,
        secret_ciphertext: ct,
    })
}
