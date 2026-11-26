use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::{default_kv_ttl_secs, default_retry_backoff_secs, FerroError, Result};

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
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
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
    pub rate_limit_rpm: Option<u32>,
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
    #[serde(default)]
    pub query_timeout_ms: Option<u64>,
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
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
    pub query_timeout_ms: Option<u64>,
    pub rate_limit_rpm: Option<u32>,
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
    #[serde(default)]
    pub rate_limit_rpm: Option<u32>,
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
    pub rate_limit_rpm: Option<u32>,
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

/// Virtual folder summary returned by bucket directory browsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobFolderEntry {
    pub prefix: String,
    pub name: String,
    pub object_count: u64,
    pub total_size: u64,
    pub updated_at: i64,
}

/// Directory-style listing result for a bucket prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlobBrowseResult {
    pub prefix: String,
    pub folders: Vec<BlobFolderEntry>,
    pub objects: Vec<ObjectMeta>,
    pub truncated: bool,
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

/// Byte stream type used by [`crate::core::traits::ObjectStore`].
pub type ByteStream =
    std::pin::Pin<Box<dyn futures_util::Stream<Item = std::io::Result<Bytes>> + Send>>;

/// Inclusive byte range `(start, end)` for HTTP `Range` responses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: Option<u64>,
}
