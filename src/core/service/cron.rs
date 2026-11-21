use chrono::Utc;
use uuid::Uuid;

use crate::core::model::{Caller, CronJob, FerroError, LogEntry, Result, TokenScope};

use super::PlatformService;

impl PlatformService {
    pub async fn list_cron_jobs(&self, caller: &Caller) -> Result<Vec<CronJob>> {
        let all = self.registry.list_cron_jobs().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|j| j.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn create_cron_job(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        schedule: &str,
        target: &str,
    ) -> Result<CronJob> {
        let name = name.trim();
        let schedule = schedule.trim();
        let target = target.trim();
        if name.is_empty() || schedule.is_empty() || target.is_empty() {
            return Err(FerroError::InvalidArgument(
                "cron name, schedule, and target are required".into(),
            ));
        }
        let resolved_pid = match (&caller.scope, project_id) {
            (TokenScope::Admin, pid) => self.resolve_optional_project_id(pid).await?,
            (TokenScope::Project(_), Some(req_pid)) => {
                let proj = self.get_project_internal(req_pid).await?;
                Self::require_project_write(caller, &proj.id)?;
                Some(proj.id)
            }
            (TokenScope::Project(scope_pid), None) => Some(scope_pid.clone()),
            (TokenScope::Readonly, _) => {
                return Err(FerroError::Forbidden(
                    "readonly token cannot create cron jobs".into(),
                ));
            }
        };

        let now = Utc::now().timestamp_millis();
        let job = CronJob {
            id: format!("cron_{}", &Uuid::new_v4().simple().to_string()[..12]),
            name: name.to_string(),
            project_id: resolved_pid.clone(),
            schedule: schedule.to_string(),
            target: target.to_string(),
            enabled: true,
            last_run_at: None,
            last_status: None,
            created_at: now,
            updated_at: now,
        };
        let saved = self.registry.create_cron_job(job).await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "cron.create",
            serde_json::json!({ "id": saved.id, "name": saved.name, "schedule": saved.schedule }),
        );
        Ok(saved)
    }

    pub async fn update_cron_job(
        &self,
        caller: &Caller,
        id: &str,
        enabled: Option<bool>,
        schedule: Option<&str>,
        target: Option<&str>,
    ) -> Result<CronJob> {
        let mut job = self
            .registry
            .get_cron_job(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("cron job '{id}'")))?;
        Self::require_resource_write(
            caller,
            job.project_id.as_deref(),
            &format!("cron job '{}'", job.name),
        )?;
        if let Some(en) = enabled {
            job.enabled = en;
        }
        if let Some(sc) = schedule {
            let sc = sc.trim();
            if sc.is_empty() {
                return Err(FerroError::InvalidArgument("schedule cannot be empty".into()));
            }
            job.schedule = sc.to_string();
        }
        if let Some(tg) = target {
            let tg = tg.trim();
            if tg.is_empty() {
                return Err(FerroError::InvalidArgument("target cannot be empty".into()));
            }
            job.target = tg.to_string();
        }
        job.updated_at = Utc::now().timestamp_millis();
        self.registry.update_cron_job(&job).await?;
        self.audit(
            job.project_id.as_deref(),
            &caller.actor,
            "cron.update",
            serde_json::json!({ "id": job.id, "enabled": job.enabled }),
        );
        Ok(job)
    }

    pub async fn delete_cron_job(&self, caller: &Caller, id: &str) -> Result<()> {
        let job = self
            .registry
            .get_cron_job(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("cron job '{id}'")))?;
        Self::require_resource_write(
            caller,
            job.project_id.as_deref(),
            &format!("cron job '{}'", job.name),
        )?;
        self.registry.delete_cron_job(&job.id).await?;
        self.audit(
            job.project_id.as_deref(),
            &caller.actor,
            "cron.delete",
            serde_json::json!({ "id": job.id, "name": job.name }),
        );
        Ok(())
    }

    pub async fn trigger_cron_job(&self, caller: &Caller, id: &str) -> Result<CronJob> {
        let mut job = self
            .registry
            .get_cron_job(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("cron job '{id}'")))?;
        Self::require_resource_write(
            caller,
            job.project_id.as_deref(),
            &format!("cron job '{}'", job.name),
        )?;

        let now = Utc::now().timestamp_millis();
        job.last_run_at = Some(now);
        job.last_status = Some("ok".into());
        job.updated_at = now;
        self.registry.update_cron_job(&job).await?;

        self.logs.emit(LogEntry {
            id: None,
            ts: now,
            project_id: job.project_id.clone(),
            request_id: None,
            source: "cron".into(),
            level: "info".into(),
            message: format!("Cron job '{}' triggered manually -> {}", job.name, job.target),
            meta_json: Some(
                serde_json::json!({
                    "cron_id": job.id,
                    "schedule": job.schedule,
                    "target": job.target,
                    "actor": caller.actor,
                })
                .to_string(),
            ),
        });
        Ok(job)
    }
}
