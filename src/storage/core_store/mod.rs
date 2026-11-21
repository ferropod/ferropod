pub mod blob_meta;
pub mod projects;
pub mod registry;
pub mod security;

use std::path::{Path, PathBuf};

use turso::{Database, Value as TursoValue};

use crate::core::model::{Bindings, Deployment, FerroError, ObjectMeta, Project, Result};
use crate::storage::tx::{
    checkpoint_db, connect_db, ensure_resource_group_column, map_turso_err, open_turso_db,
};

const CORE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    active_deployment_id TEXT,
    preview_deployment_id TEXT,
    max_concurrency INTEGER NOT NULL DEFAULT 64,
    access_policy_json TEXT,
    health_check_path TEXT NOT NULL DEFAULT '/health',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    resource_group TEXT
);

CREATE TABLE IF NOT EXISTS production_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    deployment_id TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_production_history ON production_history (project_id, id DESC);

CREATE TABLE IF NOT EXISTS deployments (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    artifact_path TEXT,
    assets_bucket TEXT,
    assets_prefix TEXT,
    spa_fallback INTEGER NOT NULL DEFAULT 0,
    bindings_json TEXT NOT NULL,
    crons_json TEXT,
    status TEXT NOT NULL DEFAULT 'ready',
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_deployments_project ON deployments (project_id, created_at DESC);

CREATE TABLE IF NOT EXISTS routes (
    host TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    target_env TEXT NOT NULL DEFAULT 'prod'
);

CREATE TABLE IF NOT EXISTS secrets (
    project_id TEXT NOT NULL,
    key TEXT NOT NULL,
    nonce BLOB NOT NULL,
    ciphertext BLOB NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project_id, key)
);

CREATE TABLE IF NOT EXISTS api_tokens (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_hash TEXT UNIQUE NOT NULL,
    scope TEXT NOT NULL,
    expires_at INTEGER,
    last_used_at INTEGER,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS kv_namespaces (
    name TEXT PRIMARY KEY,
    project_id TEXT,
    created_at INTEGER NOT NULL,
    resource_group TEXT
);

CREATE TABLE IF NOT EXISTS blob_buckets (
    name TEXT PRIMARY KEY,
    project_id TEXT,
    public_read INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    resource_group TEXT
);

CREATE TABLE IF NOT EXISTS blob_objects (
    bucket TEXT NOT NULL,
    key TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    blob_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'application/octet-stream',
    custom_meta TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (bucket, key)
);
CREATE INDEX IF NOT EXISTS idx_blob_sha ON blob_objects (sha256);

CREATE TABLE IF NOT EXISTS blob_gc (
    blob_path TEXT PRIMARY KEY,
    delete_after INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_databases (
    id TEXT PRIMARY KEY,
    name TEXT UNIQUE NOT NULL,
    project_id TEXT,
    created_at INTEGER NOT NULL,
    resource_group TEXT
);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    project_id TEXT,
    schedule TEXT NOT NULL,
    target TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run_at INTEGER,
    last_status TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS user_db_fts_indexes (
    db_id TEXT NOT NULL,
    name TEXT NOT NULL,
    table_name TEXT NOT NULL,
    columns_json TEXT NOT NULL,
    tokenizer TEXT NOT NULL DEFAULT 'ngram',
    created_at INTEGER NOT NULL,
    PRIMARY KEY (db_id, name)
);
"#;

/// Turso MVCC store for `core.db` (control-plane metadata: projects, routes, secrets, tokens, KV namespace
/// ownership, blob index, user DB registry, cron jobs). User KV entries live in the separate `kv.db`.
#[derive(Clone)]
pub struct TursoCoreStore {
    pub(super) db: Database,
    pub(super) db_path: PathBuf,
}

impl TursoCoreStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = open_turso_db(path).await?;
        let conn = connect_db(&db)?;
        conn.execute_batch(CORE_SCHEMA)
            .await
            .map_err(map_turso_err)?;
        for table in [
            "projects",
            "kv_namespaces",
            "user_databases",
            "blob_buckets",
        ] {
            ensure_resource_group_column(&conn, table).await?;
        }
        Ok(Self {
            db,
            db_path: path.to_path_buf(),
        })
    }

    pub async fn checkpoint(&self) {
        checkpoint_db(&self.db).await;
    }

    /// Underlying `core.db` handle (used for one-time legacy migrations).
    pub fn database(&self) -> &Database {
        &self.db
    }
}

pub(crate) fn opt_text(row: &turso::Row, idx: usize) -> Result<Option<String>> {
    match row.get_value(idx).map_err(map_turso_err)? {
        TursoValue::Null => Ok(None),
        TursoValue::Text(s) => Ok(Some(s)),
        other => Err(FerroError::Internal(anyhow::anyhow!(
            "unexpected column {idx} type for text: {other:?}"
        ))),
    }
}

pub(crate) fn opt_i64(row: &turso::Row, idx: usize) -> Result<Option<i64>> {
    match row.get_value(idx).map_err(map_turso_err)? {
        TursoValue::Null => Ok(None),
        TursoValue::Integer(i) => Ok(Some(i)),
        other => Err(FerroError::Internal(anyhow::anyhow!(
            "unexpected column {idx} type for i64: {other:?}"
        ))),
    }
}

pub(crate) fn opt_text_param(s: Option<String>) -> TursoValue {
    match s {
        Some(v) => TursoValue::Text(v),
        None => TursoValue::Null,
    }
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub(super) fn row_to_project(row: &turso::Row) -> Result<Project> {
    Ok(Project {
        id: row.get(0).map_err(map_turso_err)?,
        name: row.get(1).map_err(map_turso_err)?,
        active_deployment_id: opt_text(row, 2)?,
        preview_deployment_id: opt_text(row, 3)?,
        max_concurrency: row.get(4).map_err(map_turso_err)?,
        access_policy_json: opt_text(row, 5)?,
        health_check_path: row.get(6).map_err(map_turso_err)?,
        created_at: row.get(7).map_err(map_turso_err)?,
        updated_at: row.get(8).map_err(map_turso_err)?,
        group: opt_text(row, 9)?,
    })
}

pub(super) fn row_to_deployment(row: &turso::Row) -> Result<Deployment> {
    let bindings_raw: String = row.get(6).map_err(map_turso_err)?;
    let bindings: Bindings = serde_json::from_str(&bindings_raw)
        .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode bindings_json: {e}")))?;
    let crons_raw = opt_text(row, 7)?;
    let crons: Vec<String> = match crons_raw {
        Some(raw) => serde_json::from_str(&raw)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode crons_json: {e}")))?,
        None => Vec::new(),
    };
    Ok(Deployment {
        id: row.get(0).map_err(map_turso_err)?,
        project_id: row.get(1).map_err(map_turso_err)?,
        artifact_path: opt_text(row, 2)?,
        assets_bucket: opt_text(row, 3)?,
        assets_prefix: opt_text(row, 4)?,
        spa_fallback: row.get::<i64>(5).map_err(map_turso_err)? != 0,
        bindings,
        crons,
        status: row.get(8).map_err(map_turso_err)?,
        created_at: row.get(9).map_err(map_turso_err)?,
    })
}

pub(super) fn row_to_object_meta(row: &turso::Row) -> Result<ObjectMeta> {
    Ok(ObjectMeta {
        bucket: row.get(0).map_err(map_turso_err)?,
        key: row.get(1).map_err(map_turso_err)?,
        sha256: row.get(2).map_err(map_turso_err)?,
        blob_path: row.get(3).map_err(map_turso_err)?,
        size: row.get::<i64>(4).map_err(map_turso_err)? as u64,
        content_type: row.get(5).map_err(map_turso_err)?,
        custom_meta: opt_text(row, 6)?,
        updated_at: row.get(7).map_err(map_turso_err)?,
    })
}
