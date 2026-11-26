use serde::{Deserialize, Serialize};

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
