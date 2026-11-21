use async_trait::async_trait;
use chrono::Utc;
use turso::Value as TursoValue;

use crate::core::model::{CronJob, DbFtsIndexMeta, FerroError, KvNamespaceMeta, Result, UserDbMeta};
use crate::core::traits::RegistryStore;
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{opt_i64, opt_text, TursoCoreStore};

#[async_trait]
impl RegistryStore for TursoCoreStore {
    async fn register_user_db(&self, id: &str, name: &str, project_id: Option<&str>) -> Result<()> {
        let now = Utc::now().timestamp();
        let id_s = id.to_string();
        let name_s = name.to_string();
        let proj_s = project_id.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            let name_s = name_s.clone();
            let proj_s = proj_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO user_databases (id, name, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                    (id_s, name_s, proj_s, now),
                )
                .await
            }
        })
        .await?;
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
                    .execute("DELETE FROM user_db_fts_indexes WHERE db_id = ?1", [id_s.as_str()])
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
                "SELECT id, name, project_id, created_at FROM user_databases WHERE id = ?1 OR name = ?1 LIMIT 1",
                [id_or_name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(UserDbMeta {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                created_at: row.get(3).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_user_dbs(&self) -> Result<Vec<UserDbMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, project_id, created_at FROM user_databases ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(UserDbMeta {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                created_at: row.get(3).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }

    async fn create_kv_namespace(
        &self,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        let now = Utc::now().timestamp();
        let name_s = name.to_string();
        let proj_s = project_id.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let name_s = name_s.clone();
            let proj_s = proj_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO kv_namespaces (name, project_id, created_at) VALUES (?1, ?2, ?3)",
                    (name_s, proj_s, now),
                )
                .await
            }
        })
        .await?;

        Ok(KvNamespaceMeta {
            name: name.to_string(),
            project_id: project_id.map(str::to_string),
            created_at: now,
        })
    }

    async fn get_kv_namespace(&self, name: &str) -> Result<Option<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, created_at FROM kv_namespaces WHERE name = ?1 LIMIT 1",
                [name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(KvNamespaceMeta {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                created_at: row.get(2).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_kv_namespaces(&self) -> Result<Vec<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, created_at FROM kv_namespaces ORDER BY name ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(KvNamespaceMeta {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                created_at: row.get(2).map_err(map_turso_err)?,
            });
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
                    "INSERT INTO cron_jobs (id, name, project_id, schedule, target, enabled, last_run_at, last_status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            .query(
                "SELECT id, name, project_id, schedule, target, enabled, last_run_at, last_status, created_at, updated_at
                 FROM cron_jobs WHERE id = ?1 LIMIT 1",
                [id],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(CronJob {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                schedule: row.get(3).map_err(map_turso_err)?,
                target: row.get(4).map_err(map_turso_err)?,
                enabled: row.get::<i64>(5).map_err(map_turso_err)? != 0,
                last_run_at: opt_i64(&row, 6)?,
                last_status: opt_text(&row, 7)?,
                created_at: row.get(8).map_err(map_turso_err)?,
                updated_at: row.get(9).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_cron_jobs(&self) -> Result<Vec<CronJob>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, project_id, schedule, target, enabled, last_run_at, last_status, created_at, updated_at
                 FROM cron_jobs ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(CronJob {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                schedule: row.get(3).map_err(map_turso_err)?,
                target: row.get(4).map_err(map_turso_err)?,
                enabled: row.get::<i64>(5).map_err(map_turso_err)? != 0,
                last_run_at: opt_i64(&row, 6)?,
                last_status: opt_text(&row, 7)?,
                created_at: row.get(8).map_err(map_turso_err)?,
                updated_at: row.get(9).map_err(map_turso_err)?,
            });
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
                    "UPDATE cron_jobs SET name = ?1, schedule = ?2, target = ?3, enabled = ?4, last_run_at = ?5, last_status = ?6, updated_at = ?7
                     WHERE id = ?8",
                    vec![
                        TursoValue::Text(j.name),
                        TursoValue::Text(j.schedule),
                        TursoValue::Text(j.target),
                        TursoValue::Integer(if j.enabled { 1 } else { 0 }),
                        j.last_run_at.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        j.last_status.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(j.updated_at),
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
