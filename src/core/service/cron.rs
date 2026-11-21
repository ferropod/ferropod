use chrono::{DateTime, Datelike, Timelike, Utc};
use uuid::Uuid;

use crate::core::model::{Caller, CronJob, FerroError, LogEntry, RequestCtx, Result, TokenScope};

use super::PlatformService;

fn parse_every_secs(schedule: &str) -> Option<i64> {
    let rest = schedule.trim().strip_prefix("@every")?.trim();
    if let Some(num) = rest.strip_suffix('s') {
        num.trim().parse::<i64>().ok().filter(|n| *n > 0)
    } else if let Some(num) = rest.strip_suffix('m') {
        num.trim()
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|n| n * 60)
    } else if let Some(num) = rest.strip_suffix('h') {
        num.trim()
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .map(|n| n * 3600)
    } else {
        None
    }
}

fn match_cron_field(
    field: &str,
    value: u32,
    min: u32,
    max: u32,
    allow_seven_as_zero: bool,
) -> bool {
    let norm_val = if allow_seven_as_zero && value == 7 {
        0
    } else {
        value
    };
    for part in field.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return false;
        }
        let (range_str, step) = match part.split_once('/') {
            Some((r, s)) => {
                let Ok(step_val) = s.trim().parse::<u32>() else {
                    return false;
                };
                if step_val == 0 {
                    return false;
                }
                (r.trim(), step_val)
            }
            None => (part, 1),
        };

        let (start, end) = if range_str == "*" {
            (min, max)
        } else if let Some((a, b)) = range_str.split_once('-') {
            let (Ok(mut s), Ok(mut e)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) else {
                return false;
            };
            if allow_seven_as_zero && s == 7 {
                s = 0;
            }
            if allow_seven_as_zero && e == 7 {
                e = 0;
            }
            if s > e || s < min || e > max {
                return false;
            }
            (s, e)
        } else {
            let Ok(mut exact) = range_str.parse::<u32>() else {
                return false;
            };
            if allow_seven_as_zero && exact == 7 {
                exact = 0;
            }
            if exact < min || exact > max {
                return false;
            }
            if step == 1 {
                if norm_val == exact {
                    return true;
                }
                continue;
            }
            (exact, max)
        };

        if norm_val >= start && norm_val <= end && (norm_val - start) % step == 0 {
            return true;
        }
    }
    false
}

/// Evaluate whether a cron expression matches the given UTC Unix timestamp (in seconds).
/// Supports standard 5-field expressions (`*`, `*/n`, `a-b`, `a,b`) plus `@hourly`, `@daily`,
/// `@midnight`, `@weekly`, `@monthly`, and `@every <N>(s|m|h)`.
pub fn cron_matches_timestamp(schedule: &str, ts_secs: i64) -> bool {
    let trimmed = schedule.trim();
    if let Some(secs) = parse_every_secs(trimmed) {
        return ts_secs.rem_euclid(secs) == 0;
    }
    let Some(dt) = DateTime::from_timestamp(ts_secs, 0) else {
        return false;
    };
    match trimmed {
        "@hourly" => dt.minute() == 0,
        "@daily" | "@midnight" => dt.minute() == 0 && dt.hour() == 0,
        "@weekly" => dt.minute() == 0 && dt.hour() == 0 && dt.weekday().num_days_from_sunday() == 0,
        "@monthly" => dt.minute() == 0 && dt.hour() == 0 && dt.day() == 1,
        other => {
            let fields: Vec<&str> = other.split_whitespace().collect();
            if fields.len() != 5 {
                return false;
            }
            match_cron_field(fields[0], dt.minute(), 0, 59, false)
                && match_cron_field(fields[1], dt.hour(), 0, 23, false)
                && match_cron_field(fields[2], dt.day(), 1, 31, false)
                && match_cron_field(fields[3], dt.month(), 1, 12, false)
                && match_cron_field(fields[4], dt.weekday().num_days_from_sunday(), 0, 6, true)
        }
    }
}

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
                return Err(FerroError::InvalidArgument(
                    "schedule cannot be empty".into(),
                ));
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

    async fn execute_cron_job_internal(
        &self,
        mut job: CronJob,
        actor: &str,
        manual: bool,
    ) -> Result<CronJob> {
        let now = Utc::now().timestamp_millis();
        let request_id = format!("cron_{}", Uuid::new_v4().simple());
        let mut status = "ok".to_string();
        let mut level = "info";
        let mut err_msg: Option<String> = None;

        if let Some(pid) = job.project_id.as_deref() {
            if let Some(proj) = self.projects.get_project(pid).await? {
                if let Some(dep_id) = proj.active_deployment_id.as_deref() {
                    if let Some(dep) = self.projects.get_deployment(dep_id).await? {
                        if dep.artifact_path.is_some() {
                            let ctx = RequestCtx::new(proj.id.clone(), request_id.clone(), dep)
                                .with_cron_job(job.clone());
                            if let Err(e) = self.runtime.handle_cron(ctx, &job.schedule).await {
                                let msg = e.to_string();
                                status = format!("error: {msg}");
                                level = "error";
                                err_msg = Some(msg);
                            }
                        }
                    }
                }
            }
        }

        job.last_run_at = Some(now);
        job.last_status = Some(status.clone());
        job.updated_at = now;
        self.registry.update_cron_job(&job).await?;

        self.logs.emit(LogEntry {
            id: None,
            ts: now,
            project_id: job.project_id.clone(),
            request_id: Some(request_id),
            source: "cron".into(),
            level: level.into(),
            message: match &err_msg {
                Some(e) => format!("Cron job '{}' failed: {e}", job.name),
                None => format!(
                    "Cron job '{}' {} -> {}",
                    job.name,
                    if manual {
                        "triggered manually"
                    } else {
                        "executed"
                    },
                    job.target
                ),
            },
            meta_json: Some(
                serde_json::json!({
                    "cron_id": job.id,
                    "schedule": job.schedule,
                    "target": job.target,
                    "status": status,
                    "actor": actor,
                })
                .to_string(),
            ),
        });
        Ok(job)
    }

    pub async fn trigger_cron_job(&self, caller: &Caller, id: &str) -> Result<CronJob> {
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
        self.execute_cron_job_internal(job, &caller.actor, true)
            .await
    }

    /// Evaluate all enabled cron jobs and execute any job that is due in the current schedule window.
    pub async fn run_due_cron_jobs_once(&self) -> Result<usize> {
        let jobs = self.registry.list_cron_jobs().await?;
        let now_ms = Utc::now().timestamp_millis();
        let now_secs = now_ms / 1000;
        let current_min = now_secs / 60;
        let mut executed = 0usize;

        for job in jobs {
            if !job.enabled {
                continue;
            }
            let due = if let Some(interval_secs) = parse_every_secs(&job.schedule) {
                job.last_run_at
                    .map(|last| (now_ms - last) >= interval_secs * 1000)
                    .unwrap_or(true)
            } else {
                cron_matches_timestamp(&job.schedule, now_secs)
                    && job
                        .last_run_at
                        .map(|last| (last / 1000) / 60 < current_min)
                        .unwrap_or(true)
            };
            if due {
                self.execute_cron_job_internal(job, "scheduler", false)
                    .await?;
                executed += 1;
            }
        }
        Ok(executed)
    }
}
