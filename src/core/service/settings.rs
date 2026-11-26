use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tracing::warn;

use crate::config::{detect_cpu_cores, detect_system_memory_bytes};
use crate::core::crypto::Crypto;
use crate::core::model::{
    AlertTestResult, Caller, DnsCheckResult, EffectivePlatformSettings, FerroError,
    PlatformRuntimeInfo, PlatformSettingsBundle, PlatformSettingsOverrides, PlatformSettingsPatch,
    Result, TlsRuntimeState, TokenScope,
};

use super::{normalize_host, PlatformService};

const SETTING_KEY_RUNTIME: &str = "runtime";
const SETTING_KEY_WEBHOOK_SECRET: &str = "alert_webhook_secret";

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedWebhookSecret {
    nonce: String,
    ciphertext: String,
}

/// Normalize and validate a DNS hostname or IPv4 literal.
/// When `allow_leading_wildcard` is true, a single leading `*.` prefix is stripped.
pub(crate) fn normalize_hostname(raw: &str, allow_leading_wildcard: bool) -> Result<String> {
    let mut trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FerroError::InvalidArgument("domain cannot be empty".into()));
    }
    if allow_leading_wildcard {
        if let Some(rest) = trimmed.strip_prefix("*.") {
            trimmed = rest.trim();
        }
    }
    let clean = trimmed.trim_end_matches('.').to_ascii_lowercase();
    if clean.is_empty()
        || clean.len() > 253
        || clean.contains("://")
        || clean.contains(':')
        || clean.contains('/')
        || clean.contains('\\')
        || clean.contains('*')
        || clean.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(FerroError::InvalidArgument(format!(
            "invalid domain name '{raw}'"
        )));
    }
    for label in clean.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(FerroError::InvalidArgument(format!(
                "invalid domain label in '{raw}'"
            )));
        }
    }
    Ok(clean)
}

/// Returns `true` if `host` (with optional `:port`, supporting `[::1]:port`) refers to loopback / localhost.
pub fn is_loopback_host(host: &str) -> bool {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return false;
    }
    let bare = if let Some(rest) = trimmed.strip_prefix('[') {
        match rest.find(']') {
            Some(idx) => &rest[..idx],
            None => trimmed,
        }
    } else if trimmed.parse::<IpAddr>().is_ok() {
        trimmed
    } else {
        match trimmed.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => h,
            _ => trimmed,
        }
    };
    let lower = bare.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    lower.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

fn is_ip_or_localhost(domain: &str) -> bool {
    let clean = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    clean.is_empty()
        || clean == "localhost"
        || clean.ends_with(".localhost")
        || clean.parse::<IpAddr>().is_ok()
}

fn is_insecure_master_key(master_key: &str) -> bool {
    master_key.len() < 16
        || matches!(
            master_key,
            "dev-insecure-master-key-change-me"
                | "change-this-strong-master-key"
                | "CHANGE_ME_TO_A_STRONG_RANDOM_SECRET"
        )
}

async fn lookup_host_ips(host: &str) -> std::result::Result<Vec<String>, String> {
    let target = (host, 80u16);
    match tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host(target)).await {
        Ok(Ok(addrs)) => {
            let mut set = BTreeSet::new();
            for addr in addrs {
                set.insert(addr.ip().to_string());
            }
            Ok(set.into_iter().collect())
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("DNS lookup timed out after 3s".to_string()),
    }
}

impl PlatformService {
    /// Compute Profile / boot-flag defaults for all platform runtime settings.
    pub fn default_platform_settings(&self) -> EffectivePlatformSettings {
        let tls = &self.config.tls;
        let acme_default = !tls.acme_domains.is_empty()
            || tls.acme_email.is_some()
            || tls.acme_staging
            || (tls.https_bind.is_some() && tls.cert_path.is_none() && tls.key_path.is_none());

        EffectivePlatformSettings {
            instance_name: None,
            panel_domain: None,
            restrict_panel_to_domain: false,
            base_domain: None,
            public_scheme: None,
            trust_proxy_headers: true,
            acme_enabled: acme_default,
            acme_email: tls.acme_email.clone(),
            acme_staging: tls.acme_staging,
            http_redirect_https: tls.http_redirect_https,
            extra_tls_domains: Vec::new(),
            log_retention_days: self.config.log_retention_days,
            queue_pending_retention_secs: self.config.queue_pending_retention_secs,
            queue_dlq_retention_secs: self.config.queue_dlq_retention_secs,
            blob_gc_grace_secs: self.config.blob_gc_grace_secs,
            wasm_max_memory_mb: (self.config.wasm_max_memory_bytes / (1024 * 1024)).max(1),
            wasm_epoch_timeout_ms: self.config.wasm_epoch_timeout_ms,
            wasm_allow_private_outbound: self.config.wasm_allow_private_outbound,
            auto_rollback_enabled: true,
            auto_rollback_window_secs: 300,
            auto_rollback_min_requests: 10,
            auto_rollback_error_rate_pct: 50,
            alert_webhook_url: None,
            has_alert_webhook_secret: false,
            alert_on_auto_rollback: true,
            alert_on_cron_failure: true,
            alert_on_worker_oom: true,
        }
    }

    fn merge_settings(
        &self,
        ov: &PlatformSettingsOverrides,
        has_webhook_secret: bool,
    ) -> EffectivePlatformSettings {
        let defaults = self.default_platform_settings();
        EffectivePlatformSettings {
            instance_name: ov.instance_name.clone().or(defaults.instance_name),
            panel_domain: ov.panel_domain.clone().or(defaults.panel_domain),
            restrict_panel_to_domain: ov
                .restrict_panel_to_domain
                .unwrap_or(defaults.restrict_panel_to_domain),
            base_domain: ov.base_domain.clone().or(defaults.base_domain),
            public_scheme: ov.public_scheme.clone().or(defaults.public_scheme),
            trust_proxy_headers: ov
                .trust_proxy_headers
                .unwrap_or(defaults.trust_proxy_headers),
            acme_enabled: ov.acme_enabled.unwrap_or(defaults.acme_enabled),
            acme_email: ov.acme_email.clone().or(defaults.acme_email),
            acme_staging: ov.acme_staging.unwrap_or(defaults.acme_staging),
            http_redirect_https: ov
                .http_redirect_https
                .unwrap_or(defaults.http_redirect_https),
            extra_tls_domains: ov
                .extra_tls_domains
                .clone()
                .unwrap_or(defaults.extra_tls_domains),
            log_retention_days: ov.log_retention_days.unwrap_or(defaults.log_retention_days),
            queue_pending_retention_secs: ov
                .queue_pending_retention_secs
                .unwrap_or(defaults.queue_pending_retention_secs),
            queue_dlq_retention_secs: ov
                .queue_dlq_retention_secs
                .unwrap_or(defaults.queue_dlq_retention_secs),
            blob_gc_grace_secs: ov.blob_gc_grace_secs.unwrap_or(defaults.blob_gc_grace_secs),
            wasm_max_memory_mb: ov.wasm_max_memory_mb.unwrap_or(defaults.wasm_max_memory_mb),
            wasm_epoch_timeout_ms: ov
                .wasm_epoch_timeout_ms
                .unwrap_or(defaults.wasm_epoch_timeout_ms),
            wasm_allow_private_outbound: ov
                .wasm_allow_private_outbound
                .unwrap_or(defaults.wasm_allow_private_outbound),
            auto_rollback_enabled: ov
                .auto_rollback_enabled
                .unwrap_or(defaults.auto_rollback_enabled),
            auto_rollback_window_secs: ov
                .auto_rollback_window_secs
                .unwrap_or(defaults.auto_rollback_window_secs),
            auto_rollback_min_requests: ov
                .auto_rollback_min_requests
                .unwrap_or(defaults.auto_rollback_min_requests),
            auto_rollback_error_rate_pct: ov
                .auto_rollback_error_rate_pct
                .unwrap_or(defaults.auto_rollback_error_rate_pct),
            alert_webhook_url: ov.alert_webhook_url.clone().or(defaults.alert_webhook_url),
            has_alert_webhook_secret: has_webhook_secret,
            alert_on_auto_rollback: ov
                .alert_on_auto_rollback
                .unwrap_or(defaults.alert_on_auto_rollback),
            alert_on_cron_failure: ov
                .alert_on_cron_failure
                .unwrap_or(defaults.alert_on_cron_failure),
            alert_on_worker_oom: ov
                .alert_on_worker_oom
                .unwrap_or(defaults.alert_on_worker_oom),
        }
    }

    pub fn settings_overrides(&self) -> PlatformSettingsOverrides {
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn effective_settings(&self) -> EffectivePlatformSettings {
        let ov = self.settings_overrides();
        let has_secret = self.has_alert_webhook_secret.load(Ordering::Relaxed);
        self.merge_settings(&ov, has_secret)
    }

    pub fn effective_wasm_max_memory_bytes(&self) -> usize {
        self.effective_settings()
            .wasm_max_memory_mb
            .saturating_mul(1024 * 1024)
    }

    pub fn effective_wasm_epoch_timeout_ms(&self) -> u64 {
        self.effective_settings().wasm_epoch_timeout_ms
    }

    pub fn effective_wasm_allow_private_outbound(&self) -> bool {
        self.effective_settings().wasm_allow_private_outbound
    }

    pub fn effective_trust_proxy_headers(&self) -> bool {
        self.effective_settings().trust_proxy_headers
    }

    pub fn effective_http_redirect_https(&self) -> bool {
        self.effective_settings().http_redirect_https
    }

    pub fn effective_panel_restriction(&self) -> Option<String> {
        let eff = self.effective_settings();
        if eff.restrict_panel_to_domain {
            eff.panel_domain.filter(|d| !d.is_empty())
        } else {
            None
        }
    }

    pub fn notify_tls_reload(&self) {
        let next = self.tls_reload_tx.borrow().wrapping_add(1);
        let _ = self.tls_reload_tx.send(next);
    }

    pub fn subscribe_tls_reload(&self) -> watch::Receiver<u64> {
        self.tls_reload_tx.subscribe()
    }

    pub fn update_tls_runtime(&self, f: impl FnOnce(&mut TlsRuntimeState)) {
        let mut guard = self.tls_runtime.write().unwrap_or_else(|e| e.into_inner());
        f(&mut guard);
    }

    pub fn tls_runtime_snapshot(&self) -> TlsRuntimeState {
        self.tls_runtime
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Merge CLI `--acme-domain`, `panel_domain`, `extra_tls_domains`, and active route domains,
    /// filtering out loopback/localhost/IP addresses unsuitable for ACME TLS-ALPN-01.
    pub async fn effective_tls_domains(&self) -> Vec<String> {
        let eff = self.effective_settings();
        let mut set = BTreeSet::new();
        for d in &self.config.tls.acme_domains {
            let norm = normalize_host(d);
            if !is_ip_or_localhost(&norm) {
                set.insert(norm);
            }
        }
        if let Some(panel) = eff.panel_domain.as_deref() {
            let norm = normalize_host(panel);
            if !is_ip_or_localhost(&norm) {
                set.insert(norm);
            }
        }
        for d in &eff.extra_tls_domains {
            let norm = normalize_host(d);
            if !is_ip_or_localhost(&norm) {
                set.insert(norm);
            }
        }
        for d in self.allowed_domains().await {
            let norm = normalize_host(&d);
            if !is_ip_or_localhost(&norm) {
                set.insert(norm);
            }
        }
        set.into_iter().collect()
    }

    pub async fn get_platform_settings(&self, caller: &Caller) -> Result<PlatformSettingsBundle> {
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => {}
            TokenScope::Project(_) => {
                return Err(FerroError::Forbidden(
                    "admin or readonly scope required to view platform settings".into(),
                ));
            }
        }

        let overrides = self.settings_overrides();
        let defaults = self.default_platform_settings();
        let effective = self.merge_settings(
            &overrides,
            self.has_alert_webhook_secret.load(Ordering::Relaxed),
        );
        let rt = self.tls_runtime_snapshot();
        let computed_domains = self.effective_tls_domains().await;
        let tls_domains = if !rt.active_domains.is_empty()
            && (rt.tls_mode == "manual" || (rt.tls_active && computed_domains.is_empty()))
        {
            rt.active_domains.clone()
        } else {
            computed_domains
        };
        let restart_required_for_tls =
            !rt.tls_active && effective.acme_enabled && self.config.tls.cert_path.is_none();

        let runtime = PlatformRuntimeInfo {
            version: crate::update::CURRENT_VERSION.to_string(),
            profile: format!("{:?}", self.config.profile),
            bind_addr: self.config.bind_addr.to_string(),
            https_bind: rt
                .https_bind
                .or_else(|| self.config.tls.https_bind.map(|a| a.to_string())),
            data_dir: self.config.data_dir.to_string_lossy().to_string(),
            tls_mode: rt.tls_mode,
            tls_active: rt.tls_active,
            tls_domains,
            tls_cert_cache_dir: self.config.certs_dir().to_string_lossy().to_string(),
            tls_reload_generation: rt.reload_generation,
            last_acme_event: rt.last_acme_event,
            last_acme_error: rt.last_acme_error,
            restart_required_for_tls,
            local_api_host: self.config.local_api.as_ref().map(|c| c.host.clone()),
            local_api_prefix: self.config.local_api.as_ref().map(|c| c.prefix.clone()),
            local_api_upstream: self
                .config
                .local_api
                .as_ref()
                .map(|c| c.upstream.to_string()),
            tokio_worker_threads: self.config.tokio_worker_threads,
            max_concurrency: self.config.max_concurrency,
            queue_concurrency: self.config.queue_concurrency,
            max_open_user_dbs: self.config.max_open_user_dbs,
            max_blob_upload_bytes: self.config.max_blob_upload_bytes,
            wasm_cache_max_compiled: self.config.wasm_cache_max_compiled,
            insecure_default_master_key: is_insecure_master_key(&self.config.master_key),
            memory_budget_bytes: detect_system_memory_bytes(),
            cpu_cores: detect_cpu_cores(),
        };

        Ok(PlatformSettingsBundle {
            effective,
            overrides,
            defaults,
            runtime,
        })
    }

    pub async fn update_platform_settings(
        &self,
        caller: &Caller,
        patch: PlatformSettingsPatch,
        request_host: Option<&str>,
    ) -> Result<PlatformSettingsBundle> {
        Self::require_admin(caller)?;
        let mut next = self.settings_overrides();
        let mut changed_fields: Vec<&'static str> = Vec::new();

        if let Some(val) = patch.instance_name {
            next.instance_name = match val {
                Some(raw) => {
                    let t = raw.trim();
                    if t.is_empty() {
                        None
                    } else {
                        if t.chars().count() > 80 || t.chars().any(char::is_control) {
                            return Err(FerroError::InvalidArgument(
                                "instance_name must be at most 80 characters without control characters"
                                    .into(),
                            ));
                        }
                        Some(t.to_string())
                    }
                }
                None => None,
            };
            changed_fields.push("instance_name");
        }

        if let Some(val) = patch.panel_domain {
            next.panel_domain = match val {
                Some(raw) => {
                    let t = raw.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(normalize_hostname(t, false)?)
                    }
                }
                None => None,
            };
            changed_fields.push("panel_domain");
        }

        if let Some(val) = patch.restrict_panel_to_domain {
            next.restrict_panel_to_domain = val;
            changed_fields.push("restrict_panel_to_domain");
        }

        if let Some(val) = patch.base_domain {
            next.base_domain = match val {
                Some(raw) => {
                    let t = raw.trim();
                    if t.is_empty() {
                        None
                    } else {
                        Some(normalize_hostname(t, true)?)
                    }
                }
                None => None,
            };
            changed_fields.push("base_domain");
        }

        if let Some(val) = patch.public_scheme {
            next.public_scheme = match val {
                Some(raw) => {
                    let lower = raw.trim().to_ascii_lowercase();
                    if lower.is_empty() || lower == "auto" {
                        None
                    } else if lower == "http" || lower == "https" {
                        Some(lower)
                    } else {
                        return Err(FerroError::InvalidArgument(
                            "public_scheme must be 'http', 'https', or null".into(),
                        ));
                    }
                }
                None => None,
            };
            changed_fields.push("public_scheme");
        }

        if let Some(val) = patch.trust_proxy_headers {
            next.trust_proxy_headers = val;
            changed_fields.push("trust_proxy_headers");
        }

        if let Some(val) = patch.acme_enabled {
            next.acme_enabled = val;
            changed_fields.push("acme_enabled");
        }

        if let Some(val) = patch.acme_email {
            next.acme_email = match val {
                Some(raw) => {
                    let t = raw.trim();
                    if t.is_empty() {
                        None
                    } else {
                        if t.len() > 254
                            || !t.contains('@')
                            || t.chars().any(|c| c.is_whitespace() || c.is_control())
                        {
                            return Err(FerroError::InvalidArgument(
                                "acme_email must be a valid email address".into(),
                            ));
                        }
                        Some(t.to_string())
                    }
                }
                None => None,
            };
            changed_fields.push("acme_email");
        }

        if let Some(val) = patch.acme_staging {
            next.acme_staging = val;
            changed_fields.push("acme_staging");
        }

        if let Some(val) = patch.http_redirect_https {
            next.http_redirect_https = val;
            changed_fields.push("http_redirect_https");
        }

        if let Some(val) = patch.extra_tls_domains {
            next.extra_tls_domains = match val {
                Some(list) => {
                    let mut set = BTreeSet::new();
                    for raw in list {
                        let t = raw.trim();
                        if !t.is_empty() {
                            set.insert(normalize_hostname(t, false)?);
                        }
                    }
                    if set.len() > 64 {
                        return Err(FerroError::InvalidArgument(
                            "extra_tls_domains cannot exceed 64 domains".into(),
                        ));
                    }
                    let deduped: Vec<String> = set.into_iter().collect();
                    if deduped.is_empty() {
                        None
                    } else {
                        Some(deduped)
                    }
                }
                None => None,
            };
            changed_fields.push("extra_tls_domains");
        }

        if let Some(val) = patch.log_retention_days {
            if let Some(days) = val {
                if !(1..=3650).contains(&days) {
                    return Err(FerroError::InvalidArgument(
                        "log_retention_days must be between 1 and 3650".into(),
                    ));
                }
            }
            next.log_retention_days = val;
            changed_fields.push("log_retention_days");
        }

        if let Some(val) = patch.queue_pending_retention_secs {
            if let Some(secs) = val {
                if !(60..=31_536_000).contains(&secs) {
                    return Err(FerroError::InvalidArgument(
                        "queue_pending_retention_secs must be between 60 and 31536000".into(),
                    ));
                }
            }
            next.queue_pending_retention_secs = val;
            changed_fields.push("queue_pending_retention_secs");
        }

        if let Some(val) = patch.queue_dlq_retention_secs {
            if let Some(secs) = val {
                if !(60..=31_536_000).contains(&secs) {
                    return Err(FerroError::InvalidArgument(
                        "queue_dlq_retention_secs must be between 60 and 31536000".into(),
                    ));
                }
            }
            next.queue_dlq_retention_secs = val;
            changed_fields.push("queue_dlq_retention_secs");
        }

        if let Some(val) = patch.blob_gc_grace_secs {
            if let Some(secs) = val {
                if !(0..=2_592_000).contains(&secs) {
                    return Err(FerroError::InvalidArgument(
                        "blob_gc_grace_secs must be between 0 and 2592000".into(),
                    ));
                }
            }
            next.blob_gc_grace_secs = val;
            changed_fields.push("blob_gc_grace_secs");
        }

        if let Some(val) = patch.wasm_max_memory_mb {
            if let Some(mb) = val {
                if !(4..=2048).contains(&mb) {
                    return Err(FerroError::InvalidArgument(
                        "wasm_max_memory_mb must be between 4 and 2048".into(),
                    ));
                }
            }
            next.wasm_max_memory_mb = val;
            changed_fields.push("wasm_max_memory_mb");
        }

        if let Some(val) = patch.wasm_epoch_timeout_ms {
            if let Some(ms) = val {
                if !(100..=300_000).contains(&ms) {
                    return Err(FerroError::InvalidArgument(
                        "wasm_epoch_timeout_ms must be between 100 and 300000".into(),
                    ));
                }
            }
            next.wasm_epoch_timeout_ms = val;
            changed_fields.push("wasm_epoch_timeout_ms");
        }

        if let Some(val) = patch.wasm_allow_private_outbound {
            next.wasm_allow_private_outbound = val;
            changed_fields.push("wasm_allow_private_outbound");
        }

        if let Some(val) = patch.auto_rollback_enabled {
            next.auto_rollback_enabled = val;
            changed_fields.push("auto_rollback_enabled");
        }

        if let Some(val) = patch.auto_rollback_window_secs {
            if let Some(secs) = val {
                if !(30..=3600).contains(&secs) {
                    return Err(FerroError::InvalidArgument(
                        "auto_rollback_window_secs must be between 30 and 3600".into(),
                    ));
                }
            }
            next.auto_rollback_window_secs = val;
            changed_fields.push("auto_rollback_window_secs");
        }

        if let Some(val) = patch.auto_rollback_min_requests {
            if let Some(n) = val {
                if !(1..=10_000).contains(&n) {
                    return Err(FerroError::InvalidArgument(
                        "auto_rollback_min_requests must be between 1 and 10000".into(),
                    ));
                }
            }
            next.auto_rollback_min_requests = val;
            changed_fields.push("auto_rollback_min_requests");
        }

        if let Some(val) = patch.auto_rollback_error_rate_pct {
            if let Some(pct) = val {
                if !(1..=100).contains(&pct) {
                    return Err(FerroError::InvalidArgument(
                        "auto_rollback_error_rate_pct must be between 1 and 100".into(),
                    ));
                }
            }
            next.auto_rollback_error_rate_pct = val;
            changed_fields.push("auto_rollback_error_rate_pct");
        }

        if let Some(val) = patch.alert_webhook_url {
            next.alert_webhook_url = match val {
                Some(raw) => {
                    let t = raw.trim();
                    if t.is_empty() {
                        None
                    } else {
                        if t.len() > 1024
                            || (!t.starts_with("http://") && !t.starts_with("https://"))
                        {
                            return Err(FerroError::InvalidArgument(
                                "alert_webhook_url must start with http:// or https://".into(),
                            ));
                        }
                        let _: reqwest::Url = t.parse().map_err(|e| {
                            FerroError::InvalidArgument(format!("invalid alert_webhook_url: {e}"))
                        })?;
                        Some(t.to_string())
                    }
                }
                None => None,
            };
            changed_fields.push("alert_webhook_url");
        }

        if let Some(val) = patch.alert_on_auto_rollback {
            next.alert_on_auto_rollback = val;
            changed_fields.push("alert_on_auto_rollback");
        }

        if let Some(val) = patch.alert_on_cron_failure {
            next.alert_on_cron_failure = val;
            changed_fields.push("alert_on_cron_failure");
        }

        if let Some(val) = patch.alert_on_worker_oom {
            next.alert_on_worker_oom = val;
            changed_fields.push("alert_on_worker_oom");
        }

        // Anti-lockout guard when restricting panel access to panel_domain
        let prospective =
            self.merge_settings(&next, self.has_alert_webhook_secret.load(Ordering::Relaxed));
        if prospective.restrict_panel_to_domain {
            let panel_dom = prospective
                .panel_domain
                .as_deref()
                .filter(|d| !d.is_empty())
                .ok_or_else(|| {
                    FerroError::InvalidArgument(
                        "panel_domain must be set before enabling restrict_panel_to_domain".into(),
                    )
                })?;
            if let Some(req_host) = request_host.map(str::trim).filter(|h| !h.is_empty()) {
                if !is_loopback_host(req_host)
                    && normalize_host(req_host) != panel_dom
                    && patch.confirm_lockout_risk != Some(true)
                {
                    return Err(FerroError::InvalidArgument(format!(
                        "refusing to restrict panel access to '{panel_dom}' while connected via '{req_host}'; access via '{panel_dom}' first or pass confirm_lockout_risk=true"
                    )));
                }
            }
        }

        if patch.clear_alert_webhook_secret == Some(true) {
            self.registry
                .delete_platform_setting(SETTING_KEY_WEBHOOK_SECRET)
                .await?;
            self.has_alert_webhook_secret
                .store(false, Ordering::Relaxed);
            changed_fields.push("alert_webhook_secret_cleared");
        } else if let Some(raw_secret) = patch.alert_webhook_secret.as_deref() {
            let trimmed = raw_secret.trim();
            if !trimmed.is_empty() {
                let (nonce, ciphertext) = self.crypto.encrypt_secret(trimmed.as_bytes())?;
                let enc = EncryptedWebhookSecret {
                    nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
                    ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
                };
                let json = serde_json::to_string(&enc).map_err(|e| {
                    FerroError::Internal(anyhow::anyhow!("serialize webhook secret: {e}"))
                })?;
                self.registry
                    .set_platform_setting(SETTING_KEY_WEBHOOK_SECRET, &json)
                    .await?;
                self.has_alert_webhook_secret.store(true, Ordering::Relaxed);
                changed_fields.push("alert_webhook_secret");
            }
        }

        let serialized = serde_json::to_string(&next)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("serialize settings: {e}")))?;
        self.registry
            .set_platform_setting(SETTING_KEY_RUNTIME, &serialized)
            .await?;

        {
            let mut guard = self.settings.write().unwrap_or_else(|e| e.into_inner());
            *guard = next;
        }
        self.notify_tls_reload();
        self.audit(
            None,
            &caller.actor,
            "settings.update",
            serde_json::json!({ "fields": changed_fields }),
        );
        self.get_platform_settings(caller).await
    }

    pub async fn reset_platform_settings(
        &self,
        caller: &Caller,
        section: Option<&str>,
    ) -> Result<PlatformSettingsBundle> {
        Self::require_admin(caller)?;
        let sec = section.unwrap_or("all").trim().to_ascii_lowercase();
        let mut next = self.settings_overrides();
        let mut clear_secret = false;

        match sec.as_str() {
            "" | "all" => {
                next = PlatformSettingsOverrides::default();
                clear_secret = true;
            }
            "domains" => {
                next.instance_name = None;
                next.panel_domain = None;
                next.restrict_panel_to_domain = None;
                next.base_domain = None;
                next.public_scheme = None;
                next.trust_proxy_headers = None;
                next.acme_enabled = None;
                next.acme_email = None;
                next.acme_staging = None;
                next.http_redirect_https = None;
                next.extra_tls_domains = None;
            }
            "maintenance" => {
                next.log_retention_days = None;
                next.queue_pending_retention_secs = None;
                next.queue_dlq_retention_secs = None;
                next.blob_gc_grace_secs = None;
            }
            "runtime" => {
                next.wasm_max_memory_mb = None;
                next.wasm_epoch_timeout_ms = None;
                next.wasm_allow_private_outbound = None;
            }
            "rollback" => {
                next.auto_rollback_enabled = None;
                next.auto_rollback_window_secs = None;
                next.auto_rollback_min_requests = None;
                next.auto_rollback_error_rate_pct = None;
            }
            "alerts" => {
                next.alert_webhook_url = None;
                next.alert_on_auto_rollback = None;
                next.alert_on_cron_failure = None;
                next.alert_on_worker_oom = None;
                clear_secret = true;
            }
            other => {
                return Err(FerroError::InvalidArgument(format!(
                    "unknown settings section '{other}'"
                )));
            }
        }

        if clear_secret {
            self.registry
                .delete_platform_setting(SETTING_KEY_WEBHOOK_SECRET)
                .await?;
            self.has_alert_webhook_secret
                .store(false, Ordering::Relaxed);
        }

        let serialized = serde_json::to_string(&next)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("serialize settings: {e}")))?;
        self.registry
            .set_platform_setting(SETTING_KEY_RUNTIME, &serialized)
            .await?;

        {
            let mut guard = self.settings.write().unwrap_or_else(|e| e.into_inner());
            *guard = next;
        }
        self.notify_tls_reload();
        self.audit(
            None,
            &caller.actor,
            "settings.reset",
            serde_json::json!({ "section": if sec.is_empty() { "all" } else { sec.as_str() } }),
        );
        self.get_platform_settings(caller).await
    }

    pub async fn check_domain_dns(
        &self,
        caller: &Caller,
        domain: &str,
        check_wildcard: bool,
    ) -> Result<DnsCheckResult> {
        Self::require_admin(caller)?;
        let norm = normalize_hostname(domain, true)?;
        let (resolved_ips, primary_err) = match lookup_host_ips(&norm).await {
            Ok(ips) => (ips, None),
            Err(e) => (Vec::new(), Some(e)),
        };

        let (wildcard_probe_host, wildcard_probe_ips, wildcard_err) = if check_wildcard {
            let probe = format!("_ferropod-probe.{norm}");
            match lookup_host_ips(&probe).await {
                Ok(ips) => (Some(probe), ips, None),
                Err(e) => (Some(probe), Vec::new(), Some(e)),
            }
        } else {
            (None, Vec::new(), None)
        };

        let tls_domains = self.effective_tls_domains().await;
        let in_tls_domains = tls_domains.iter().any(|d| d == &norm);
        let matches_route = self.is_domain_allowed(&norm).await;
        let reachable_dns = !resolved_ips.is_empty() || !wildcard_probe_ips.is_empty();
        let error = if reachable_dns {
            None
        } else {
            primary_err.or(wildcard_err)
        };

        Ok(DnsCheckResult {
            domain: norm,
            resolved_ips,
            reachable_dns,
            in_tls_domains,
            matches_route,
            wildcard_probe_host,
            wildcard_probe_ips,
            error,
        })
    }

    async fn decrypt_alert_webhook_secret(&self) -> Option<String> {
        if !self.has_alert_webhook_secret.load(Ordering::Relaxed) {
            return None;
        }
        let raw = self
            .registry
            .get_platform_setting(SETTING_KEY_WEBHOOK_SECRET)
            .await
            .ok()
            .flatten()?;
        let enc: EncryptedWebhookSecret = serde_json::from_str(&raw).ok()?;
        let nonce = base64::engine::general_purpose::STANDARD
            .decode(&enc.nonce)
            .ok()?;
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(&enc.ciphertext)
            .ok()?;
        let plain = self.crypto.decrypt_secret(&nonce, &ciphertext).ok()?;
        String::from_utf8(plain).ok()
    }

    async fn send_webhook_payload(
        &self,
        url: &str,
        event: &str,
        title: &str,
        detail: &serde_json::Value,
    ) -> AlertTestResult {
        let eff = self.effective_settings();
        let payload = serde_json::json!({
            "event": event,
            "instance": eff.instance_name.as_deref().unwrap_or("ferropod"),
            "title": title,
            "detail": detail,
            "timestamp": Utc::now().to_rfc3339(),
        });
        let body_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let secret = self.decrypt_alert_webhook_secret().await;

        let started = Instant::now();
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return AlertTestResult {
                    ok: false,
                    status_code: None,
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: Some(format!("build HTTP client failed: {e}")),
                };
            }
        };

        let mut req = client
            .post(url)
            .header("content-type", "application/json")
            .header("x-ferropod-event", event);
        if let Some(sec) = secret {
            let sig = Crypto::hmac_sha256_hex(sec.as_bytes(), &body_bytes);
            req = req.header("x-ferropod-signature", format!("sha256={sig}"));
        }

        match req.body(body_bytes).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let ok = resp.status().is_success();
                AlertTestResult {
                    ok,
                    status_code: Some(status),
                    latency_ms: started.elapsed().as_millis() as u64,
                    error: if ok {
                        None
                    } else {
                        Some(format!("webhook endpoint returned HTTP {status}"))
                    },
                }
            }
            Err(e) => AlertTestResult {
                ok: false,
                status_code: None,
                latency_ms: started.elapsed().as_millis() as u64,
                error: Some(e.to_string()),
            },
        }
    }

    /// Fire an asynchronous platform alert webhook if configured and enabled for `event`.
    pub fn fire_alert(&self, event: &str, title: String, detail: serde_json::Value) {
        let eff = self.effective_settings();
        let enabled = match event {
            "auto_rollback" => eff.alert_on_auto_rollback,
            "cron_failure" => eff.alert_on_cron_failure,
            "worker_oom" => eff.alert_on_worker_oom,
            _ => true,
        };
        if !enabled {
            return;
        }
        let scope = detail
            .get("project_id")
            .or_else(|| detail.get("provider"))
            .or_else(|| detail.get("cron_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("global");
        let cd_key = format!("{event}:{scope}");
        let cooldown = std::time::Duration::from_secs(super::ALERT_COOLDOWN_SECS);
        {
            let mut cds = self
                .alert_cooldowns
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if cds.get(&cd_key).is_some_and(|t| t.elapsed() < cooldown) {
                return;
            }
            if cds.len() > 512 {
                cds.retain(|_, t| t.elapsed() < cooldown);
            }
            cds.insert(cd_key, std::time::Instant::now());
        }
        let Ok(permit) = self.alert_concurrency.clone().try_acquire_owned() else {
            warn!(
                event = %event,
                "alert webhook concurrency limit reached, dropping alert"
            );
            return;
        };
        let legacy_url = eff.alert_webhook_url.filter(|u| !u.is_empty());
        let svc = self.clone();
        let event_s = event.to_string();
        tokio::spawn(async move {
            let _permit = permit;
            if let Some(url) = legacy_url {
                let res = svc
                    .send_webhook_payload(&url, &event_s, &title, &detail)
                    .await;
                if !res.ok {
                    warn!(
                        event = %event_s,
                        error = ?res.error,
                        status = ?res.status_code,
                        "alert webhook delivery failed"
                    );
                }
            }
            svc.dispatch_alert_webhooks(&event_s, &title, &detail).await;
        });
    }

    pub async fn test_alert_webhook(&self, caller: &Caller) -> Result<AlertTestResult> {
        Self::require_admin(caller)?;
        let eff = self.effective_settings();
        let url = eff
            .alert_webhook_url
            .filter(|u| !u.is_empty())
            .ok_or_else(|| {
                FerroError::InvalidArgument("alert_webhook_url is not configured".into())
            })?;
        let res = self
            .send_webhook_payload(
                &url,
                "test",
                "FerroPod test alert",
                &serde_json::json!({ "actor": caller.actor }),
            )
            .await;
        self.audit(
            None,
            &caller.actor,
            "settings.alerts.test",
            serde_json::json!({ "ok": res.ok, "status_code": res.status_code }),
        );
        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_hostname_validation() {
        assert_eq!(
            normalize_hostname(" Panel.Example.COM. ", false).unwrap(),
            "panel.example.com"
        );
        assert_eq!(
            normalize_hostname("*.Apps.Example.com.", true).unwrap(),
            "apps.example.com"
        );
        assert!(normalize_hostname("*.apps.example.com", false).is_err());
        assert!(normalize_hostname("https://panel.example.com", false).is_err());
        assert!(normalize_hostname("panel.example.com:8080", false).is_err());
        assert!(normalize_hostname("panel.example.com/path", false).is_err());
        assert!(normalize_hostname("", false).is_err());
    }

    #[test]
    fn loopback_host_detection() {
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("localhost:8080"));
        assert!(is_loopback_host("app.localhost:3000"));
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.1:8080"));
        assert!(is_loopback_host("[::1]:8443"));
        assert!(is_loopback_host("::1"));
        assert!(!is_loopback_host("panel.example.com"));
        assert!(!is_loopback_host("192.168.1.10:8080"));
    }
}
