use std::future::Future;
use std::path::Path;
use std::time::Duration;

use anyhow::anyhow;
use turso::{Builder, Connection, Database, Error as TursoError, Value as TursoValue};
use tracing::warn;

use crate::core::model::{FerroError, Result, SqlValue};

/// Open a local Turso database, creating parent directories and enabling MVCC when supported.
pub async fn open_turso_db(path: &Path) -> Result<Database> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("failed to create db dir: {e}")))?;
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| FerroError::InvalidArgument("non-utf8 database path".into()))?;

    let db = Builder::new_local(path_str)
        .build()
        .await
        .map_err(|e| FerroError::Internal(anyhow!("failed to open turso db {path_str}: {e}")))?;

    let conn = db
        .connect()
        .map_err(|e| FerroError::Internal(anyhow!("failed to connect turso db {path_str}: {e}")))?;
    enable_mvcc_or_wal(&conn).await;
    Ok(db)
}

/// Configure MVCC journal mode (falling back gracefully to default WAL) and busy timeout.
pub async fn enable_mvcc_or_wal(conn: &Connection) {
    let _ = conn.busy_timeout(Duration::from_millis(5_000));
    if let Err(e) = conn.pragma_update("journal_mode", "'mvcc'").await {
        warn!(error = %e, "MVCC journal_mode pragma unavailable, keeping default journal mode");
    }
}

/// Connect to a Turso `Database` with a 5s busy timeout configured.
pub fn connect_db(db: &Database) -> Result<Connection> {
    let conn = db
        .connect()
        .map_err(|e| FerroError::Internal(anyhow!("turso connect error: {e}")))?;
    let _ = conn.busy_timeout(Duration::from_millis(5_000));
    Ok(conn)
}

pub fn is_retryable_turso_err(err: &TursoError) -> bool {
    match err {
        TursoError::Busy(_) | TursoError::BusySnapshot(_) => true,
        TursoError::Error(msg) => {
            let low = msg.to_ascii_lowercase();
            low.contains("conflict") || low.contains("busy") || low.contains("locked")
        }
        _ => false,
    }
}

pub fn map_turso_err(err: TursoError) -> FerroError {
    match err {
        TursoError::Constraint(msg) => FerroError::Conflict(msg),
        other => FerroError::Internal(anyhow!("turso error: {other}")),
    }
}

/// Retry an async Turso operation when MVCC commit conflict or transient busy error occurs.
pub async fn with_retry<T, F, Fut>(mut op: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<T, TursoError>>,
{
    let mut attempt = 0u32;
    loop {
        match op().await {
            Ok(v) => return Ok(v),
            Err(e) if is_retryable_turso_err(&e) && attempt < 8 => {
                attempt += 1;
                let backoff_ms = (1u64 << attempt.min(4)).min(25);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
            Err(e) => return Err(map_turso_err(e)),
        }
    }
}

/// Best-effort checkpoint and page cache flush for graceful shutdown or snapshot.
pub async fn checkpoint_db(db: &Database) {
    if let Ok(conn) = connect_db(db) {
        let _ = conn.pragma_update("wal_checkpoint", "TRUNCATE").await;
        let _ = conn.cacheflush();
    }
}

/// Copy a local Turso database file along with any existing WAL/log/shm sidecars.
pub async fn copy_db_with_sidecars(src: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("create snapshot parent: {e}")))?;
    }
    if src.exists() {
        tokio::fs::copy(src, dest)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("copy db snapshot: {e}")))?;
    }
    for suffix in ["-wal", "-log", "-shm"] {
        let sidecar = std::path::PathBuf::from(format!("{}{}", src.display(), suffix));
        if sidecar.exists() {
            let dest_sidecar =
                std::path::PathBuf::from(format!("{}{}", dest.display(), suffix));
            tokio::fs::copy(&sidecar, &dest_sidecar)
                .await
                .map_err(|e| FerroError::Internal(anyhow!("copy db sidecar: {e}")))?;
        }
    }
    Ok(())
}

pub fn sql_value_to_turso(v: &SqlValue) -> TursoValue {
    match v {
        SqlValue::Null => TursoValue::Null,
        SqlValue::Bool(b) => TursoValue::Integer(if *b { 1 } else { 0 }),
        SqlValue::Integer(i) => TursoValue::Integer(*i),
        SqlValue::Real(f) => TursoValue::Real(*f),
        SqlValue::Text(s) => TursoValue::Text(s.clone()),
        SqlValue::Blob(b) => TursoValue::Blob(b.clone()),
    }
}

pub fn turso_value_to_json(v: TursoValue) -> serde_json::Value {
    match v {
        TursoValue::Null => serde_json::Value::Null,
        TursoValue::Integer(i) => serde_json::json!(i),
        TursoValue::Real(f) => serde_json::json!(f),
        TursoValue::Text(s) => serde_json::Value::String(s),
        TursoValue::Blob(b) => serde_json::Value::String(format!("0x{}", hex::encode(b))),
    }
}
