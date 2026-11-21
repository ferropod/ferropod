use bytes::Bytes;

use crate::core::model::{
    Caller, EnqueueOpts, FerroError, Job, JobStatus, QueueConfig, QueueStats, Result, TokenScope,
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

    pub async fn queue_create(
        &self,
        caller: &Caller,
        mut cfg: QueueConfig,
    ) -> Result<QueueConfig> {
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
}
