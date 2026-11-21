use std::path::Path;
use std::time::Instant;

use chrono::Utc;
use uuid::Uuid;

use crate::core::model::{
    Caller, DbFtsHit, DbFtsIndexMeta, DbFtsQueryResult, FerroError, QueryResult, Result, SqlValue,
    TokenScope, UserDbMeta,
};
use crate::storage::search_store::score_json_document;

use super::PlatformService;

fn valid_sql_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

impl PlatformService {
    async fn resolve_user_db(&self, id_or_name: &str) -> Result<UserDbMeta> {
        self.registry
            .get_user_db(id_or_name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("user db '{id_or_name}'")))
    }

    pub async fn create_user_db(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<String> {
        self.create_grouped_user_db(caller, name, project_id, None)
            .await
    }

    pub async fn create_grouped_user_db(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        group: Option<&str>,
    ) -> Result<String> {
        let group = super::normalize_resource_group(group)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "database name cannot be empty".into(),
            ));
        }
        let resolved_pid = match (&caller.scope, project_id) {
            (TokenScope::Admin, pid) => self.resolve_optional_project_id(pid).await?,
            (TokenScope::Project(_), Some(req_pid)) => {
                let proj = self.get_project_internal(req_pid).await?;
                Self::require_project_write(caller, &proj.id)?;
                Some(proj.id)
            }
            (TokenScope::Project(scope_pid), None) => Some(scope_pid.clone()),
            (TokenScope::Readonly, _) => {
                return Err(FerroError::Forbidden(
                    "readonly token cannot create databases".into(),
                ));
            }
        };

        let id = format!("db_{}", Uuid::new_v4().simple());
        self.registry
            .register_user_db(&id, name, resolved_pid.as_deref(), group)
            .await?;
        if let Err(e) = self.db.create_db(&id).await {
            let _ = self.registry.delete_user_db(&id).await;
            return Err(e);
        }
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "db.create",
            serde_json::json!({ "id": id, "name": name }),
        );
        Ok(id)
    }

    pub async fn list_user_dbs(&self, caller: &Caller) -> Result<Vec<UserDbMeta>> {
        let all = self.registry.list_user_dbs().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|d| d.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn delete_user_db(&self, caller: &Caller, id_or_name: &str) -> Result<()> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.delete_db(&meta.id).await?;
        self.registry.delete_user_db(&meta.id).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "db.delete",
            serde_json::json!({ "id": meta.id, "name": meta.name }),
        );
        Ok(())
    }

    pub async fn schema_user_db(&self, caller: &Caller, id_or_name: &str) -> Result<String> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.schema(&meta.id).await
    }

    pub async fn query_user_db(
        &self,
        caller: &Caller,
        id_or_name: &str,
        sql: &str,
        params: Vec<SqlValue>,
    ) -> Result<QueryResult> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.query(&meta.id, sql, params).await
    }

    pub async fn execute_user_db_batch(
        &self,
        caller: &Caller,
        id_or_name: &str,
        stmts: Vec<(String, Vec<SqlValue>)>,
    ) -> Result<u64> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.execute_batch(&meta.id, stmts).await
    }

    pub async fn snapshot_user_dbs(&self, caller: &Caller, out_dir: &Path) -> Result<usize> {
        Self::require_admin(caller)?;
        tokio::fs::create_dir_all(out_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("create dbs snapshot dir: {e}")))?;
        let dbs = self.registry.list_user_dbs().await?;
        let mut count = 0usize;
        for db in dbs {
            let safe_name: String = db
                .name
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();
            let dest = out_dir.join(format!("{safe_name}__{}.db", db.id));
            self.db.snapshot_to(&db.id, &dest).await?;
            count += 1;
        }
        Ok(count)
    }

    pub async fn list_user_db_fts(
        &self,
        caller: &Caller,
        id_or_name: &str,
    ) -> Result<Vec<DbFtsIndexMeta>> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        let mut indexes = self.registry.list_db_fts_indexes(&meta.id).await?;
        for idx in &mut indexes {
            if valid_sql_ident(&idx.table) {
                let sql = format!("SELECT COUNT(*) FROM \"{}\"", idx.table);
                if let Ok(qr) = self.db.query(&meta.id, &sql, vec![]).await {
                    if let Some(first_row) = qr.rows.first() {
                        if let Some(cnt) = first_row.first().and_then(|v| v.as_i64()) {
                            idx.rows_indexed = cnt.max(0) as usize;
                            idx.size_bytes =
                                (idx.rows_indexed as u64).saturating_mul(256).max(4096);
                        }
                    }
                }
            }
        }
        Ok(indexes)
    }

    pub async fn create_user_db_fts(
        &self,
        caller: &Caller,
        id_or_name: &str,
        name: &str,
        table: &str,
        columns: Vec<String>,
        tokenizer: Option<&str>,
    ) -> Result<DbFtsIndexMeta> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        let name = name.trim();
        let table = table.trim();
        let cols: Vec<String> = columns
            .into_iter()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
            .collect();
        if !valid_sql_ident(name)
            || !valid_sql_ident(table)
            || cols.is_empty()
            || !cols.iter().all(|c| valid_sql_ident(c))
        {
            return Err(FerroError::InvalidArgument(
                "invalid FTS index name, table, or column identifier".into(),
            ));
        }

        let quoted_columns = cols
            .iter()
            .map(|col| format!("\"{col}\""))
            .collect::<Vec<_>>()
            .join(", ");
        self.db
            .query(
                &meta.id,
                &format!("SELECT {quoted_columns} FROM \"{table}\" LIMIT 0"),
                vec![],
            )
            .await?;

        let mut rows_indexed = 0usize;
        let count_sql = format!("SELECT COUNT(*) FROM \"{table}\"");
        if let Ok(qr) = self.db.query(&meta.id, &count_sql, vec![]).await {
            if let Some(cnt) = qr
                .rows
                .first()
                .and_then(|r| r.first())
                .and_then(|v| v.as_i64())
            {
                rows_indexed = cnt.max(0) as usize;
            }
        }

        let fts_meta = DbFtsIndexMeta {
            db_id: meta.id.clone(),
            name: name.to_string(),
            table: table.to_string(),
            columns: cols,
            tokenizer: tokenizer
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("ngram")
                .to_string(),
            rows_indexed,
            size_bytes: (rows_indexed as u64).saturating_mul(256).max(16384),
            created_at: Utc::now().timestamp_millis(),
        };
        let saved = self.registry.create_db_fts_index(fts_meta).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "db.fts.create",
            serde_json::json!({ "db_id": meta.id, "index": saved.name, "table": saved.table }),
        );
        Ok(saved)
    }

    pub async fn drop_user_db_fts(
        &self,
        caller: &Caller,
        id_or_name: &str,
        index_name: &str,
    ) -> Result<()> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.registry
            .delete_db_fts_index(&meta.id, index_name)
            .await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "db.fts.delete",
            serde_json::json!({ "db_id": meta.id, "index": index_name }),
        );
        Ok(())
    }

    pub async fn query_user_db_fts(
        &self,
        caller: &Caller,
        id_or_name: &str,
        index_name: &str,
        query: &str,
    ) -> Result<DbFtsQueryResult> {
        let started = Instant::now();
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        let indexes = self.registry.list_db_fts_indexes(&meta.id).await?;
        let mut idx = indexes
            .into_iter()
            .find(|i| i.name == index_name)
            .ok_or_else(|| FerroError::NotFound(format!("fts index '{index_name}'")))?;

        let q = query.trim();
        let safe_q = q.replace('\'', "''");
        let sql_preview = if q.is_empty() {
            format!("SELECT * FROM {} LIMIT 20;", idx.table)
        } else {
            format!(
                "SELECT *, fts_score({}, '{}') AS score\nFROM {}\nWHERE fts_match({}, '{}')\nORDER BY score DESC\nLIMIT 20;",
                idx.columns.join(", "),
                safe_q,
                idx.table,
                idx.columns.join(", "),
                safe_q
            )
        };

        let mut hits: Vec<DbFtsHit> = Vec::new();
        if valid_sql_ident(&idx.table) {
            let select_sql = format!("SELECT * FROM \"{}\"", idx.table);
            {
                let qr = self.db.query(&meta.id, &select_sql, vec![]).await?;
                idx.rows_indexed = qr.rows.len();
                for row_vals in qr.rows {
                    let mut full_obj = serde_json::Map::new();
                    let mut indexed_parts = Vec::new();
                    for (col_name, val) in qr.columns.iter().zip(row_vals) {
                        if idx.columns.iter().any(|c| c.eq_ignore_ascii_case(col_name)) {
                            match &val {
                                serde_json::Value::String(s) => indexed_parts.push(s.clone()),
                                serde_json::Value::Null => {}
                                other => indexed_parts.push(other.to_string()),
                            }
                        }
                        full_obj.insert(col_name.clone(), val);
                    }
                    let score_doc = serde_json::json!({
                        "title": full_obj.get("title").cloned().unwrap_or(serde_json::Value::Null),
                        "body": indexed_parts.join(" "),
                    });
                    let score = score_json_document(&score_doc, q, false);
                    if score > 0.0 {
                        hits.push(DbFtsHit {
                            row: serde_json::Value::Object(full_obj),
                            score,
                        });
                    }
                }
            }
        }

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(20);

        Ok(DbFtsQueryResult {
            index: idx,
            rows: hits,
            sql: sql_preview,
            took_ms: ((started.elapsed().as_secs_f64() * 1000.0) * 10.0).round() / 10.0,
        })
    }
}
