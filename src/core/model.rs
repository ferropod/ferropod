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

/// Default KV entry TTL (7 days = 604,800 seconds). KV entries always expire.
pub const DEFAULT_KV_TTL_SECS: u64 = 7 * 86_400;

pub fn default_kv_ttl_secs() -> u64 {
    DEFAULT_KV_TTL_SECS
}

pub fn default_retry_backoff_secs() -> i64 {
    5
}

pub fn default_utc_timezone() -> String {
    "UTC".to_string()
}

/// Project entity stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub group: Option<String>,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub active_deployment_id: Option<String>,
    pub preview_deployment_id: Option<String>,
    pub max_concurrency: i64,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub wasm_max_memory_mb: Option<usize>,
    #[serde(default)]
    pub allow_private_outbound: bool,
    pub access_policy_json: Option<String>,
    pub health_check_path: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Options for creating or updating a project/app.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectOpts {
    pub group: Option<String>,
    pub description: Option<String>,
    pub health_check_path: Option<String>,
    pub max_concurrency: Option<i64>,
    pub timeout_ms: Option<u64>,
    pub wasm_max_memory_mb: Option<usize>,
    pub allow_private_outbound: Option<bool>,
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetEnv {
    #[default]
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

/// Hostname-to-project route mapping with optional reverse proxy, rate limit, Basic Auth, and canary split.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Route {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
    #[serde(default)]
    pub proxy_prefix: Option<String>,
    #[serde(default)]
    pub proxy_upstream: Option<String>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
    #[serde(default)]
    pub basic_auth_user: Option<String>,
    #[serde(default, skip_serializing)]
    pub basic_auth_pass_hash: Option<String>,
    #[serde(default)]
    pub basic_auth_enabled: bool,
    #[serde(default)]
    pub canary_deployment_id: Option<String>,
    #[serde(default)]
    pub canary_weight: u8,
}

/// Optional route policy overrides for creating/updating a hostname route.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RouteOpts {
    pub proxy_prefix: Option<String>,
    pub proxy_upstream: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub basic_auth_user: Option<String>,
    pub basic_auth_password: Option<String>,
    pub clear_basic_auth: Option<bool>,
    pub canary_deployment_id: Option<String>,
    pub canary_weight: Option<u8>,
}

/// Cached entry in the hot in-memory routing table.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    pub host: String,
    pub project_id: String,
    pub target_env: TargetEnv,
    pub active_deployment: Option<Deployment>,
    pub proxy_prefix: Option<String>,
    pub proxy_upstream: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub basic_auth_user: Option<String>,
    pub basic_auth_pass_hash: Option<String>,
    pub canary_deployment: Option<Deployment>,
    pub canary_weight: u8,
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

/// KV namespace ownership and policy metadata stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KvNamespaceMeta {
    #[serde(default)]
    pub group: Option<String>,
    pub name: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_kv_ttl_secs")]
    pub default_ttl_secs: u64,
    #[serde(default)]
    pub default_ephemeral: bool,
    #[serde(default)]
    pub max_value_bytes: Option<usize>,
    #[serde(default)]
    pub max_keys: Option<usize>,
    pub created_at: i64,
}

/// Options for creating or updating a KV namespace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KvNamespaceOpts {
    pub group: Option<String>,
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub default_ttl_secs: Option<u64>,
    pub default_ephemeral: Option<bool>,
    pub max_value_bytes: Option<usize>,
    pub max_keys: Option<usize>,
}

/// User database registry metadata stored in `core.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDbMeta {
    #[serde(default)]
    pub group: Option<String>,
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub max_rows: Option<usize>,
    #[serde(default)]
    pub max_size_mb: Option<usize>,
    #[serde(default)]
    pub foreign_keys: bool,
    #[serde(default)]
    pub read_only: bool,
    pub created_at: i64,
}

/// Options for creating or updating a user database.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserDbOpts {
    pub group: Option<String>,
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub init_sql: Option<String>,
    pub max_rows: Option<usize>,
    pub max_size_mb: Option<usize>,
    pub foreign_keys: Option<bool>,
    pub read_only: Option<bool>,
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
    #[serde(default)]
    pub group: Option<String>,
    pub name: String,
    pub project_id: Option<String>,
    pub consumer_project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub max_concurrency: i64,
    pub rate_limit_per_sec: Option<i64>,
    pub batch_size: i64,
    pub lease_secs: i64,
    pub max_attempts: i64,
    #[serde(default = "default_retry_backoff_secs")]
    pub retry_backoff_secs: i64,
    #[serde(default)]
    pub dlq_retention_secs: Option<i64>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default)]
    pub max_pending_jobs: Option<i64>,
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
    #[serde(default)]
    pub group: Option<String>,
    pub name: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub public_read: bool,
    #[serde(default)]
    pub max_object_size_mb: Option<usize>,
    #[serde(default)]
    pub quota_mb: Option<usize>,
    #[serde(default)]
    pub allowed_content_types: Option<String>,
    #[serde(default)]
    pub cache_control: Option<String>,
    #[serde(default)]
    pub cors_origins: Option<String>,
    pub created_at: i64,
}

/// Options for creating or updating a blob bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BucketOpts {
    pub group: Option<String>,
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub public_read: Option<bool>,
    pub max_object_size_mb: Option<usize>,
    pub quota_mb: Option<usize>,
    pub allowed_content_types: Option<String>,
    pub cache_control: Option<String>,
    pub cors_origins: Option<String>,
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
    pub timeout_ms: Option<u64>,
    pub wasm_max_memory_mb: Option<usize>,
    pub allow_private_outbound: bool,
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
            timeout_ms: None,
            wasm_max_memory_mb: None,
            allow_private_outbound: false,
        }
    }

    pub fn with_cron_job(mut self, job: CronJob) -> Self {
        if job.timeout_ms.is_some() {
            self.timeout_ms = job.timeout_ms;
        }
        self.cron_job = Some(job);
        self
    }

    pub fn with_project_limits(mut self, project: &Project) -> Self {
        if self.timeout_ms.is_none() {
            self.timeout_ms = project.timeout_ms;
        }
        self.wasm_max_memory_mb = project.wasm_max_memory_mb;
        self.allow_private_outbound = project.allow_private_outbound;
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
    #[serde(default)]
    pub description: Option<String>,
    pub schedule: String,
    pub target: String,
    pub enabled: bool,
    #[serde(default = "default_utc_timezone")]
    pub timezone: String,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_retries: u32,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Options for creating or updating a cron job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CronJobOpts {
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub schedule: Option<String>,
    pub target: Option<String>,
    pub enabled: Option<bool>,
    pub timezone: Option<String>,
    pub timeout_ms: Option<u64>,
    pub max_retries: Option<u32>,
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
    #[serde(default)]
    pub group: Option<String>,
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub tokenizer: String,
    pub doc_count: usize,
    pub segments: usize,
    pub disk_bytes: u64,
    pub writer_budget_mb: usize,
    pub created_at: i64,
    pub updated_at: i64,
    pub schema: Vec<SearchFieldSchema>,
}

/// Options for creating or updating a search index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchIndexOpts {
    pub group: Option<String>,
    pub project_id: Option<String>,
    pub description: Option<String>,
    pub tokenizer: Option<String>,
    pub writer_budget_mb: Option<usize>,
    pub schema: Option<Vec<SearchFieldSchema>>,
    pub facet_fields: Option<Vec<String>>,
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
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default)]
    pub alpha: Option<f64>,
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

pub fn default_ai_timeout_secs() -> u64 {
    120
}

/// Public metadata for a configured AI Gateway upstream provider (never exposes API keys).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub has_api_key: bool,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub fallback_provider: Option<String>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
    #[serde(default)]
    pub cache_ttl_secs: u64,
    #[serde(default)]
    pub monthly_token_budget: Option<u64>,
    #[serde(default = "default_ai_timeout_secs")]
    pub timeout_secs: u64,
    pub enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Internal persisted AI provider record containing optional AES-GCM encrypted API key.
#[derive(Debug, Clone)]
pub struct AiProviderRecord {
    pub provider: AiProvider,
    pub api_key_nonce: Option<Vec<u8>>,
    pub api_key_ciphertext: Option<Vec<u8>>,
}

/// Options for creating or updating an AI Gateway provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiProviderOpts {
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub default_model: Option<String>,
    pub fallback_provider: Option<String>,
    pub rate_limit_rpm: Option<u32>,
    pub cache_ttl_secs: Option<u64>,
    pub monthly_token_budget: Option<u64>,
    pub timeout_secs: Option<u64>,
    pub enabled: Option<bool>,
}

/// Usage audit record emitted by the AI Gateway proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiUsageRecord {
    pub id: i64,
    pub provider_name: String,
    pub project_id: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub cached: bool,
    pub status: u16,
    pub latency_ms: u64,
    pub created_at: i64,
}

/// Aggregated AI Gateway token usage summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiUsageSummary {
    pub total_requests: usize,
    pub cached_requests: usize,
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    pub total_tokens: u64,
    #[serde(default)]
    pub month_tokens: u64,
    pub recent: Vec<AiUsageRecord>,
}

/// Configured outgoing alert webhook destination (never exposes HMAC secret).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertWebhook {
    pub id: String,
    pub name: String,
    pub url: String,
    pub has_secret: bool,
    #[serde(default)]
    pub events: Vec<String>,
    pub enabled: bool,
    #[serde(default)]
    pub last_status: Option<u16>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_triggered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Internal persisted alert webhook record containing optional AES-GCM encrypted signing secret.
#[derive(Debug, Clone)]
pub struct AlertWebhookRecord {
    pub webhook: AlertWebhook,
    pub secret_nonce: Option<Vec<u8>>,
    pub secret_ciphertext: Option<Vec<u8>>,
}

/// Options for creating or updating an alert webhook.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AlertWebhookOpts {
    pub name: Option<String>,
    pub url: Option<String>,
    pub secret: Option<String>,
    pub clear_secret: Option<bool>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
}

fn default_backup_interval_hours() -> u32 {
    24
}

fn default_backup_max_retain() -> usize {
    7
}

/// Persisted scheduled automatic backup configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AutoBackupConfig {
    pub enabled: bool,
    #[serde(default = "default_backup_interval_hours")]
    pub interval_hours: u32,
    #[serde(default = "default_backup_max_retain")]
    pub max_backups: usize,
    pub last_backup_at_ms: Option<i64>,
}

impl Default for AutoBackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_hours: default_backup_interval_hours(),
            max_backups: default_backup_max_retain(),
            last_backup_at_ms: None,
        }
    }
}

/// Real-time Pub-Sub event delivered over `/_realtime/{channel}/sse`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeMessage {
    pub id: String,
    pub channel: String,
    pub event: String,
    pub data: serde_json::Value,
    pub ts: i64,
}

/// Summary metadata for an active Realtime Pub-Sub channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeChannelInfo {
    pub channel: String,
    pub subscribers: usize,
    pub published_total: u64,
    pub last_published_at_ms: Option<i64>,
}

fn deserialize_some_option<'de, T, D>(
    deserializer: D,
) -> std::result::Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Persisted platform-wide runtime setting overrides stored in `core.db`.
/// `None` means inherit the boot/Profile default from [`crate::config::Config`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlatformSettingsOverrides {
    pub instance_name: Option<String>,
    pub panel_domain: Option<String>,
    pub restrict_panel_to_domain: Option<bool>,
    pub base_domain: Option<String>,
    pub public_scheme: Option<String>,
    pub trust_proxy_headers: Option<bool>,
    pub acme_enabled: Option<bool>,
    pub acme_email: Option<String>,
    pub acme_staging: Option<bool>,
    pub http_redirect_https: Option<bool>,
    pub extra_tls_domains: Option<Vec<String>>,
    pub log_retention_days: Option<u32>,
    pub queue_pending_retention_secs: Option<i64>,
    pub queue_dlq_retention_secs: Option<i64>,
    pub blob_gc_grace_secs: Option<i64>,
    pub wasm_max_memory_mb: Option<usize>,
    pub wasm_epoch_timeout_ms: Option<u64>,
    pub wasm_allow_private_outbound: Option<bool>,
    pub auto_rollback_enabled: Option<bool>,
    pub auto_rollback_window_secs: Option<i64>,
    pub auto_rollback_min_requests: Option<u64>,
    pub auto_rollback_error_rate_pct: Option<u8>,
    pub alert_webhook_url: Option<String>,
    pub alert_on_auto_rollback: Option<bool>,
    pub alert_on_cron_failure: Option<bool>,
    pub alert_on_worker_oom: Option<bool>,
}

/// Resolved effective platform settings (boot/Profile defaults merged with persisted overrides).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EffectivePlatformSettings {
    pub instance_name: Option<String>,
    pub panel_domain: Option<String>,
    pub restrict_panel_to_domain: bool,
    pub base_domain: Option<String>,
    pub public_scheme: Option<String>,
    pub trust_proxy_headers: bool,
    pub acme_enabled: bool,
    pub acme_email: Option<String>,
    pub acme_staging: bool,
    pub http_redirect_https: bool,
    pub extra_tls_domains: Vec<String>,
    pub log_retention_days: u32,
    pub queue_pending_retention_secs: i64,
    pub queue_dlq_retention_secs: i64,
    pub blob_gc_grace_secs: i64,
    pub wasm_max_memory_mb: usize,
    pub wasm_epoch_timeout_ms: u64,
    pub wasm_allow_private_outbound: bool,
    pub auto_rollback_enabled: bool,
    pub auto_rollback_window_secs: i64,
    pub auto_rollback_min_requests: u64,
    pub auto_rollback_error_rate_pct: u8,
    pub alert_webhook_url: Option<String>,
    pub has_alert_webhook_secret: bool,
    pub alert_on_auto_rollback: bool,
    pub alert_on_cron_failure: bool,
    pub alert_on_worker_oom: bool,
}

/// Live TLS / ACME listener telemetry maintained by [`crate::gateway::tls`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsRuntimeState {
    pub tls_mode: String,
    pub tls_active: bool,
    pub https_bind: Option<String>,
    pub active_domains: Vec<String>,
    pub reload_generation: u64,
    pub last_acme_event: Option<String>,
    pub last_acme_error: Option<String>,
}

impl Default for TlsRuntimeState {
    fn default() -> Self {
        Self {
            tls_mode: "disabled".to_string(),
            tls_active: false,
            https_bind: None,
            active_domains: Vec::new(),
            reload_generation: 0,
            last_acme_event: None,
            last_acme_error: None,
        }
    }
}

/// Read-only bootstrap environment & hardware budget telemetry for the Settings UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformRuntimeInfo {
    pub version: String,
    pub profile: String,
    pub bind_addr: String,
    pub https_bind: Option<String>,
    pub data_dir: String,
    pub tls_mode: String,
    pub tls_active: bool,
    pub tls_domains: Vec<String>,
    pub tls_cert_cache_dir: String,
    pub tls_reload_generation: u64,
    pub last_acme_event: Option<String>,
    pub last_acme_error: Option<String>,
    pub restart_required_for_tls: bool,
    pub local_api_host: Option<String>,
    pub local_api_prefix: Option<String>,
    pub local_api_upstream: Option<String>,
    pub tokio_worker_threads: usize,
    pub max_concurrency: usize,
    pub queue_concurrency: usize,
    pub max_open_user_dbs: usize,
    pub max_blob_upload_bytes: usize,
    pub wasm_cache_max_compiled: usize,
    pub insecure_default_master_key: bool,
    pub memory_budget_bytes: u64,
    pub cpu_cores: usize,
}

/// Full platform settings bundle returned by `GET/PATCH /_api/v1/settings`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformSettingsBundle {
    pub effective: EffectivePlatformSettings,
    pub overrides: PlatformSettingsOverrides,
    pub defaults: EffectivePlatformSettings,
    pub runtime: PlatformRuntimeInfo,
}

/// Partial update payload for `PATCH /_api/v1/settings`.
/// Distinguishes omitted fields (`None`), explicit JSON `null` (`Some(None)` = reset to default),
/// and concrete values (`Some(Some(val))`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PlatformSettingsPatch {
    #[serde(deserialize_with = "deserialize_some_option")]
    pub instance_name: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub panel_domain: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub restrict_panel_to_domain: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub base_domain: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub public_scheme: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub trust_proxy_headers: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub acme_enabled: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub acme_email: Option<Option<String>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub acme_staging: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub http_redirect_https: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub extra_tls_domains: Option<Option<Vec<String>>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub log_retention_days: Option<Option<u32>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub queue_pending_retention_secs: Option<Option<i64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub queue_dlq_retention_secs: Option<Option<i64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub blob_gc_grace_secs: Option<Option<i64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub wasm_max_memory_mb: Option<Option<usize>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub wasm_epoch_timeout_ms: Option<Option<u64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub wasm_allow_private_outbound: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub auto_rollback_enabled: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub auto_rollback_window_secs: Option<Option<i64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub auto_rollback_min_requests: Option<Option<u64>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub auto_rollback_error_rate_pct: Option<Option<u8>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub alert_webhook_url: Option<Option<String>>,
    pub alert_webhook_secret: Option<String>,
    pub clear_alert_webhook_secret: Option<bool>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub alert_on_auto_rollback: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub alert_on_cron_failure: Option<Option<bool>>,
    #[serde(deserialize_with = "deserialize_some_option")]
    pub alert_on_worker_oom: Option<Option<bool>>,
    pub confirm_lockout_risk: Option<bool>,
}

/// Result of verifying DNS resolution and FerroPod route/TLS membership for a domain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DnsCheckResult {
    pub domain: String,
    pub resolved_ips: Vec<String>,
    pub reachable_dns: bool,
    pub in_tls_domains: bool,
    pub matches_route: bool,
    pub wildcard_probe_host: Option<String>,
    pub wildcard_probe_ips: Vec<String>,
    pub error: Option<String>,
}

/// Result of dispatching a test alert webhook delivery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertTestResult {
    pub ok: bool,
    pub status_code: Option<u16>,
    pub latency_ms: u64,
    pub error: Option<String>,
}
