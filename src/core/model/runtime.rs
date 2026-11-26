use serde::{Deserialize, Serialize};

use super::{default_utc_timezone, ByteStream, Deployment, Project};

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
