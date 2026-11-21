use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};

use crate::core::model::{FerroError, QueryResult, Result, SqlValue};
use crate::core::traits::DbManager;
use crate::storage::tx::{
    checkpoint_db, connect_db, copy_db_with_sidecars, map_turso_err, open_turso_db,
    sql_value_to_turso, turso_value_to_json, with_retry,
};

struct CachedUserDb {
    db: Database,
    ddl_lock: Arc<RwLock<()>>,
    last_used: u64,
}

/// Manages per-project isolated Turso databases under `/data/dbs/<db_id>.db` with an LRU open-DB cap.
pub struct TursoUserDbManager {
    dbs_dir: PathBuf,
    max_open: usize,
    cache: Mutex<HashMap<String, CachedUserDb>>,
    seq: AtomicU64,
}

impl TursoUserDbManager {
    pub fn new(dbs_dir: PathBuf, max_open: usize) -> Result<Self> {
        std::fs::create_dir_all(&dbs_dir)
            .map_err(|e| FerroError::Internal(anyhow!("failed to create user dbs dir: {e}")))?;
        Ok(Self {
            dbs_dir,
            max_open: max_open.max(2),
            cache: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
        })
    }

    fn db_file_path(&self, db_id: &str) -> Result<PathBuf> {
        if db_id.is_empty()
            || !db_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(FerroError::InvalidArgument(format!(
                "invalid user db id '{db_id}'"
            )));
        }
        Ok(self.dbs_dir.join(format!("{db_id}.db")))
    }

    async fn get_or_open(&self, db_id: &str) -> Result<(Database, Arc<RwLock<()>>)> {
        let tick = self.seq.fetch_add(1, Ordering::Relaxed);
        {
            let mut guard = self.cache.lock().await;
            if let Some(entry) = guard.get_mut(db_id) {
                entry.last_used = tick;
                return Ok((entry.db.clone(), entry.ddl_lock.clone()));
            }
        }

        let path = self.db_file_path(db_id)?;
        let db = open_turso_db(&path).await?;
        let ddl_lock = Arc::new(RwLock::new(()));

        let mut guard = self.cache.lock().await;
        if let Some(entry) = guard.get_mut(db_id) {
            entry.last_used = tick;
            return Ok((entry.db.clone(), entry.ddl_lock.clone()));
        }
        if guard.len() >= self.max_open {
            if let Some(lru_id) = guard
                .iter()
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone())
            {
                if let Some(evicted) = guard.remove(&lru_id) {
                    tokio::spawn(async move {
                        checkpoint_db(&evicted.db).await;
                    });
                }
            }
        }
        guard.insert(
            db_id.to_string(),
            CachedUserDb {
                db: db.clone(),
                ddl_lock: ddl_lock.clone(),
                last_used: tick,
            },
        );
        Ok((db, ddl_lock))
    }
}

#[derive(Debug, Clone)]
struct SqlKeyword {
    word: String,
    followed_by_lparen: bool,
}

/// Tokenize SQL into `;`-separated statements, stripping comments, quoted strings/identifiers,
/// bind parameters, and dot-qualified member names, returning uppercase keyword tokens per statement.
fn tokenize_sql_statements(sql: &str) -> Vec<Vec<SqlKeyword>> {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut stmts: Vec<Vec<SqlKeyword>> = Vec::new();
    let mut current: Vec<SqlKeyword> = Vec::new();
    let mut after_dot = false;

    while i < len {
        let b = bytes[i];

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment: -- ...\n
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment: /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        // Statement separator
        if b == b';' {
            if !current.is_empty() {
                stmts.push(std::mem::take(&mut current));
            }
            after_dot = false;
            i += 1;
            continue;
        }

        // Single-quoted string or double-quoted / backtick identifier
        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            i += 1;
            while i < len {
                if bytes[i] == quote {
                    if i + 1 < len && bytes[i + 1] == quote {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            after_dot = false;
            continue;
        }

        // Bracket-quoted identifier: [...]
        if b == b'[' {
            i += 1;
            while i < len && bytes[i] != b']' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            after_dot = false;
            continue;
        }

        // Named bind parameters: :name, @name, $name
        if matches!(b, b':' | b'@' | b'$')
            && i + 1 < len
            && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_')
        {
            i += 2;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            after_dot = false;
            continue;
        }

        if b == b'.' {
            after_dot = true;
            i += 1;
            continue;
        }

        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
                i += 1;
            }
            if !after_dot {
                let word = std::str::from_utf8(&bytes[start..i])
                    .unwrap_or("")
                    .to_ascii_uppercase();
                // Peek past whitespace/comments to see if immediately followed by '(' (e.g. REPLACE(...)).
                let mut k = i;
                let mut followed_by_lparen = false;
                while k < len {
                    let c = bytes[k];
                    if c.is_ascii_whitespace() {
                        k += 1;
                    } else if c == b'-' && k + 1 < len && bytes[k + 1] == b'-' {
                        k += 2;
                        while k < len && bytes[k] != b'\n' {
                            k += 1;
                        }
                    } else if c == b'/' && k + 1 < len && bytes[k + 1] == b'*' {
                        k += 2;
                        while k + 1 < len && !(bytes[k] == b'*' && bytes[k + 1] == b'/') {
                            k += 1;
                        }
                        k = (k + 2).min(len);
                    } else {
                        followed_by_lparen = c == b'(';
                        break;
                    }
                }
                current.push(SqlKeyword {
                    word,
                    followed_by_lparen,
                });
            }
            after_dot = false;
            continue;
        }

        after_dot = false;
        i += 1;
    }

    if !current.is_empty() {
        stmts.push(current);
    }
    stmts
}

/// Validate that a SQL string passed to the read-only `query` endpoint is a single
/// non-mutating statement (`SELECT`, `EXPLAIN`, `VALUES`, or `WITH`).
pub fn validate_read_only_sql(sql: &str) -> Result<()> {
    let stmts = tokenize_sql_statements(sql);
    if stmts.is_empty() {
        return Err(FerroError::InvalidArgument(
            "SQL query must not be empty".into(),
        ));
    }
    if stmts.len() > 1 {
        return Err(FerroError::InvalidArgument(
            "read-only query accepts a single SQL statement".into(),
        ));
    }
    let tokens = &stmts[0];
    let first = tokens[0].word.as_str();
    if !matches!(first, "SELECT" | "EXPLAIN" | "VALUES" | "WITH") {
        return Err(FerroError::InvalidArgument(format!(
            "read-only query must start with SELECT, EXPLAIN, VALUES, or WITH (got '{first}')"
        )));
    }
    for tok in tokens {
        let w = tok.word.as_str();
        let is_mutating = matches!(
            w,
            "INSERT"
                | "UPDATE"
                | "DELETE"
                | "DROP"
                | "ALTER"
                | "CREATE"
                | "ATTACH"
                | "DETACH"
                | "VACUUM"
                | "REINDEX"
                | "PRAGMA"
                | "ANALYZE"
                | "BEGIN"
                | "COMMIT"
                | "ROLLBACK"
                | "SAVEPOINT"
                | "RELEASE"
        ) || (w == "REPLACE" && !tok.followed_by_lparen);
        if is_mutating {
            return Err(FerroError::InvalidArgument(format!(
                "mutating or control keyword '{w}' is not allowed in read-only query"
            )));
        }
    }
    Ok(())
}

fn is_ddl_or_pragma_sql(sql: &str) -> bool {
    tokenize_sql_statements(sql).iter().any(|tokens| {
        tokens.first().is_some_and(|kw| {
            matches!(
                kw.word.as_str(),
                "CREATE" | "ALTER" | "DROP" | "VACUUM" | "REINDEX" | "PRAGMA" | "ATTACH" | "DETACH"
            )
        })
    })
}

fn contains_ddl_or_pragma(stmts: &[(String, Vec<SqlValue>)]) -> bool {
    stmts.iter().any(|(sql, _)| is_ddl_or_pragma_sql(sql))
}

#[async_trait]
impl DbManager for TursoUserDbManager {
    async fn create_db(&self, db_id: &str) -> Result<()> {
        let (db, _lock) = self.get_or_open(db_id).await?;
        checkpoint_db(&db).await;
        Ok(())
    }

    async fn query(&self, db_id: &str, sql: &str, params: Vec<SqlValue>) -> Result<QueryResult> {
        validate_read_only_sql(sql)?;
        let (db, ddl_lock) = self.get_or_open(db_id).await?;
        let _read_guard = ddl_lock.read().await;
        let conn = connect_db(&db)?;
        let turso_params: Vec<TursoValue> = params.iter().map(sql_value_to_turso).collect();
        let mut rows = conn.query(sql, turso_params).await.map_err(map_turso_err)?;
        let columns = rows.column_names();
        let col_count = columns.len();

        let mut out_rows = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let mut vals = Vec::with_capacity(col_count);
            for i in 0..col_count {
                let v = row.get_value(i).map_err(map_turso_err)?;
                vals.push(turso_value_to_json(v));
            }
            out_rows.push(vals);
        }

        Ok(QueryResult {
            columns,
            rows: out_rows,
        })
    }

    async fn execute_batch(
        &self,
        db_id: &str,
        stmts: Vec<(String, Vec<SqlValue>)>,
    ) -> Result<u64> {
        if stmts.is_empty() {
            return Ok(0);
        }
        let (db, ddl_lock) = self.get_or_open(db_id).await?;
        let is_ddl = contains_ddl_or_pragma(&stmts);
        if is_ddl {
            let _write_guard = ddl_lock.write().await;
            let mut conn = connect_db(&db)?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await
                .map_err(map_turso_err)?;
            let mut total = 0u64;
            for (sql, params) in &stmts {
                let turso_params: Vec<TursoValue> = params.iter().map(sql_value_to_turso).collect();
                let n = tx
                    .execute(sql.as_str(), turso_params)
                    .await
                    .map_err(map_turso_err)?;
                if !is_ddl_or_pragma_sql(sql) {
                    total += n;
                }
            }
            tx.commit().await.map_err(map_turso_err)?;
            Ok(total)
        } else {
            let _read_guard = ddl_lock.read().await;
            with_retry(|| {
                let db = db.clone();
                let stmts = stmts.clone();
                async move {
                    let mut conn = db.connect()?;
                    let tx = conn
                        .transaction_with_behavior(TransactionBehavior::Concurrent)
                        .await?;
                    let mut total = 0u64;
                    for (sql, params) in &stmts {
                        let turso_params: Vec<TursoValue> =
                            params.iter().map(sql_value_to_turso).collect();
                        total += tx.execute(sql.as_str(), turso_params).await?;
                    }
                    tx.commit().await?;
                    Ok(total)
                }
            })
            .await
        }
    }

    async fn schema(&self, db_id: &str) -> Result<String> {
        let (db, ddl_lock) = self.get_or_open(db_id).await?;
        let _read_guard = ddl_lock.read().await;
        let conn = connect_db(&db)?;
        let mut rows = conn
            .query(
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type DESC, name ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut stmts = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let sql: String = row.get(0).map_err(map_turso_err)?;
            stmts.push(format!("{sql};"));
        }
        Ok(stmts.join("\n"))
    }

    async fn snapshot_to(&self, db_id: &str, dest_path: &Path) -> Result<()> {
        let (db, ddl_lock) = self.get_or_open(db_id).await?;
        let _write_guard = ddl_lock.write().await;
        checkpoint_db(&db).await;

        let src = self.db_file_path(db_id)?;
        copy_db_with_sidecars(&src, dest_path).await
    }

    async fn checkpoint_all(&self) -> Result<()> {
        let dbs: Vec<Database> = {
            let guard = self.cache.lock().await;
            guard.values().map(|v| v.db.clone()).collect()
        };
        for db in dbs {
            checkpoint_db(&db).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_sql_guard_allows_safe_queries() {
        assert!(validate_read_only_sql("SELECT 1").is_ok());
        assert!(validate_read_only_sql("-- leading comment\nSELECT * FROM posts;").is_ok());
        assert!(validate_read_only_sql("/* c */ WITH cte AS (SELECT 1) SELECT * FROM cte").is_ok());
        assert!(validate_read_only_sql("SELECT 'DELETE FROM x; DROP TABLE y' AS msg").is_ok());
        assert!(validate_read_only_sql("SELECT REPLACE(title, 'a', 'b'), t.update FROM posts t").is_ok());
        assert!(validate_read_only_sql("SELECT \"delete\" FROM posts WHERE id = :delete").is_ok());
        assert!(validate_read_only_sql("EXPLAIN QUERY PLAN SELECT * FROM posts").is_ok());
        assert!(validate_read_only_sql("VALUES (1, 2)").is_ok());
    }

    #[test]
    fn read_only_sql_guard_blocks_mutations_and_stacking() {
        assert!(validate_read_only_sql("").is_err());
        assert!(validate_read_only_sql("   -- only comment").is_err());
        assert!(validate_read_only_sql("DELETE FROM posts").is_err());
        assert!(validate_read_only_sql("/* c */ DELETE FROM posts").is_err());
        assert!(validate_read_only_sql("INSERT INTO posts (title) VALUES ('x') RETURNING id").is_err());
        assert!(validate_read_only_sql("UPDATE posts SET title = 'x' RETURNING *").is_err());
        assert!(validate_read_only_sql("REPLACE INTO posts VALUES (1, 'x')").is_err());
        assert!(validate_read_only_sql("DROP TABLE posts").is_err());
        assert!(validate_read_only_sql("PRAGMA writable_schema = ON").is_err());
        assert!(validate_read_only_sql("ATTACH DATABASE '/tmp/x.db' AS x").is_err());
        assert!(validate_read_only_sql("SELECT 1; DROP TABLE posts").is_err());
        assert!(validate_read_only_sql("WITH d AS (DELETE FROM posts RETURNING *) SELECT * FROM d").is_err());
        assert!(validate_read_only_sql("EXPLAIN DELETE FROM posts").is_err());
    }

    #[test]
    fn ddl_detection_handles_comments_and_multi_statements() {
        assert!(contains_ddl_or_pragma(&[(
            "-- setup\nCREATE TABLE a (id INT)".into(),
            vec![]
        )]));
        assert!(contains_ddl_or_pragma(&[(
            "INSERT INTO a VALUES (1); DROP TABLE a".into(),
            vec![]
        )]));
        assert!(!contains_ddl_or_pragma(&[(
            "INSERT INTO a (txt) VALUES ('CREATE TABLE fake; DROP TABLE fake')".into(),
            vec![]
        )]));
    }
}
