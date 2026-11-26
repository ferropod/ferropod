mod ai;
mod alerts;
mod blob;
mod cron;
mod db;
mod kv;
mod logs;
mod projects;
mod queue;
mod realtime;
mod search;
mod secrets;
mod settings;
mod tokens;

pub use ai::{AiProxyResponse, AiProxyResult, AiProxyStreamResponse};
pub use cron::cron_matches_timestamp;
pub use logs::LogSubscription;
pub(crate) use projects::normalize_host;
pub use projects::CreateDeploymentInput;
pub use realtime::RealtimeHub;
pub use settings::is_loopback_host;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tracing::warn;

use crate::config::{Config, Profile};
use crate::core::crypto::Crypto;
use crate::core::model::{
    AutoBackupConfig, Caller, FerroError, LogEntry, PlatformSettingsOverrides, Project, Result,
    RouteEntry, TlsRuntimeState, UserDbMeta,
};

pub const DEFAULT_SERVICE_RATE_LIMIT_RPM: u32 = 1_200;
pub const DEFAULT_KV_RATE_LIMIT_RPM: u32 = 2_400;
pub const DEFAULT_BLOB_RATE_LIMIT_RPM: u32 = 1_200;
pub const DEFAULT_QUEUE_ENQUEUE_RPM: u32 = 1_200;
pub const DEFAULT_DB_MAX_CONCURRENCY: usize = 16;
pub const MAX_DB_BATCH_STATEMENTS: usize = 200;
pub const MAX_QUEUE_PAYLOAD_BYTES: usize = 512 * 1024;
pub const DEFAULT_INGRESS_IP_RPM: u32 = 2_400;
pub const DEFAULT_API_IP_RPM: u32 = 1_200;
pub const DEFAULT_ROUTE_IP_RPM: u32 = 1_200;
pub const DEFAULT_DB_MAX_SIZE_MB: usize = 512;
pub const DEFAULT_SEARCH_MAX_CONCURRENCY: usize = 8;
pub const DEFAULT_AI_RATE_LIMIT_RPM: u32 = 120;
pub const DEFAULT_AI_MAX_CONCURRENCY: usize = 16;
pub const ALERT_COOLDOWN_SECS: u64 = 60;
use crate::core::traits::{
    DbManager, KvStore, LogStore, ObjectStore, ProjectStore, QueueStore, RegistryStore,
    SearchStore, SecretStore, TokenStore, WorkerRuntime,
};
use crate::runtime::WasmtimeRuntime;
use crate::storage::search_store::TursoSearchStore;

/// Domain store dependencies required to build a [`PlatformService`].
pub struct PlatformDeps {
    pub projects: Arc<dyn ProjectStore>,
    pub secrets: Arc<dyn SecretStore>,
    pub tokens: Arc<dyn TokenStore>,
    pub registry: Arc<dyn RegistryStore>,
    pub kv: Arc<dyn KvStore>,
    pub queue: Arc<dyn QueueStore>,
    pub objects: Arc<dyn ObjectStore>,
    pub db: Arc<dyn DbManager>,
    pub logs: Arc<dyn LogStore>,
}

/// Lightweight atomic runtime metrics exported via `/_api/v1/metrics/prometheus`.
#[derive(Debug, Default)]
pub struct GatewayMetrics {
    pub http_requests_2xx: AtomicU64,
    pub http_requests_3xx: AtomicU64,
    pub http_requests_4xx: AtomicU64,
    pub http_requests_5xx: AtomicU64,
    pub http_duration_ms_sum: AtomicU64,
    pub http_duration_ms_count: AtomicU64,
    pub rate_limited_total: AtomicU64,
    pub worker_errors_total: AtomicU64,
    pub worker_oom_total: AtomicU64,
    pub active_http_requests: AtomicU64,
}

impl GatewayMetrics {
    pub fn record_http(&self, status: u16, elapsed_ms: u64) {
        match status {
            200..=299 => self.http_requests_2xx.fetch_add(1, Ordering::Relaxed),
            300..=399 => self.http_requests_3xx.fetch_add(1, Ordering::Relaxed),
            400..=499 => self.http_requests_4xx.fetch_add(1, Ordering::Relaxed),
            _ if status >= 500 => self.http_requests_5xx.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
        self.http_duration_ms_sum
            .fetch_add(elapsed_ms, Ordering::Relaxed);
        self.http_duration_ms_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// Rolling 5-minute error-rate window used to automatically roll back unhealthy production deployments.
#[derive(Debug, Default)]
struct DepHealthWindow {
    promoted_at_secs: i64,
    total: u64,
    errors_5xx: u64,
    rolled_back: bool,
}

#[derive(Debug, Default)]
pub struct DeploymentHealthTracker {
    windows: std::sync::Mutex<HashMap<String, DepHealthWindow>>,
}

impl DeploymentHealthTracker {
    pub fn mark_promoted(&self, dep_id: &str) {
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 256 {
            let cutoff = Utc::now().timestamp() - 600;
            map.retain(|_, w| w.promoted_at_secs >= cutoff);
        }
        map.insert(
            dep_id.to_string(),
            DepHealthWindow {
                promoted_at_secs: Utc::now().timestamp(),
                total: 0,
                errors_5xx: 0,
                rolled_back: false,
            },
        );
    }

    pub fn record_outcome(&self, dep_id: &str, is_5xx: bool) -> bool {
        self.record_outcome_with_policy(dep_id, is_5xx, true, 300, 10, 50)
    }

    pub fn record_outcome_with_policy(
        &self,
        dep_id: &str,
        is_5xx: bool,
        enabled: bool,
        window_secs: i64,
        min_requests: u64,
        error_rate_pct: u8,
    ) -> bool {
        if !enabled {
            return false;
        }
        let now = Utc::now().timestamp();
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        let Some(win) = map.get_mut(dep_id) else {
            return false;
        };
        if win.rolled_back || (now - win.promoted_at_secs) > window_secs.max(1) {
            return false;
        }
        win.total += 1;
        if is_5xx {
            win.errors_5xx += 1;
        }
        let threshold_pct = u64::from(error_rate_pct.clamp(1, 100));
        if win.total >= min_requests.max(1)
            && win.errors_5xx.saturating_mul(100) >= win.total.saturating_mul(threshold_pct)
        {
            win.rolled_back = true;
            true
        } else {
            false
        }
    }
}

/// Unified business orchestration layer shared by REST/Admin, MCP (M3), and Runtime (M2).
#[derive(Clone)]
pub struct PlatformService {
    config: Config,
    crypto: Crypto,
    projects: Arc<dyn ProjectStore>,
    secrets: Arc<dyn SecretStore>,
    tokens: Arc<dyn TokenStore>,
    registry: Arc<dyn RegistryStore>,
    kv: Arc<dyn KvStore>,
    queue: Arc<dyn QueueStore>,
    objects: Arc<dyn ObjectStore>,
    db: Arc<dyn DbManager>,
    logs: Arc<dyn LogStore>,
    search: Arc<dyn SearchStore>,
    runtime: Arc<dyn WorkerRuntime>,
    metrics: Arc<GatewayMetrics>,
    pub(crate) rate_limiter: Arc<crate::gateway::guard::SlidingWindowRateLimiter>,
    health_tracker: Arc<DeploymentHealthTracker>,
    canary_counter: Arc<AtomicU64>,
    ai_cache: Arc<RwLock<HashMap<String, (i64, serde_json::Value)>>>,
    routes: Arc<RwLock<HashMap<String, RouteEntry>>>,
    http_concurrency: Arc<Semaphore>,
    route_refresh_lock: Arc<Mutex<()>>,
    settings: Arc<std::sync::RwLock<PlatformSettingsOverrides>>,
    has_alert_webhook_secret: Arc<AtomicBool>,
    tls_runtime: Arc<std::sync::RwLock<TlsRuntimeState>>,
    tls_reload_tx: watch::Sender<u64>,
    realtime: Arc<RealtimeHub>,
    bucket_cache_control: Arc<std::sync::RwLock<HashMap<String, Option<String>>>>,
    db_concurrency: Arc<std::sync::Mutex<HashMap<String, Arc<Semaphore>>>>,
    db_row_count_cache: Arc<std::sync::Mutex<HashMap<String, (Instant, usize)>>>,
    admin_concurrency: Arc<Semaphore>,
    app_concurrency: Arc<std::sync::Mutex<HashMap<String, Arc<Semaphore>>>>,
    db_meta_cache: Arc<std::sync::Mutex<HashMap<String, (Instant, UserDbMeta)>>>,
    search_concurrency: Arc<std::sync::Mutex<HashMap<String, Arc<Semaphore>>>>,
    ai_http_client: reqwest::Client,
    ai_concurrency: Arc<Semaphore>,
    alert_cooldowns: Arc<std::sync::Mutex<HashMap<String, Instant>>>,
    alert_concurrency: Arc<Semaphore>,
    in_flight_crons: Arc<std::sync::Mutex<HashSet<String>>>,
    queue_concurrency: Arc<std::sync::Mutex<HashMap<String, Arc<Semaphore>>>>,
}

impl PlatformService {
    pub async fn new(config: Config, deps: PlatformDeps) -> Result<Self> {
        let crypto = Crypto::from_master_key(&config.master_key);
        let http_concurrency = Arc::new(Semaphore::new(config.max_concurrency));
        let admin_concurrency = Arc::new(Semaphore::new((config.max_concurrency / 2).clamp(16, 64)));
        let ai_http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(16)
            .build()
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("failed to build AI client: {e}")))?;
        let search: Arc<dyn SearchStore> =
            Arc::new(TursoSearchStore::open(&config.search_db_path()).await?);
        let wasmtime_rt = Arc::new(WasmtimeRuntime::new(config.clone())?);
        let runtime: Arc<dyn WorkerRuntime> = wasmtime_rt.clone();

        let initial_overrides = match deps.registry.get_platform_setting("runtime").await? {
            Some(raw) => match serde_json::from_str::<PlatformSettingsOverrides>(&raw) {
                Ok(parsed) => parsed,
                Err(e) => {
                    warn!(error = %e, "ignoring malformed persisted platform settings in core.db");
                    PlatformSettingsOverrides::default()
                }
            },
            None => PlatformSettingsOverrides::default(),
        };
        let has_webhook_secret = deps
            .registry
            .get_platform_setting("alert_webhook_secret")
            .await?
            .is_some();
        let initial_tls_mode = if config.tls.cert_path.is_some() && config.tls.key_path.is_some() {
            "manual"
        } else if !config.tls.acme_domains.is_empty()
            || config.tls.acme_email.is_some()
            || config.tls.acme_staging
            || config.tls.https_bind.is_some()
        {
            "acme"
        } else {
            "disabled"
        };
        let initial_tls_state = TlsRuntimeState {
            tls_mode: initial_tls_mode.to_string(),
            tls_active: false,
            https_bind: config.tls.https_bind.map(|a| a.to_string()),
            active_domains: Vec::new(),
            reload_generation: 0,
            last_acme_event: None,
            last_acme_error: None,
        };
        let (tls_reload_tx, _) = watch::channel(0u64);
        let max_rt_channels = match config.profile.resolve() {
            Profile::LowMem => 256,
            Profile::Standard | Profile::Auto => 1024,
        };
        let realtime = Arc::new(RealtimeHub::new(max_rt_channels));

        let svc = Self {
            config,
            crypto,
            projects: deps.projects,
            secrets: deps.secrets,
            tokens: deps.tokens,
            registry: deps.registry,
            kv: deps.kv,
            queue: deps.queue,
            objects: deps.objects,
            db: deps.db,
            logs: deps.logs,
            search,
            runtime,
            metrics: Arc::new(GatewayMetrics::default()),
            rate_limiter: Arc::new(crate::gateway::guard::SlidingWindowRateLimiter::default()),
            health_tracker: Arc::new(DeploymentHealthTracker::default()),
            canary_counter: Arc::new(AtomicU64::new(0)),
            ai_cache: Arc::new(RwLock::new(HashMap::new())),
            routes: Arc::new(RwLock::new(HashMap::new())),
            http_concurrency,
            route_refresh_lock: Arc::new(Mutex::new(())),
            settings: Arc::new(std::sync::RwLock::new(initial_overrides)),
            has_alert_webhook_secret: Arc::new(AtomicBool::new(has_webhook_secret)),
            tls_runtime: Arc::new(std::sync::RwLock::new(initial_tls_state)),
            tls_reload_tx,
            realtime,
            bucket_cache_control: Arc::new(std::sync::RwLock::new(HashMap::new())),
            db_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
            db_row_count_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            admin_concurrency,
            app_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
            db_meta_cache: Arc::new(std::sync::Mutex::new(HashMap::new())),
            search_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
            ai_http_client,
            ai_concurrency: Arc::new(Semaphore::new(DEFAULT_AI_MAX_CONCURRENCY)),
            alert_cooldowns: Arc::new(std::sync::Mutex::new(HashMap::new())),
            alert_concurrency: Arc::new(Semaphore::new(8)),
            in_flight_crons: Arc::new(std::sync::Mutex::new(HashSet::new())),
            queue_concurrency: Arc::new(std::sync::Mutex::new(HashMap::new())),
        };
        wasmtime_rt.bind_service(svc.clone());
        svc.refresh_routes().await?;
        Ok(svc)
    }

    pub fn metrics(&self) -> &Arc<GatewayMetrics> {
        &self.metrics
    }

    pub fn rate_limiter(&self) -> &Arc<crate::gateway::guard::SlidingWindowRateLimiter> {
        &self.rate_limiter
    }

    pub(crate) fn enforce_service_rate_limit(
        &self,
        rl_key: &str,
        rpm: Option<u32>,
        default_rpm: u32,
        resource_label: &str,
    ) -> Result<()> {
        let effective_rpm = rpm.filter(|r| *r > 0).unwrap_or(default_rpm);
        if effective_rpm == 0 {
            return Ok(());
        }
        let (allowed, _, reset_secs) = self.rate_limiter.check_and_record(rl_key, effective_rpm);
        if !allowed {
            self.metrics
                .rate_limited_total
                .fetch_add(1, Ordering::Relaxed);
            return Err(FerroError::RateLimited(format!(
                "{resource_label} rate limit exceeded ({effective_rpm}/min, retry after {reset_secs}s)"
            )));
        }
        Ok(())
    }

    fn get_or_init_semaphore(
        map: &std::sync::Mutex<HashMap<String, Arc<Semaphore>>>,
        key: &str,
        cap: usize,
    ) -> Arc<Semaphore> {
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(cap.max(1))))
            .clone()
    }

    pub(crate) async fn acquire_timed_permit(
        &self,
        sem: Arc<Semaphore>,
        busy_msg: String,
    ) -> Result<OwnedSemaphorePermit> {
        match tokio::time::timeout(Duration::from_millis(250), sem.acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) | Err(_) => {
                self.metrics
                    .rate_limited_total
                    .fetch_add(1, Ordering::Relaxed);
                Err(FerroError::RateLimited(busy_msg))
            }
        }
    }

    pub(crate) async fn acquire_db_permit(
        &self,
        db_id: &str,
        db_name: &str,
    ) -> Result<OwnedSemaphorePermit> {
        let sem =
            Self::get_or_init_semaphore(&self.db_concurrency, db_id, DEFAULT_DB_MAX_CONCURRENCY);
        self.acquire_timed_permit(
            sem,
            format!("database '{db_name}' is busy (max concurrency {DEFAULT_DB_MAX_CONCURRENCY} reached)"),
        )
        .await
    }

    pub(crate) fn invalidate_db_caches(&self, db_id: &str) {
        let mut cache = self
            .db_row_count_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        cache.remove(db_id);
    }

    pub(crate) fn remove_db_concurrency(&self, db_id: &str) {
        let mut guard = self.db_concurrency.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(db_id);
    }

    pub(crate) fn invalidate_db_meta_cache(&self) {
        let mut guard = self.db_meta_cache.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }

    pub fn try_acquire_admin_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.admin_concurrency.clone().try_acquire_owned().ok()
    }

    pub(crate) fn try_acquire_app_permit(&self, project_id: &str) -> Option<OwnedSemaphorePermit> {
        let cap = (self.config.max_concurrency / 2).max(4);
        let sem = Self::get_or_init_semaphore(&self.app_concurrency, project_id, cap);
        sem.try_acquire_owned().ok()
    }

    pub(crate) async fn acquire_search_permit(
        &self,
        index_id: &str,
        index_name: &str,
    ) -> Result<OwnedSemaphorePermit> {
        let sem = Self::get_or_init_semaphore(
            &self.search_concurrency,
            index_id,
            DEFAULT_SEARCH_MAX_CONCURRENCY,
        );
        self.acquire_timed_permit(
            sem,
            format!(
                "search index '{index_name}' is busy (max concurrency {DEFAULT_SEARCH_MAX_CONCURRENCY} reached)"
            ),
        )
        .await
    }

    pub(crate) fn remove_search_concurrency(&self, index_id: &str) {
        let mut guard = self
            .search_concurrency
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        guard.remove(index_id);
    }

    pub fn health_tracker(&self) -> &Arc<DeploymentHealthTracker> {
        &self.health_tracker
    }

    pub fn next_canary_ticket(&self) -> u64 {
        self.canary_counter.fetch_add(1, Ordering::Relaxed)
    }

    pub fn runtime(&self) -> &Arc<dyn WorkerRuntime> {
        &self.runtime
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn try_acquire_http_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.http_concurrency.clone().try_acquire_owned().ok()
    }

    pub fn emit_log(&self, entry: LogEntry) {
        self.logs.emit(entry);
    }

    pub fn audit(
        &self,
        project_id: Option<&str>,
        actor: &str,
        action: &str,
        detail: serde_json::Value,
    ) {
        self.logs.emit(LogEntry {
            id: None,
            ts: Utc::now().timestamp_millis(),
            project_id: project_id.map(str::to_string),
            request_id: None,
            source: "audit".to_string(),
            level: "info".to_string(),
            message: format!("{actor}: {action}"),
            meta_json: Some(detail.to_string()),
        });
    }

    /// Flush buffered logs and checkpoint all backing stores cleanly.
    pub async fn flush_and_checkpoint(&self) -> Result<()> {
        self.logs.flush().await?;
        self.db.checkpoint_all().await?;
        self.projects.checkpoint().await?;
        self.kv.checkpoint().await?;
        self.queue.checkpoint().await?;
        self.search.checkpoint().await?;
        Ok(())
    }

    /// Execute one background maintenance cycle (KV TTL, queue lease reaper/retention, blob GC, log rotation).
    pub async fn run_maintenance_once(&self) {
        let eff = self.effective_settings();

        loop {
            match self.kv.cleanup_expired_batch(500).await {
                Ok(500) => tokio::time::sleep(Duration::from_millis(50)).await,
                Ok(_) => break,
                Err(e) => {
                    warn!(error = %e, "kv ttl cleanup error");
                    break;
                }
            }
        }

        if let Err(e) = self.queue.reap_expired().await {
            warn!(error = %e, "queue lease reaper error");
        }

        loop {
            match self
                .queue
                .cleanup_expired_batch(
                    eff.queue_pending_retention_secs,
                    eff.queue_dlq_retention_secs,
                    500,
                )
                .await
            {
                Ok(500) => tokio::time::sleep(Duration::from_millis(50)).await,
                Ok(_) => break,
                Err(e) => {
                    warn!(error = %e, "queue retention cleanup error");
                    break;
                }
            }
        }

        if let Err(e) = self.objects.run_gc_batch(200).await {
            warn!(error = %e, "blob gc error");
        }

        if let Err(e) = self
            .run_artifact_gc_with_grace(u64::try_from(eff.blob_gc_grace_secs).unwrap_or(0).max(300))
            .await
        {
            warn!(error = %e, "artifact gc error");
        }

        if let Err(e) = self.logs.cleanup_old_files(eff.log_retention_days).await {
            warn!(error = %e, "daily log cleanup error");
        }

        let ai_cutoff_ms = Utc::now().timestamp_millis() - (30i64 * 86_400 * 1000);
        if let Err(e) = self.registry.cleanup_ai_usage_before(ai_cutoff_ms).await {
            warn!(error = %e, "ai usage cleanup error");
        }

        self.maybe_run_auto_backup().await;
    }

    /// Remove compiled/uploaded artifact files on disk no longer referenced by any deployment.
    pub async fn run_artifact_gc(&self) -> Result<usize> {
        self.run_artifact_gc_with_grace(0).await
    }

    /// Remove unreferenced artifact files older than `grace_secs` seconds.
    pub async fn run_artifact_gc_with_grace(&self, grace_secs: u64) -> Result<usize> {
        let entries = self.projects.list_referenced_artifact_paths().await?;
        let mut referenced = std::collections::HashSet::with_capacity(entries.len() * 2);
        for (project_id, raw) in entries {
            let trimmed = raw.trim().trim_start_matches('/');
            if !trimmed.is_empty() && !trimmed.contains('/') {
                referenced.insert(format!("{project_id}/{trimmed}"));
            }
            referenced.insert(raw);
        }
        self.runtime
            .gc_unreferenced_artifacts(&referenced, grace_secs)
            .await
    }

    /// Export consistent database snapshots (`core.db`, `kv.db`, `queue.db`, and user DBs) for off-site backup.
    pub async fn create_snapshot(&self, caller: &Caller, out_dir: &Path) -> Result<usize> {
        Self::require_admin(caller)?;
        tokio::fs::create_dir_all(out_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("create snapshot dir: {e}")))?;
        self.flush_and_checkpoint().await?;
        self.projects.snapshot_to(&out_dir.join("core.db")).await?;
        self.kv.snapshot_to(&out_dir.join("kv.db")).await?;
        self.queue.snapshot_to(&out_dir.join("queue.db")).await?;
        self.search.snapshot_to(&out_dir.join("search.db")).await?;
        self.snapshot_user_dbs(caller, &out_dir.join("dbs")).await
    }

    pub async fn get_auto_backup_config(&self, caller: &Caller) -> Result<AutoBackupConfig> {
        Self::require_admin(caller)?;
        let raw = self.registry.get_platform_setting("auto_backup").await?;
        match raw {
            Some(s) => Ok(serde_json::from_str(&s).unwrap_or_default()),
            None => Ok(AutoBackupConfig::default()),
        }
    }

    pub async fn update_auto_backup_config(
        &self,
        caller: &Caller,
        mut cfg: AutoBackupConfig,
    ) -> Result<AutoBackupConfig> {
        Self::require_admin(caller)?;
        cfg.interval_hours = cfg.interval_hours.clamp(1, 720);
        cfg.max_backups = cfg.max_backups.clamp(1, 100);
        let prev = self.get_auto_backup_config(caller).await?;
        if cfg.last_backup_at_ms.is_none() {
            cfg.last_backup_at_ms = prev.last_backup_at_ms;
        }
        let json = serde_json::to_string(&cfg)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("serialize auto_backup: {e}")))?;
        self.registry
            .set_platform_setting("auto_backup", &json)
            .await?;
        self.audit(
            None,
            &caller.actor,
            "backup.schedule.update",
            serde_json::json!({
                "enabled": cfg.enabled,
                "interval_hours": cfg.interval_hours,
                "max_backups": cfg.max_backups,
            }),
        );
        Ok(cfg)
    }

    async fn maybe_run_auto_backup(&self) {
        let mut cfg = match self.registry.get_platform_setting("auto_backup").await {
            Ok(Some(s)) => serde_json::from_str::<AutoBackupConfig>(&s).unwrap_or_default(),
            _ => return,
        };
        if !cfg.enabled {
            return;
        }
        let now_ms = Utc::now().timestamp_millis();
        let interval_ms = i64::from(cfg.interval_hours.max(1)) * 3_600_000;
        if let Some(last) = cfg.last_backup_at_ms {
            if now_ms.saturating_sub(last) < interval_ms {
                return;
            }
        }
        let backups_root = self.config.data_dir.join("backups");
        let name = format!("auto_{}", Utc::now().format("%Y%m%d_%H%M%S"));
        let out_dir = backups_root.join(&name);
        if let Err(e) = self
            .create_snapshot(&Caller::system("auto-backup"), &out_dir)
            .await
        {
            warn!(error = %e, "scheduled auto-backup snapshot failed");
            return;
        }
        cfg.last_backup_at_ms = Some(now_ms);
        if let Ok(json) = serde_json::to_string(&cfg) {
            let _ = self
                .registry
                .set_platform_setting("auto_backup", &json)
                .await;
        }

        // Prune oldest `auto_*` directories exceeding `max_backups`
        let keep = cfg.max_backups.max(1);
        if let Ok(mut rd) = tokio::fs::read_dir(&backups_root).await {
            let mut auto_dirs = Vec::new();
            while let Ok(Some(entry)) = rd.next_entry().await {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with("auto_") {
                    if let Ok(meta) = entry.metadata().await {
                        if meta.is_dir() {
                            auto_dirs.push(fname);
                        }
                    }
                }
            }
            auto_dirs.sort();
            if auto_dirs.len() > keep {
                let to_remove = auto_dirs.len() - keep;
                for old in auto_dirs.into_iter().take(to_remove) {
                    let _ = tokio::fs::remove_dir_all(backups_root.join(old)).await;
                }
            }
        }
    }

    pub async fn restore_snapshot(
        &self,
        caller: &Caller,
        backup_dir: &Path,
    ) -> Result<Vec<String>> {
        Self::require_admin(caller)?;
        let src_core = backup_dir.join("core.db");
        if !src_core.exists() {
            return Err(FerroError::InvalidArgument(
                "backup directory does not contain core.db".into(),
            ));
        }

        let _ = self.flush_and_checkpoint().await;

        let safety_dir = self.config.data_dir.join("backups").join(format!(
            "pre_restore_{}",
            Utc::now().format("%Y%m%d_%H%M%S")
        ));
        if let Err(e) = self.create_snapshot(caller, &safety_dir).await {
            warn!(error = %e, "pre-restore safety snapshot failed");
        }

        let mut restored = Vec::new();
        for (name, dest) in [
            ("core.db", self.config.core_db_path()),
            ("kv.db", self.config.kv_db_path()),
            ("queue.db", self.config.queue_db_path()),
            ("search.db", self.config.search_db_path()),
        ] {
            let src = backup_dir.join(name);
            if src.exists() {
                copy_restore_db_file(&src, &dest).await?;
                restored.push(name.to_string());
            }
        }

        let src_dbs = backup_dir.join("dbs");
        if src_dbs.is_dir() {
            let dest_dbs = self.config.user_dbs_dir();
            tokio::fs::create_dir_all(&dest_dbs)
                .await
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("create dbs dir: {e}")))?;
            let mut rd = tokio::fs::read_dir(&src_dbs)
                .await
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("read backup dbs: {e}")))?;
            while let Some(entry) = rd
                .next_entry()
                .await
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("iter backup dbs: {e}")))?
            {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.ends_with(".db") {
                    let src = entry.path();
                    let dest = dest_dbs.join(&fname);
                    copy_restore_db_file(&src, &dest).await?;
                    restored.push(format!("dbs/{fname}"));
                }
            }
        }

        self.audit(
            None,
            &caller.actor,
            "backup.restore",
            serde_json::json!({
                "backup_dir": backup_dir.display().to_string(),
                "restored_files": restored,
            }),
        );
        Ok(restored)
    }

    pub(super) fn require_admin(caller: &Caller) -> Result<()> {
        if caller.scope.allows_admin() {
            Ok(())
        } else {
            Err(FerroError::Forbidden("admin scope required".into()))
        }
    }

    pub(super) fn require_project_read(caller: &Caller, project_id: &str) -> Result<()> {
        if caller.scope.allows_project_read(project_id) {
            Ok(())
        } else {
            Err(FerroError::Forbidden(format!(
                "read access denied for project '{project_id}'"
            )))
        }
    }

    pub(super) fn require_project_write(caller: &Caller, project_id: &str) -> Result<()> {
        if caller.scope.allows_project_write(project_id) {
            Ok(())
        } else {
            Err(FerroError::Forbidden(format!(
                "write access denied for project '{project_id}'"
            )))
        }
    }

    pub(super) fn require_resource_read(
        caller: &Caller,
        owner_project_id: Option<&str>,
        resource_desc: &str,
    ) -> Result<()> {
        if caller.scope.allows_resource_read(owner_project_id) {
            Ok(())
        } else {
            Err(FerroError::Forbidden(format!(
                "read access denied for {resource_desc}"
            )))
        }
    }

    pub(super) fn require_resource_write(
        caller: &Caller,
        owner_project_id: Option<&str>,
        resource_desc: &str,
    ) -> Result<()> {
        if caller.scope.allows_resource_write(owner_project_id) {
            Ok(())
        } else {
            Err(FerroError::Forbidden(format!(
                "write access denied for {resource_desc}"
            )))
        }
    }

    pub(super) async fn get_project_internal(&self, id_or_name: &str) -> Result<Project> {
        self.projects
            .get_project(id_or_name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("project '{id_or_name}'")))
    }

    pub(super) async fn resolve_optional_project_id(
        &self,
        project_id: Option<&str>,
    ) -> Result<Option<String>> {
        match project_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => {
                let proj = self.get_project_internal(raw).await?;
                Ok(Some(proj.id))
            }
            None => Ok(None),
        }
    }
}

fn normalize_resource_group(group: Option<&str>) -> Result<Option<&str>> {
    let group = group.map(str::trim).filter(|name| !name.is_empty());
    if let Some(name) = group {
        if name.chars().count() > 80 || name.chars().any(char::is_control) {
            return Err(FerroError::InvalidArgument(
                "resource group must be at most 80 characters and contain no control characters"
                    .into(),
            ));
        }
    }
    Ok(group)
}

async fn copy_restore_db_file(src: &Path, dest: &Path) -> Result<()> {
    for suffix in ["-wal", "-log", "-shm"] {
        let dest_sidecar = std::path::PathBuf::from(format!("{}{}", dest.display(), suffix));
        if dest_sidecar.exists() {
            let _ = tokio::fs::remove_file(&dest_sidecar).await;
        }
    }
    crate::storage::tx::copy_db_with_sidecars(src, dest).await
}
