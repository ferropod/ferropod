mod blob;
mod cron;
mod db;
mod kv;
mod logs;
mod projects;
mod queue;
mod secrets;
mod tokens;

pub use cron::cron_matches_timestamp;
pub use logs::LogSubscription;
pub use projects::CreateDeploymentInput;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tracing::warn;

use crate::config::Config;
use crate::core::crypto::Crypto;
use crate::core::model::{Caller, FerroError, LogEntry, Project, Result, RouteEntry};
use crate::core::traits::{
    DbManager, KvStore, LogStore, ObjectStore, ProjectStore, QueueStore, RegistryStore,
    SecretStore, TokenStore, WorkerRuntime,
};
use crate::runtime::WasmtimeRuntime;

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
    runtime: Arc<dyn WorkerRuntime>,
    routes: Arc<RwLock<HashMap<String, RouteEntry>>>,
    http_concurrency: Arc<Semaphore>,
    route_refresh_lock: Arc<Mutex<()>>,
}

impl PlatformService {
    pub async fn new(config: Config, deps: PlatformDeps) -> Result<Self> {
        let crypto = Crypto::from_master_key(&config.master_key);
        let http_concurrency = Arc::new(Semaphore::new(config.max_concurrency));
        let wasmtime_rt = Arc::new(WasmtimeRuntime::new(config.clone())?);
        let runtime: Arc<dyn WorkerRuntime> = wasmtime_rt.clone();
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
            runtime,
            routes: Arc::new(RwLock::new(HashMap::new())),
            http_concurrency,
            route_refresh_lock: Arc::new(Mutex::new(())),
        };
        wasmtime_rt.bind_service(svc.clone());
        svc.refresh_routes().await?;
        Ok(svc)
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
        Ok(())
    }

    /// Execute one background maintenance cycle (KV TTL, queue lease reaper/retention, blob GC, log rotation).
    pub async fn run_maintenance_once(&self) {
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
                    self.config.queue_pending_retention_secs,
                    self.config.queue_dlq_retention_secs,
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
            .logs
            .cleanup_old_files(self.config.log_retention_days)
            .await
        {
            warn!(error = %e, "daily log cleanup error");
        }
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
        self.snapshot_user_dbs(caller, &out_dir.join("dbs")).await
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
