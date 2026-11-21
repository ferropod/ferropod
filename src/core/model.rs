use std::collections::HashMap;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Unified error type across core services and adapters.
#[derive(Debug, Error)]
pub enum FerroError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("already exists: {0}")]
    Conflict(String),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("backpressure / rate limited: {0}")]
    RateLimited(String),
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, FerroError>;

/// Logical binding declarations for a deployment (logical name -> physical resource name).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Bindings {
    #[serde(default)]
    pub kv: HashMap<String, String>,
    #[serde(default)]
    pub db: HashMap<String, String>,
    #[serde(default)]
    pub queue: HashMap<String, String>,
    #[serde(default)]
    pub bucket: HashMap<String, String>,
    #[serde(default)]
    pub search: HashMap<String, String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Project entity stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub active_deployment_id: Option<String>,
    pub preview_deployment_id: Option<String>,
    pub max_concurrency: i64,
    pub access_policy_json: Option<String>,
    pub health_check_path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Immutable deployment version record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deployment {
    pub id: String,
    pub project_id: String,
    pub artifact_path: Option<String>,
    pub assets_bucket: Option<String>,
    pub assets_prefix: Option<String>,
    pub spa_fallback: bool,
    pub bindings: Bindings,
    pub crons: Vec<String>,
    pub status: String,
    pub created_at: i64,
}

/// Target environment for a routed hostname.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetEnv {
    Prod,
    Preview,
}

impl TargetEnv {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Prod => "prod",
            Self::Preview => "preview",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "prod" => Ok(Self::Prod),
            "preview" => Ok(Self::Preview),
            other => Err(FerroError::InvalidArgument(format!(
                "invalid target_env '{other}', expected 'prod' or 'preview'"
            ))),
        }
    }
}

/// Hostname-to-project route mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
}

/// Cached entry in the hot in-memory routing table.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
    pub active_deployment: Option<Deployment>,
}

/// API / MCP access token metadata (raw secret is never persisted).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub scope: TokenScope,
    pub expires_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub created_at: i64,
}

/// Token authorization scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
    Admin,
    Project(String),
    Readonly,
}

impl TokenScope {
    pub fn as_str(&self) -> String {
        match self {
            Self::Admin => "admin".to_string(),
            Self::Project(id) => format!("project:{id}"),
            Self::Readonly => "readonly".to_string(),
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        if s == "admin" {
            Ok(Self::Admin)
        } else if s == "readonly" {
            Ok(Self::Readonly)
        } else if let Some(pid) = s.strip_prefix("project:") {
            if pid.is_empty() {
                return Err(FerroError::InvalidArgument("empty project scope".into()));
            }
            Ok(Self::Project(pid.to_string()))
        } else {
            Err(FerroError::InvalidArgument(format!(
                "invalid scope '{s}', expected 'admin', 'readonly', or 'project:<id>'"
            )))
        }
    }

    pub fn allows_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn allows_project_read(&self, project_id: &str) -> bool {
        match self {
            Self::Admin | Self::Readonly => true,
            Self::Project(pid) => pid == project_id,
        }
    }

    pub fn allows_project_write(&self, project_id: &str) -> bool {
        match self {
            Self::Admin => true,
            Self::Project(pid) => pid == project_id,
            Self::Readonly => false,
        }
    }

    pub fn allows_resource_read(&self, owner_project_id: Option<&str>) -> bool {
        match self {
            Self::Admin | Self::Readonly => true,
            Self::Project(pid) => owner_project_id == Some(pid.as_str()),
        }
    }

    pub fn allows_resource_write(&self, owner_project_id: Option<&str>) -> bool {
        match self {
            Self::Admin => true,
            Self::Project(pid) => owner_project_id == Some(pid.as_str()),
            Self::Readonly => false,
        }
    }

    pub fn allows_read(&self) -> bool {
        true
    }
}

/// Authenticated caller identity passed into `PlatformService` by all adapters
/// (REST API, Blob API, CLI, future MCP server, and future Wasmtime host bindings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub actor: String,
    pub scope: TokenScope,
}

impl Caller {
    pub fn from_token(token: &ApiToken) -> Self {
        Self {
            actor: token.name.clone(),
            scope: token.scope.clone(),
        }
    }

    pub fn system(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Admin,
        }
    }

    pub fn project(actor: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Project(project_id.into()),
        }
    }

    pub fn readonly(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: TokenScope::Readonly,
        }
    }

    pub fn project_id(&self) -> Option<&str> {
        match &self.scope {
            TokenScope::Project(pid) => Some(pid.as_str()),
            _ => None,
        }
    }
}

/// Authorization context for Blob operations (supports Bearer caller, HMAC presigned URL, or anonymous public read).
#[derive(Debug, Clone)]
pub enum BlobAuth<'a> {
    Caller(&'a Caller),
    Presigned { op: &'a str, exp: i64, sig: &'a str },
    Anonymous,
}

/// KV namespace ownership metadata stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvNamespaceMeta {
    pub name: String,
    pub project_id: Option<String>,
    pub created_at: i64,
}

/// User database registry metadata stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDbMeta {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub created_at: i64,
}

/// Stored KV entry value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvValue {
    pub value: Vec<u8>,
    pub metadata: Option<String>,
    pub expires_at: Option<i64>,
    pub updated_at: i64,
}

/// Key metadata returned by KV list operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvKeyMeta {
    pub key: String,
    pub size: usize,
    pub metadata: Option<String>,
    pub expires_at: Option<i64>,
    pub updated_at: i64,
}

/// Persistent queue job lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Dead,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Dead => "dead",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "dead" => Ok(Self::Dead),
            other => Err(FerroError::InvalidArgument(format!(
                "unknown job status '{other}'"
            ))),
        }
    }
}

/// Queue job record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: i64,
    pub queue: String,
    pub payload: Vec<u8>,
    pub status: JobStatus,
    pub priority: i32,
    pub run_at: i64,
    pub lease_until: i64,
    pub attempts: i32,
    pub max_attempts: i32,
    pub dedup_key: Option<String>,
    pub last_error: Option<String>,
    pub created_at: i64,
}

/// Options when enqueuing a job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnqueueOpts {
    pub delay_secs: Option<u64>,
    pub priority: Option<i32>,
    pub dedup_key: Option<String>,
    pub max_attempts: Option<i32>,
}

/// Queue configuration metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    pub name: String,
    pub project_id: Option<String>,
    pub consumer_project_id: Option<String>,
    pub max_concurrency: i64,
    pub rate_limit_per_sec: Option<i64>,
    pub batch_size: i64,
    pub lease_secs: i64,
    pub max_attempts: i64,
    pub created_at: i64,
}

/// Queue depth counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueueStats {
    pub queue: String,
    pub pending: i64,
    pub running: i64,
    pub dead: i64,
}

/// Native Blob bucket metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BucketMeta {
    pub name: String,
    pub project_id: Option<String>,
    pub public_read: bool,
    pub created_at: i64,
}

/// Native Blob object metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    pub bucket: String,
    pub key: String,
    pub sha256: String,
    pub blob_path: String,
    pub size: u64,
    pub content_type: String,
    pub custom_meta: Option<String>,
    pub updated_at: i64,
}

/// Typed SQL parameter / cell value for user Turso DBs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SqlValue {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// Tabular SQL query result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

/// Log entry emitted across access/runtime/queue/cron/audit/system pipelines.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub id: Option<i64>,
    pub ts: i64,
    pub project_id: Option<String>,
    pub request_id: Option<String>,
    pub source: String,
    pub level: String,
    pub message: String,
    pub meta_json: Option<String>,
}

/// Log search filter.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LogQuery {
    pub project_id: Option<String>,
    pub request_id: Option<String>,
    pub source: Option<String>,
    pub level: Option<String>,
    pub since_ts: Option<i64>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub q: Option<String>,
}

/// Byte stream type used by [`crate::core::traits::ObjectStore`].
pub type ByteStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send>>;

/// Inclusive byte range `(start, end)` for HTTP `Range` responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: Option<u64>,
}

/// Worker execution context passed to `WorkerRuntime`.
#[derive(Debug, Clone)]
pub struct RequestCtx {
    pub project_id: String,
    pub request_id: String,
    pub deployment: Deployment,
    pub cron_job: Option<CronJob>,
}

impl RequestCtx {
    pub fn new(
        project_id: impl Into<String>,
        request_id: impl Into<String>,
        deployment: Deployment,
    ) -> Self {
        Self {
            project_id: project_id.into(),
            request_id: request_id.into(),
            deployment,
            cron_job: None,
        }
    }

    pub fn with_cron_job(mut self, job: CronJob) -> Self {
        self.cron_job = Some(job);
        self
    }
}

/// Transport-agnostic HTTP request passed to `WorkerRuntime` (avoids coupling `core` to `axum`).
pub struct WorkerHttpRequest {
    pub method: String,
    pub uri: String,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: ByteStream,
}

/// Transport-agnostic HTTP response returned by `WorkerRuntime`.
pub struct WorkerHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: ByteStream,
}

/// Outcome for a batch-processed job (used in M2 queue worker).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobOutcome {
    Ack,
    Retry { delay_secs: u64, error: String },
    Dead { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub schedule: String,
    pub target: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFieldSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub tokenizer: String,
    pub stored: bool,
    pub indexed: bool,
    pub fast: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndexMeta {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub tokenizer: String,
    pub doc_count: usize,
    pub segments: usize,
    pub disk_bytes: u64,
    pub writer_budget_mb: usize,
    pub created_at: i64,
    pub updated_at: i64,
    pub schema: Vec<SearchFieldSchema>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQueryInput {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub fuzzy: bool,
    pub sort: Option<String>,
    pub limit: Option<usize>,
    #[serde(default)]
    pub facets: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc: serde_json::Value,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQueryResult {
    pub hits: Vec<SearchHit>,
    pub total: usize,
    pub took_ms: f64,
    pub facets: HashMap<String, HashMap<String, usize>>,
    pub tokenizer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsIndexMeta {
    pub db_id: String,
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub tokenizer: String,
    pub rows_indexed: usize,
    pub size_bytes: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsHit {
    pub row: serde_json::Value,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbFtsQueryResult {
    pub index: DbFtsIndexMeta,
    pub rows: Vec<DbFtsHit>,
    pub sql: String,
    pub took_ms: f64,
}
