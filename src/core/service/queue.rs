use bytes::Bytes;
use chrono::Utc;
use uuid::Uuid;

use crate::core::model::{
    Caller, EnqueueOpts, FerroError, Job, JobOutcome, JobStatus, LogEntry, QueueConfig, QueueStats,
    RequestCtx, Result, TokenScope,
};

use super::PlatformService;

impl PlatformService {
    async fn require_queue_read(&self, caller: &Caller, queue: &str) -> Result<QueueConfig> {
        let cfg = self
            .queue
            .get_queue(queue)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("queue '{queue}'")))?;
        let is_consumer = caller.project_id().is_some()
            && cfg.consumer_project_id.as_deref() == caller.project_id();
        if !is_consumer {
            Self::require_resource_read(
                caller,
                cfg.project_id.as_deref(),
                &format!("queue '{queue}'"),
            )?;
        }
        Ok(cfg)
    }

    async fn require_queue_write(&self, caller: &Caller, queue: &str) -> Result<QueueConfig> {
        let cfg = self
            .queue
            .get_queue(queue)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("queue '{queue}'")))?;
        Self::require_resource_write(
            caller,
            cfg.project_id.as_deref(),
            &format!("queue '{queue}'"),
        )?;
        Ok(cfg)
    }

    pub async fn queue_create(&self, caller: &Caller, mut cfg: QueueConfig) -> Result<QueueConfig> {
        cfg.group = super::normalize_resource_group(cfg.group.as_deref())?.map(str::to_owned);
        cfg.name = cfg.name.trim().to_string();
        if cfg.name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "queue name cannot be empty".into(),
            ));
        }
        let resolved_pid = match (&caller.scope, cfg.project_id.as_deref()) {
            (TokenScope::Admin, pid) => self.resolve_optional_project_id(pid).await?,
            (TokenScope::Project(_), Some(req_pid)) => {
                let proj = self.get_project_internal(req_pid).await?;
                Self::require_project_write(caller, &proj.id)?;
                Some(proj.id)
            }
            (TokenScope::Project(scope_pid), None) => Some(scope_pid.clone()),
            (TokenScope::Readonly, _) => {
                return Err(FerroError::Forbidden(
                    "readonly token cannot create queues".into(),
                ));
            }
        };
        cfg.project_id = resolved_pid.clone();
        cfg.consumer_project_id = self
            .resolve_optional_project_id(cfg.consumer_project_id.as_deref())
            .await?;
        if let Some(ref consumer_pid) = cfg.consumer_project_id {
            Self::require_project_write(caller, consumer_pid)?;
        }

        self.queue.create_queue(cfg.clone()).await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "queue.create",
            serde_json::json!({ "name": cfg.name }),
        );
        Ok(cfg)
    }

    pub async fn queue_list(&self, caller: &Caller) -> Result<Vec<QueueConfig>> {
        let all = self.queue.list_queues().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|q| q.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn queue_delete(&self, caller: &Caller, queue: &str) -> Result<()> {
        let cfg = self.require_queue_write(caller, queue).await?;
        self.queue.delete_queue(queue).await?;
        self.audit(
            cfg.project_id.as_deref(),
            &caller.actor,
            "queue.delete",
            serde_json::json!({ "name": queue }),
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn queue_update(
        &self,
        caller: &Caller,
        queue: &str,
        max_concurrency: Option<i64>,
        rate_limit_per_sec: Option<Option<i64>>,
        batch_size: Option<i64>,
        lease_secs: Option<i64>,
        max_attempts: Option<i64>,
    ) -> Result<QueueConfig> {
        let mut cfg = self.require_queue_write(caller, queue).await?;
        if let Some(v) = max_concurrency {
            cfg.max_concurrency = v.max(1);
        }
        if let Some(v) = rate_limit_per_sec {
            cfg.rate_limit_per_sec = v.filter(|n| *n > 0);
        }
        if let Some(v) = batch_size {
            cfg.batch_size = v.max(1);
        }
        if let Some(v) = lease_secs {
            cfg.lease_secs = v.max(5);
        }
        if let Some(v) = max_attempts {
            cfg.max_attempts = v.max(1);
        }
        self.queue.update_queue(cfg.clone()).await?;
        self.audit(
            cfg.project_id.as_deref(),
            &caller.actor,
            "queue.update",
            serde_json::json!({
                "name": cfg.name,
                "max_concurrency": cfg.max_concurrency,
                "rate_limit_per_sec": cfg.rate_limit_per_sec,
            }),
        );
        Ok(cfg)
    }

    pub async fn queue_enqueue(
        &self,
        caller: &Caller,
        queue: &str,
        payload: Bytes,
        opts: EnqueueOpts,
    ) -> Result<i64> {
        self.require_queue_write(caller, queue).await?;
        self.queue.enqueue(queue, payload, opts).await
    }

    pub async fn queue_stats(&self, caller: &Caller, queue: &str) -> Result<QueueStats> {
        self.require_queue_read(caller, queue).await?;
        self.queue.stats(queue).await
    }

    pub async fn queue_inspect(
        &self,
        caller: &Caller,
        queue: &str,
        status: JobStatus,
        limit: usize,
    ) -> Result<Vec<Job>> {
        self.require_queue_read(caller, queue).await?;
        self.queue.inspect(queue, status, limit).await
    }

    pub async fn queue_dlq_replay(
        &self,
        caller: &Caller,
        queue: &str,
        job_ids: Option<&[i64]>,
    ) -> Result<u64> {
        let cfg = self.require_queue_write(caller, queue).await?;
        let replayed = self.queue.dlq_replay(queue, job_ids).await?;
        self.audit(
            cfg.project_id.as_deref(),
            &caller.actor,
            "queue.dlq_replay",
            serde_json::json!({ "queue": queue, "replayed": replayed }),
        );
        Ok(replayed)
    }

    pub async fn queue_purge(&self, caller: &Caller, queue: &str) -> Result<u64> {
        let cfg = self.require_queue_write(caller, queue).await?;
        let purged = self.queue.purge(queue).await?;
        self.audit(
            cfg.project_id.as_deref(),
            &caller.actor,
            "queue.purge",
            serde_json::json!({ "queue": queue, "purged": purged }),
        );
        Ok(purged)
    }

    /// Poll all queues with a configured `consumer_project_id`, dispatch claimed batches to
    /// the Wasmtime runtime, and apply `Ack` / `Retry` / `Dead` outcomes.
    pub async fn run_queue_dispatch_once(&self) -> Result<usize> {
        let queues = self.queue.list_queues().await?;
        let mut dispatched = 0usize;

        for cfg in queues {
            let Some(consumer_pid) = cfg.consumer_project_id.as_deref() else {
                continue;
            };
            let Some(proj) = self.projects.get_project(consumer_pid).await? else {
                continue;
            };
            let Some(dep_id) = proj.active_deployment_id.as_deref() else {
                continue;
            };
            let Some(dep) = self.projects.get_deployment(dep_id).await? else {
                continue;
            };
            if dep.artifact_path.is_none() {
                continue;
            }

            let limit = cfg.batch_size.clamp(1, 100) as usize;
            let lease_secs = cfg.lease_secs.max(5);
            let batch = self.queue.claim_batch(&cfg.name, limit, lease_secs).await?;
            if batch.is_empty() {
                continue;
            }

            let request_id = format!("queue_{}", Uuid::new_v4().simple());
            let ctx = RequestCtx::new(proj.id.clone(), request_id.clone(), dep);
            match self.runtime.handle_queue(ctx, batch.clone()).await {
                Ok(outcomes) => {
                    let mut ack_ids = Vec::new();
                    for (idx, job) in batch.iter().enumerate() {
                        match outcomes.get(idx) {
                            Some(JobOutcome::Ack) => ack_ids.push(job.id),
                            Some(JobOutcome::Retry { delay_secs, error }) => {
                                self.queue.nack(job.id, (*delay_secs).max(1), error).await?;
                            }
                            Some(JobOutcome::Dead { error }) => {
                                self.queue.mark_dead(job.id, error).await?;
                            }
                            None => {
                                self.queue
                                    .nack(job.id, 5, "worker returned fewer outcomes than jobs")
                                    .await?;
                            }
                        }
                    }
                    if !ack_ids.is_empty() {
                        self.queue.ack_batch(&ack_ids).await?;
                    }
                    dispatched += batch.len();
                }
                Err(err) => {
                    let msg = err.to_string();
                    self.logs.emit(LogEntry {
                        id: None,
                        ts: Utc::now().timestamp_millis(),
                        project_id: Some(proj.id.clone()),
                        request_id: Some(request_id),
                        source: "queue".into(),
                        level: "error".into(),
                        message: format!("Queue '{}' consumer failed: {msg}", cfg.name),
                        meta_json: None,
                    });
                    for job in &batch {
                        let exp = job.attempts.clamp(1, 6) as u32;
                        let delay = (1u64 << exp).min(300);
                        let _ = self.queue.nack(job.id, delay, &msg).await;
                    }
                }
            }
        }

        Ok(dispatched)
    }
}
