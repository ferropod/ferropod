use thiserror::Error;

mod features;
mod project;
mod runtime;
mod settings;
mod storage;

pub use features::*;
pub use project::*;
pub use runtime::*;
pub use settings::*;
pub use storage::*;

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
