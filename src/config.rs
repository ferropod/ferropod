use std::net::SocketAddr;
use std::path::PathBuf;

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

/// Resource sizing preset for `ferropod serve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// Tuned for 512 MiB RAM VPS deployments.
    LowMem,
    /// Standard preset for 2 GiB+ hosts.
    #[default]
    Standard,
}

/// Native TLS / HTTPS configuration for `ferropod serve`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Optional HTTPS bind address (e.g. `0.0.0.0:443`). Defaults to `0.0.0.0:443`
    /// automatically when `--tls-cert` or `--acme-domain` is configured.
    pub https_bind: Option<SocketAddr>,
    /// PEM certificate chain path for manual TLS mode.
    pub cert_path: Option<PathBuf>,
    /// PEM private key path for manual TLS mode.
    pub key_path: Option<PathBuf>,
    /// Domains for automatic Let's Encrypt ACME (TLS-ALPN-01) certificates.
    pub acme_domains: Vec<String>,
    /// Optional contact email for Let's Encrypt ACME registration.
    pub acme_email: Option<String>,
    /// Use Let's Encrypt staging directory instead of production.
    pub acme_staging: bool,
    /// Redirect plain HTTP requests on `bind_addr` to HTTPS (`308 Permanent Redirect`),
    /// keeping `/_healthz` accessible over HTTP.
    pub http_redirect_https: bool,
}

/// A path on a routed site that is served by a configured HTTP backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalApiConfig {
    pub host: String,
    pub prefix: String,
    pub upstream: SocketAddr,
}

/// Runtime configuration for the single-binary `ferropod` server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub profile: Profile,
    pub bind_addr: SocketAddr,
    pub data_dir: PathBuf,
    /// Master key material used to derive AES-256-GCM and HMAC-SHA256 keys.
    pub master_key: String,
    pub tls: TlsConfig,
    #[serde(default)]
    pub local_api: Option<LocalApiConfig>,
    pub tokio_worker_threads: usize,
    pub max_concurrency: usize,
    pub queue_concurrency: usize,
    pub max_open_user_dbs: usize,
    pub max_ephemeral_kv_entries: usize,
    pub log_channel_capacity: usize,
    pub log_retention_days: u32,
    pub queue_pending_retention_secs: i64,
    pub queue_dlq_retention_secs: i64,
    pub queue_lease_secs: i64,
    pub blob_gc_grace_secs: i64,
    pub maintenance_interval_secs: u64,
    pub max_blob_upload_bytes: usize,
    pub wasm_max_memory_bytes: usize,
    pub wasm_max_table_elements: usize,
    pub wasm_compile_concurrency: usize,
    pub wasm_epoch_timeout_ms: u64,
    pub wasm_cache_max_compiled: usize,
}

impl Config {
    /// Create configuration defaults for the selected [`Profile`].
    pub fn for_profile(
        profile: Profile,
        bind_addr: SocketAddr,
        data_dir: PathBuf,
        master_key: String,
    ) -> Self {
        match profile {
            Profile::LowMem => Self {
                profile,
                bind_addr,
                data_dir,
                master_key,
                tls: TlsConfig::default(),
                local_api: None,
                tokio_worker_threads: 2,
                max_concurrency: 32,
                queue_concurrency: 4,
                max_open_user_dbs: 10,
                max_ephemeral_kv_entries: 4_096,
                log_channel_capacity: 1_024,
                log_retention_days: 7,
                queue_pending_retention_secs: 7 * 24 * 3600,
                queue_dlq_retention_secs: 7 * 24 * 3600,
                queue_lease_secs: 60,
                blob_gc_grace_secs: 30,
                maintenance_interval_secs: 30,
                max_blob_upload_bytes: 64 * 1024 * 1024,
                wasm_max_memory_bytes: 32 * 1024 * 1024,
                wasm_max_table_elements: 4_096,
                wasm_compile_concurrency: 1,
                wasm_epoch_timeout_ms: 10_000,
                wasm_cache_max_compiled: 8,
            },
            Profile::Standard => Self {
                profile,
                bind_addr,
                data_dir,
                master_key,
                tls: TlsConfig::default(),
                local_api: None,
                tokio_worker_threads: std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4),
                max_concurrency: 256,
                queue_concurrency: 16,
                max_open_user_dbs: 64,
                max_ephemeral_kv_entries: 32_768,
                log_channel_capacity: 4_096,
                log_retention_days: 7,
                queue_pending_retention_secs: 7 * 24 * 3600,
                queue_dlq_retention_secs: 14 * 24 * 3600,
                queue_lease_secs: 60,
                blob_gc_grace_secs: 30,
                maintenance_interval_secs: 30,
                max_blob_upload_bytes: 256 * 1024 * 1024,
                wasm_max_memory_bytes: 128 * 1024 * 1024,
                wasm_max_table_elements: 16_384,
                wasm_compile_concurrency: 2,
                wasm_epoch_timeout_ms: 10_000,
                wasm_cache_max_compiled: 32,
            },
        }
    }

    pub fn core_db_path(&self) -> PathBuf {
        self.data_dir.join("core.db")
    }

    pub fn kv_db_path(&self) -> PathBuf {
        self.data_dir.join("kv.db")
    }

    pub fn queue_db_path(&self) -> PathBuf {
        self.data_dir.join("queue.db")
    }

    pub fn search_db_path(&self) -> PathBuf {
        self.data_dir.join("search.db")
    }

    pub fn user_dbs_dir(&self) -> PathBuf {
        self.data_dir.join("dbs")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    pub fn blobs_dir(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }

    pub fn artifacts_dir(&self) -> PathBuf {
        self.data_dir.join("artifacts")
    }

    pub fn certs_dir(&self) -> PathBuf {
        self.data_dir.join("certs")
    }
}
