use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Utc;
use uuid::Uuid;

use crate::core::model::{
    Caller, DbFtsHit, DbFtsIndexMeta, DbFtsQueryResult, FerroError, QueryResult, Result, SqlValue,
    TokenScope, UserDbMeta, UserDbOpts,
};
use crate::storage::search_store::score_json_document;

use super::PlatformService;

fn valid_sql_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn starts_with_sql_keyword(sql: &str, kw: &str) -> bool {
    let trimmed = sql.trim_start();
    trimmed
        .get(..kw.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(kw))
}

impl PlatformService {
    async fn resolve_user_db(&self, id_or_name: &str) -> Result<UserDbMeta> {
        if let Some(meta) = self.caches.db_meta.get(id_or_name) {
            return Ok(meta);
        }
        let meta = self
            .registry
            .get_user_db(id_or_name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("user db '{id_or_name}'")))?;
        self.caches.db_meta.insert(meta.id.clone(), meta.clone());
        self.caches.db_meta.insert(meta.name.clone(), meta.clone());
        Ok(meta)
    }

    fn enforce_db_rate_limit(&self, meta: &UserDbMeta, actor: &str) -> Result<()> {
        self.enforce_service_rate_limit(
            &format!("db:{}:{}", meta.id, actor),
            meta.rate_limit_rpm,
            super::DEFAULT_SERVICE_RATE_LIMIT_RPM,
            &format!("database '{}'", meta.name),
        )
    }

    async fn count_user_db_rows(&self, db_id: &str) -> Result<usize> {
        let tables_qr = self
            .db
            .query(db_id, "SELECT name, type FROM sqlite_master", vec![])
            .await?;
        let mut total_rows = 0usize;
        for row in &tables_qr.rows {
            let tbl = row.first().and_then(|v| v.as_str()).unwrap_or("");
            let typ = row.get(1).and_then(|v| v.as_str()).unwrap_or("");
            if typ.eq_ignore_ascii_case("table")
                && !tbl.starts_with("sqlite_")
                && !tbl.starts_with("__turso_")
                && !tbl.starts_with("_turso_")
                && valid_sql_ident(tbl)
            {
                let cnt_sql = format!("SELECT COUNT(*) FROM \"{tbl}\"");
                if let Ok(cqr) = self.db.query(db_id, &cnt_sql, vec![]).await {
                    let n = cqr
                        .rows
                        .first()
                        .and_then(|r| r.first())
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    total_rows = total_rows.saturating_add(n.max(0) as usize);
                }
            }
        }
        Ok(total_rows)
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
        let meta = self
            .create_user_db_with_opts(
                caller,
                name,
                UserDbOpts {
                    group: group.map(str::to_owned),
                    project_id: project_id.map(str::to_owned),
                    ..Default::default()
                },
            )
            .await?;
        Ok(meta.id)
    }

    pub async fn create_user_db_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        mut opts: UserDbOpts,
    ) -> Result<UserDbMeta> {
        opts.group = super::normalize_resource_group(opts.group.as_deref())?.map(str::to_owned);
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "database name cannot be empty".into(),
            ));
        }
        let resolved_pid = self
            .resolve_resource_owner(caller, opts.project_id.as_deref(), "databases")
            .await?;
        opts.project_id = resolved_pid.clone();
        let init_sql = opts
            .init_sql
            .take()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let id = format!("db_{}", Uuid::new_v4().simple());
        self.registry
            .register_user_db_with_opts(&id, name, opts)
            .await?;
        if let Err(e) = self.db.create_db(&id).await {
            let _ = self.registry.delete_user_db(&id).await;
            return Err(e);
        }
        if let Some(sql) = init_sql {
            let stmts: Vec<String> = sql
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            for stmt in stmts {
                if let Err(e) = self.db.execute_batch(&id, vec![(stmt, Vec::new())]).await {
                    let _ = self.db.delete_db(&id).await;
                    let _ = self.registry.delete_user_db(&id).await;
                    return Err(e);
                }
            }
        }
        self.invalidate_db_meta_cache();
        let meta = self.resolve_user_db(&id).await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "db.create",
            serde_json::json!({ "id": id, "name": name }),
        );
        Ok(meta)
    }

    pub async fn update_user_db(
        &self,
        caller: &Caller,
        id_or_name: &str,
        opts: UserDbOpts,
    ) -> Result<UserDbMeta> {
        let mut meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        if let Some(g) = opts.group {
            meta.group = super::normalize_resource_group(Some(&g))?.map(str::to_owned);
        }
        if let Some(desc) = opts.description {
            let trimmed = desc.trim();
            meta.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(mr) = opts.max_rows {
            meta.max_rows = if mr == 0 { None } else { Some(mr) };
        }
        if let Some(ms) = opts.max_size_mb {
            meta.max_size_mb = if ms == 0 { None } else { Some(ms) };
        }
        if let Some(fk) = opts.foreign_keys {
            meta.foreign_keys = fk;
        }
        if let Some(ro) = opts.read_only {
            meta.read_only = ro;
        }
        if let Some(qt) = opts.query_timeout_ms {
            meta.query_timeout_ms = if qt == 0 { None } else { Some(qt) };
        }
        if let Some(rpm) = opts.rate_limit_rpm {
            meta.rate_limit_rpm = if rpm == 0 { None } else { Some(rpm) };
        }
        self.registry.update_user_db(&meta).await?;
        self.invalidate_db_meta_cache();
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "db.update",
            serde_json::json!({ "id": meta.id, "name": meta.name, "read_only": meta.read_only }),
        );
        Ok(meta)
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
        self.invalidate_db_caches(&meta.id);
        self.invalidate_db_meta_cache();
        self.remove_db_concurrency(&meta.id);
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
        self.enforce_db_rate_limit(&meta, &caller.actor)?;
        let _permit = self.acquire_db_permit(&meta.id, &meta.name).await?;
        let row_cap = meta.max_rows.filter(|r| *r > 0).unwrap_or(10_000);
        let timeout_ms = meta.query_timeout_ms.filter(|t| *t > 0).unwrap_or(15_000);
        let res = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.db.query_limited(&meta.id, sql, params, Some(row_cap)),
        )
        .await
        .map_err(|_| {
            FerroError::InvalidArgument(format!(
                "database '{}' query timed out after {timeout_ms}ms",
                meta.name
            ))
        })??;
        Ok(res)
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
        self.enforce_db_rate_limit(&meta, &caller.actor)?;
        if meta.read_only {
            return Err(FerroError::Forbidden(format!(
                "database '{}' is configured as read-only",
                meta.name
            )));
        }
        if stmts.len() > super::MAX_DB_BATCH_STATEMENTS {
            return Err(FerroError::InvalidArgument(format!(
                "database '{}' batch has {} statements (max {})",
                meta.name,
                stmts.len(),
                super::MAX_DB_BATCH_STATEMENTS
            )));
        }
        let _permit = self.acquire_db_permit(&meta.id, &meta.name).await?;
        let max_mb = meta
            .max_size_mb
            .filter(|m| *m > 0)
            .unwrap_or(super::DEFAULT_DB_MAX_SIZE_MB);
        if max_mb > 0 {
            let db_path = self.config.user_dbs_dir().join(format!("{}.db", meta.id));
            let wal_path = self
                .config
                .user_dbs_dir()
                .join(format!("{}.db-wal", meta.id));
            let main_len = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);
            let wal_len = std::fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
            if main_len.saturating_add(wal_len) > (max_mb as u64).saturating_mul(1024 * 1024) {
                return Err(FerroError::InvalidArgument(format!(
                    "database '{}' exceeded max_size_mb quota ({max_mb} MiB)",
                    meta.name
                )));
            }
        }
        let timeout_ms = meta.query_timeout_ms.filter(|t| *t > 0).unwrap_or(15_000);
        let insert_count = stmts
            .iter()
            .filter(|(s, _)| starts_with_sql_keyword(s, "INSERT"))
            .count();
        let has_delete_or_ddl = stmts.iter().any(|(s, _)| {
            starts_with_sql_keyword(s, "DELETE")
                || starts_with_sql_keyword(s, "DROP")
                || starts_with_sql_keyword(s, "TRUNCATE")
        });
        if let Some(max_r) = meta.max_rows.filter(|r| *r > 0) {
            if insert_count > 0 {
                let fresh_cached = {
                    let cache = self
                        .caches
                        .db_row_counts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    cache.get(&meta.id).and_then(|(ts, cnt)| {
                        if ts.elapsed() < Duration::from_secs(5) {
                            Some(*cnt)
                        } else {
                            None
                        }
                    })
                };
                if let Some(cached_rows) = fresh_cached {
                    if cached_rows >= max_r {
                        return Err(FerroError::InvalidArgument(format!(
                            "database '{}' exceeded max_rows quota ({max_r})",
                            meta.name
                        )));
                    }
                    let mut cache = self
                        .caches
                        .db_row_counts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if let Some((_, cnt)) = cache.get_mut(&meta.id) {
                        *cnt = cnt.saturating_add(insert_count);
                    }
                } else {
                    let total_rows = tokio::time::timeout(
                        Duration::from_millis(timeout_ms),
                        self.count_user_db_rows(&meta.id),
                    )
                    .await
                    .map_err(|_| {
                        FerroError::InvalidArgument(format!(
                            "database '{}' execute timed out after {timeout_ms}ms",
                            meta.name
                        ))
                    })??;
                    if total_rows >= max_r {
                        let mut cache = self
                            .caches
                            .db_row_counts
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        cache.insert(meta.id.clone(), (Instant::now(), total_rows));
                        return Err(FerroError::InvalidArgument(format!(
                            "database '{}' exceeded max_rows quota ({max_r})",
                            meta.name
                        )));
                    }
                    let mut cache = self
                        .caches
                        .db_row_counts
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    cache.insert(
                        meta.id.clone(),
                        (Instant::now(), total_rows.saturating_add(insert_count)),
                    );
                }
            }
        }
        let affected = tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            self.db.execute_batch(&meta.id, stmts),
        )
        .await
        .map_err(|_| {
            FerroError::InvalidArgument(format!(
                "database '{}' execute timed out after {timeout_ms}ms",
                meta.name
            ))
        })??;
        if has_delete_or_ddl {
            self.invalidate_db_caches(&meta.id);
        }
        Ok(affected)
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
        self.enforce_db_rate_limit(&meta, &caller.actor)?;
        let _permit = self.acquire_db_permit(&meta.id, &meta.name).await?;
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

        if !valid_sql_ident(&idx.table) {
            return Err(FerroError::InvalidArgument(
                "invalid FTS table identifier".into(),
            ));
        }
        let has_title_col = idx.columns.iter().any(|c| c.eq_ignore_ascii_case("title"));
        let mut hits: Vec<DbFtsHit> = Vec::new();
        let mut rows_indexed = 0usize;
        let select_sql = format!("SELECT * FROM \"{}\"", idx.table);
        self.db
            .visit_rows(&meta.id, &select_sql, &mut |columns, values| {
                rows_indexed += 1;
                let mut title_val = serde_json::Value::Null;
                let mut indexed_parts = Vec::new();
                for (col_name, val) in columns.iter().zip(values.iter()) {
                    if idx.columns.iter().any(|c| c.eq_ignore_ascii_case(col_name)) {
                        if has_title_col && col_name.eq_ignore_ascii_case("title") {
                            title_val = val.clone();
                        }
                        match val {
                            serde_json::Value::String(value) => indexed_parts.push(value.clone()),
                            serde_json::Value::Null => {}
                            other => indexed_parts.push(other.to_string()),
                        }
                    }
                }
                let score_doc = serde_json::json!({
                    "title": title_val,
                    "body": indexed_parts.join(" "),
                });
                let score = score_json_document(&score_doc, q, false);
                if score > 0.0 {
                    let should_insert =
                        hits.len() < 20 || hits.last().is_some_and(|last| score > last.score);
                    if should_insert {
                        let mut full_obj = serde_json::Map::with_capacity(columns.len());
                        for (col_name, val) in columns.iter().zip(values.iter()) {
                            full_obj.insert(col_name.clone(), val.clone());
                        }
                        let pos = hits.partition_point(|h| h.score >= score);
                        hits.insert(
                            pos,
                            DbFtsHit {
                                row: serde_json::Value::Object(full_obj),
                                score,
                            },
                        );
                        hits.truncate(20);
                    }
                }
            })
            .await?;
        idx.rows_indexed = rows_indexed;

        Ok(DbFtsQueryResult {
            index: idx,
            rows: hits,
            sql: sql_preview,
            took_ms: ((started.elapsed().as_secs_f64() * 1000.0) * 10.0).round() / 10.0,
        })
    }
}
