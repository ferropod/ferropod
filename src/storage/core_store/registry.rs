use async_trait::async_trait;
use chrono::Utc;
use turso::Value as TursoValue;

use crate::core::model::{
    CronJob, DbFtsIndexMeta, FerroError, KvNamespaceMeta, KvNamespaceOpts, Result, UserDbMeta,
    UserDbOpts, DEFAULT_KV_TTL_SECS,
};
use crate::core::traits::RegistryStore;
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{opt_i64, opt_text, opt_text_param, TursoCoreStore};

const SELECT_USER_DB: &str =
    "SELECT id, name, project_id, created_at, resource_group, description, max_rows, max_size_mb, foreign_keys, read_only FROM user_databases";

const SELECT_KV_NAMESPACE: &str =
    "SELECT name, project_id, created_at, resource_group, description, default_ttl_secs, default_ephemeral, max_value_bytes, max_keys FROM kv_namespaces";

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
                    "INSERT INTO user_databases (id, name, project_id, created_at, resource_group, description, max_rows, max_size_mb, foreign_keys, read_only)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                    "UPDATE user_databases SET resource_group = ?1, description = ?2, max_rows = ?3, max_size_mb = ?4, foreign_keys = ?5, read_only = ?6
                     WHERE id = ?7",
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
        };

        let m = meta.clone();
        with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO kv_namespaces (name, project_id, created_at, resource_group, description, default_ttl_secs, default_ephemeral, max_value_bytes, max_keys)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
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
                    "UPDATE kv_namespaces SET resource_group = ?1, description = ?2, default_ttl_secs = ?3, default_ephemeral = ?4, max_value_bytes = ?5, max_keys = ?6
                     WHERE name = ?7",
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
}
