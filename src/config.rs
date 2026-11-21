use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use clap::ValueEnum;
use http::uri::Authority;
use serde::{Deserialize, Serialize};

/// Resource sizing preset for `ferropod serve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Profile {
    /// Automatically detect host / container (cgroup) memory and CPU cores to allocate quotas.
    #[default]
    Auto,
    /// Tuned for 512 MiB - 1 GiB RAM VPS deployments.
    LowMem,
    /// Standard preset for 2 GiB+ hosts.
    Standard,
}

/// Detect the effective memory budget in bytes from `/proc/meminfo` and Linux cgroup v1/v2 limits.
pub fn detect_system_memory_bytes() -> u64 {
    let mut host_total = 0u64;
    if let Ok(content) = std::fs::read_to_string("/proc/meminfo") {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("MemTotal:") {
                if let Some(kb) = rest
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    host_total = kb.saturating_mul(1024);
                    break;
                }
            }
        }
    }
    if host_total == 0 {
        host_total = 2 * 1024 * 1024 * 1024;
    }

    let mut effective = host_total;
    for cg_path in [
        "/sys/fs/cgroup/memory.max",
        "/sys/fs/cgroup/memory/memory.limit_in_bytes",
    ] {
        if let Ok(raw) = std::fs::read_to_string(cg_path) {
            let trimmed = raw.trim();
            if trimmed != "max" {
                if let Ok(limit) = trimmed.parse::<u64>() {
                    // Ignore sentinel "unlimited" values near u64::MAX / page-aligned max
                    if limit > 0 && limit < (1u64 << 50) {
                        effective = effective.min(limit);
                    }
                }
            }
        }
    }
    effective
}

/// Detect available logical CPU cores.
pub fn detect_cpu_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .max(1)
}

impl Profile {
    /// Resolve [`Profile::Auto`] into [`Profile::LowMem`] (`<= 1 GiB`) or [`Profile::Standard`] (`> 1 GiB`).
    pub fn resolve_with_memory(self, mem_bytes: u64) -> Self {
        match self {
            Profile::Auto => {
                if mem_bytes <= 1024 * 1024 * 1024 {
                    Profile::LowMem
                } else {
                    Profile::Standard
                }
            }
            explicit => explicit,
        }
    }

    /// Resolve [`Profile::Auto`] using the current system/container memory budget.
    pub fn resolve(self) -> Self {
        self.resolve_with_memory(detect_system_memory_bytes())
    }
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

/// Reserved Ferropod gateway path prefixes that `--local-api-prefix` must not shadow.
pub const RESERVED_LOCAL_API_PREFIXES: &[&str] =
    &["/_api", "/_blob", "/_panel", "/_internal", "/_healthz"];

fn canonical_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(v6)),
        v4 @ IpAddr::V4(_) => v4,
    }
}

/// A path on a routed site that is served by a configured HTTP backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalApiConfig {
    pub host: String,
    pub prefix: String,
    pub upstream: SocketAddr,
}

impl LocalApiConfig {
    /// Validate and normalize `--local-api-*` configuration against active Ferropod listeners.
    pub fn validated(
        host: &str,
        prefix: Option<&str>,
        upstream: SocketAddr,
        listeners: &[SocketAddr],
    ) -> anyhow::Result<Self> {
        let trimmed_host = host.trim();
        let authority: Authority = trimmed_host
            .parse()
            .map_err(|_| anyhow::anyhow!("--local-api-host must be a valid hostname"))?;
        anyhow::ensure!(
            authority.as_str() == authority.host()
                && !authority.host().trim_end_matches('.').is_empty(),
            "--local-api-host must be a hostname without a port"
        );

        let upstream_ip = canonical_ip(upstream.ip());
        anyhow::ensure!(
            upstream.port() != 0 && !upstream_ip.is_unspecified(),
            "--local-api-upstream requires a backend IP address and nonzero port"
        );
        anyhow::ensure!(
            !listeners.iter().any(|listener| {
                listener.port() == upstream.port()
                    && (canonical_ip(listener.ip()) == upstream_ip
                        || listener.ip().is_unspecified())
            }),
            "--local-api-upstream must differ from the Ferropod listen addresses"
        );

        let raw_prefix = prefix.unwrap_or("/api").trim().trim_end_matches('/');
        anyhow::ensure!(
            raw_prefix.starts_with('/')
                && !raw_prefix[1..].is_empty()
                && raw_prefix[1..].split('/').all(|segment| {
                    !segment.is_empty()
                        && segment.bytes().all(|byte| {
                            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
                        })
                }),
            "--local-api-prefix must be a non-root path such as /api or /backend/v1"
        );
        let prefix_lower = raw_prefix.to_ascii_lowercase();
        anyhow::ensure!(
            !RESERVED_LOCAL_API_PREFIXES.iter().any(|reserved| {
                prefix_lower == *reserved
                    || prefix_lower
                        .strip_prefix(reserved)
                        .is_some_and(|rest| rest.starts_with('/'))
            }),
            "--local-api-prefix cannot shadow reserved Ferropod routes"
        );

        Ok(Self {
            host: authority.host().trim_end_matches('.').to_ascii_lowercase(),
            prefix: raw_prefix.to_owned(),
            upstream,
        })
    }
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
    #[serde(default)]
    pub wasm_allow_private_outbound: bool,
}

impl Config {
    /// Create configuration defaults for the selected [`Profile`], auto-detecting system resources when [`Profile::Auto`] is selected.
    pub fn for_profile(
        profile: Profile,
        bind_addr: SocketAddr,
        data_dir: PathBuf,
        master_key: String,
    ) -> Self {
        Self::for_profile_with_resources(
            profile,
            detect_system_memory_bytes(),
            detect_cpu_cores(),
            bind_addr,
            data_dir,
            master_key,
        )
    }

    /// Create configuration for a [`Profile`] given explicit detected memory (`mem_bytes`) and logical CPU cores (`cpu_cores`).
    pub fn for_profile_with_resources(
        profile: Profile,
        mem_bytes: u64,
        cpu_cores: usize,
        bind_addr: SocketAddr,
        data_dir: PathBuf,
        master_key: String,
    ) -> Self {
        let cores = cpu_cores.max(1);
        match profile {
            Profile::Auto => {
                let resolved = profile.resolve_with_memory(mem_bytes);
                let mem_mib = (mem_bytes / (1024 * 1024)).max(256) as usize;
                if resolved == Profile::LowMem {
                    let is_tiny = mem_mib <= 512;
                    Self {
                        profile: Profile::LowMem,
                        bind_addr,
                        data_dir,
                        master_key,
                        tls: TlsConfig::default(),
                        local_api: None,
                        tokio_worker_threads: cores.clamp(1, 2),
                        max_concurrency: if is_tiny { 32 } else { 64 },
                        queue_concurrency: if is_tiny { 4 } else { 8 },
                        max_open_user_dbs: if is_tiny { 10 } else { 16 },
                        max_ephemeral_kv_entries: if is_tiny { 4_096 } else { 8_192 },
                        log_channel_capacity: if is_tiny { 1_024 } else { 2_048 },
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
                        wasm_cache_max_compiled: if is_tiny { 8 } else { 12 },
                        wasm_allow_private_outbound: false,
                    }
                } else {
                    Self {
                        profile: Profile::Standard,
                        bind_addr,
                        data_dir,
                        master_key,
                        tls: TlsConfig::default(),
                        local_api: None,
                        tokio_worker_threads: cores.clamp(2, 16),
                        max_concurrency: (mem_mib / 16).clamp(128, 1024),
                        queue_concurrency: (cores * 4).clamp(8, 32),
                        max_open_user_dbs: (mem_mib / 64).clamp(24, 128),
                        max_ephemeral_kv_entries: (mem_mib * 8).clamp(16_384, 65_536),
                        log_channel_capacity: mem_mib.clamp(2_048, 8_192),
                        log_retention_days: 7,
                        queue_pending_retention_secs: 7 * 24 * 3600,
                        queue_dlq_retention_secs: 14 * 24 * 3600,
                        queue_lease_secs: 60,
                        blob_gc_grace_secs: 30,
                        maintenance_interval_secs: 30,
                        max_blob_upload_bytes: if mem_mib < 3072 {
                            128 * 1024 * 1024
                        } else {
                            256 * 1024 * 1024
                        },
                        wasm_max_memory_bytes: if mem_mib < 3072 {
                            64 * 1024 * 1024
                        } else {
                            128 * 1024 * 1024
                        },
                        wasm_max_table_elements: 16_384,
                        wasm_compile_concurrency: (cores / 2).clamp(1, 4),
                        wasm_epoch_timeout_ms: 10_000,
                        wasm_cache_max_compiled: (mem_mib / 128).clamp(16, 64),
                        wasm_allow_private_outbound: false,
                    }
                }
            }
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
                wasm_allow_private_outbound: false,
            },
            Profile::Standard => Self {
                profile,
                bind_addr,
                data_dir,
                master_key,
                tls: TlsConfig::default(),
                local_api: None,
                tokio_worker_threads: cores,
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
                wasm_allow_private_outbound: false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_profile_scales_with_detected_memory_and_cores() {
        let bind: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let data = PathBuf::from("./data");

        let cfg_512m = Config::for_profile_with_resources(
            Profile::Auto,
            512 * 1024 * 1024,
            1,
            bind,
            data.clone(),
            "k".into(),
        );
        assert_eq!(cfg_512m.profile, Profile::LowMem);
        assert_eq!(cfg_512m.tokio_worker_threads, 1);
        assert_eq!(cfg_512m.max_concurrency, 32);
        assert_eq!(cfg_512m.wasm_cache_max_compiled, 8);
        assert_eq!(cfg_512m.max_open_user_dbs, 10);

        let cfg_1g = Config::for_profile_with_resources(
            Profile::Auto,
            1024 * 1024 * 1024,
            2,
            bind,
            data.clone(),
            "k".into(),
        );
        assert_eq!(cfg_1g.profile, Profile::LowMem);
        assert_eq!(cfg_1g.tokio_worker_threads, 2);
        assert_eq!(cfg_1g.max_concurrency, 64);
        assert_eq!(cfg_1g.wasm_cache_max_compiled, 12);
        assert_eq!(cfg_1g.max_open_user_dbs, 16);

        let cfg_2g = Config::for_profile_with_resources(
            Profile::Auto,
            2 * 1024 * 1024 * 1024,
            2,
            bind,
            data.clone(),
            "k".into(),
        );
        assert_eq!(cfg_2g.profile, Profile::Standard);
        assert_eq!(cfg_2g.max_concurrency, 128);
        assert_eq!(cfg_2g.wasm_max_memory_bytes, 64 * 1024 * 1024);
        assert_eq!(cfg_2g.wasm_cache_max_compiled, 16);

        let cfg_8g = Config::for_profile_with_resources(
            Profile::Auto,
            8 * 1024 * 1024 * 1024,
            8,
            bind,
            data,
            "k".into(),
        );
        assert_eq!(cfg_8g.profile, Profile::Standard);
        assert_eq!(cfg_8g.tokio_worker_threads, 8);
        assert_eq!(cfg_8g.max_concurrency, 512);
        assert_eq!(cfg_8g.wasm_max_memory_bytes, 128 * 1024 * 1024);
        assert_eq!(cfg_8g.wasm_cache_max_compiled, 64);
        assert_eq!(cfg_8g.max_open_user_dbs, 128);
    }
}
