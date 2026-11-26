use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use chrono::{DateTime, Datelike, Timelike, Utc};
use futures_util::stream::{self, StreamExt};
use uuid::Uuid;

struct CronInFlightGuard {
    set: Arc<std::sync::Mutex<HashSet<String>>>,
    job_id: String,
}

impl Drop for CronInFlightGuard {
    fn drop(&mut self) {
        let mut guard = self.set.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(&self.job_id);
    }
}

use crate::core::model::{
    Caller, CronJob, CronJobOpts, FerroError, LogEntry, RequestCtx, Result, TokenScope,
};

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
            .and_then(|n| n.checked_mul(60))
    } else if let Some(num) = rest.strip_suffix('h') {
        num.trim()
            .parse::<i64>()
            .ok()
            .filter(|n| *n > 0)
            .and_then(|n| n.checked_mul(3600))
    } else {
        None
    }
}

fn parse_cron_part(
    part: &str,
    min: u32,
    max: u32,
    allow_seven_as_zero: bool,
) -> Option<(u32, u32, u32)> {
    let part = part.trim();
    if part.is_empty() {
        return None;
    }
    let (range_str, step) = match part.split_once('/') {
        Some((r, s)) => {
            let step_val = s.trim().parse::<u32>().ok().filter(|v| *v > 0)?;
            (r.trim(), step_val)
        }
        None => (part, 1),
    };

    if range_str == "*" {
        return Some((min, max, step));
    }
    if let Some((a, b)) = range_str.split_once('-') {
        let mut s = a.trim().parse::<u32>().ok()?;
        let mut e = b.trim().parse::<u32>().ok()?;
        if allow_seven_as_zero && s == 7 {
            s = 0;
        }
        if allow_seven_as_zero && e == 7 {
            e = 0;
        }
        if s > e || s < min || e > max {
            return None;
        }
        return Some((s, e, step));
    }
    let mut exact = range_str.parse::<u32>().ok()?;
    if allow_seven_as_zero && exact == 7 {
        exact = 0;
    }
    if exact < min || exact > max {
        return None;
    }
    let end = if step == 1 { exact } else { max };
    Some((exact, end, step))
}

fn validate_cron_field(field: &str, min: u32, max: u32, allow_seven_as_zero: bool) -> bool {
    let mut any = false;
    for part in field.split(',') {
        any = true;
        if parse_cron_part(part, min, max, allow_seven_as_zero).is_none() {
            return false;
        }
    }
    any
}

/// Validate a cron schedule string before persisting it.
pub fn validate_cron_schedule(schedule: &str) -> Result<()> {
    let trimmed = schedule.trim();
    if trimmed.is_empty() {
        return Err(FerroError::InvalidArgument(
            "cron schedule cannot be empty".into(),
        ));
    }
    if parse_every_secs(trimmed).is_some()
        || matches!(
            trimmed,
            "@hourly" | "@daily" | "@midnight" | "@weekly" | "@monthly"
        )
    {
        return Ok(());
    }
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() == 5
        && validate_cron_field(fields[0], 0, 59, false)
        && validate_cron_field(fields[1], 0, 23, false)
        && validate_cron_field(fields[2], 1, 31, false)
        && validate_cron_field(fields[3], 1, 12, false)
        && validate_cron_field(fields[4], 0, 6, true)
    {
        return Ok(());
    }
    Err(FerroError::InvalidArgument(format!(
        "invalid cron schedule '{trimmed}': expected 5 fields, @hourly/@daily/@weekly/@monthly, or @every <N>(s|m|h)"
    )))
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
        let Some((start, end, step)) = parse_cron_part(part, min, max, allow_seven_as_zero) else {
            return false;
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
        self.create_cron_job_with_opts(
            caller,
            name,
            CronJobOpts {
                project_id: project_id.map(str::to_owned),
                schedule: Some(schedule.to_owned()),
                target: Some(target.to_owned()),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn create_cron_job_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        opts: CronJobOpts,
    ) -> Result<CronJob> {
        let name = name.trim();
        let schedule = opts.schedule.as_deref().unwrap_or("").trim();
        let target = opts.target.as_deref().unwrap_or("").trim();
        if name.is_empty() || schedule.is_empty() || target.is_empty() {
            return Err(FerroError::InvalidArgument(
                "cron name, schedule, and target are required".into(),
            ));
        }
        validate_cron_schedule(schedule)?;
        let resolved_pid = self
            .resolve_resource_owner(caller, opts.project_id.as_deref(), "cron jobs")
            .await?;

        let now = Utc::now().timestamp_millis();
        let description = opts
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let timezone = opts
            .timezone
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("UTC")
            .to_string();
        let job = CronJob {
            id: format!("cron_{}", &Uuid::new_v4().simple().to_string()[..12]),
            name: name.to_string(),
            project_id: resolved_pid.clone(),
            description,
            schedule: schedule.to_string(),
            target: target.to_string(),
            enabled: opts.enabled.unwrap_or(true),
            timezone,
            timeout_ms: opts.timeout_ms.filter(|t| *t >= 50),
            max_retries: opts.max_retries.unwrap_or(0).min(5),
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
        self.update_cron_job_with_opts(
            caller,
            id,
            CronJobOpts {
                enabled,
                schedule: schedule.map(str::to_owned),
                target: target.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn update_cron_job_with_opts(
        &self,
        caller: &Caller,
        id: &str,
        opts: CronJobOpts,
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
        if let Some(en) = opts.enabled {
            job.enabled = en;
        }
        if let Some(sc) = opts.schedule.as_deref() {
            let sc = sc.trim();
            validate_cron_schedule(sc)?;
            job.schedule = sc.to_string();
        }
        if let Some(tg) = opts.target.as_deref() {
            let tg = tg.trim();
            if tg.is_empty() {
                return Err(FerroError::InvalidArgument("target cannot be empty".into()));
            }
            job.target = tg.to_string();
        }
        if let Some(desc) = opts.description {
            let trimmed = desc.trim();
            job.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(tz) = opts.timezone {
            let trimmed = tz.trim();
            if !trimmed.is_empty() {
                job.timezone = trimmed.to_string();
            }
        }
        if let Some(t) = opts.timeout_ms {
            job.timeout_ms = if t == 0 { None } else { Some(t.max(50)) };
        }
        if let Some(r) = opts.max_retries {
            job.max_retries = r.min(5);
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
                            let max_tries = 1 + job.max_retries.min(5);
                            for attempt in 0..max_tries {
                                let ctx = RequestCtx::new(
                                    proj.id.clone(),
                                    request_id.clone(),
                                    dep.clone(),
                                )
                                .with_project_limits(&proj)
                                .with_cron_job(job.clone());
                                match self.runtime.handle_cron(ctx, &job.schedule).await {
                                    Ok(()) => {
                                        err_msg = None;
                                        break;
                                    }
                                    Err(e) => {
                                        err_msg = Some(e.to_string());
                                        if attempt + 1 < max_tries {
                                            tokio::time::sleep(std::time::Duration::from_millis(
                                                150,
                                            ))
                                            .await;
                                        }
                                    }
                                }
                            }
                        } else {
                            err_msg = Some("active deployment has no worker artifact".into());
                        }
                    } else {
                        err_msg = Some("active deployment was not found".into());
                    }
                } else {
                    err_msg = Some("app has no active deployment".into());
                }
            } else {
                err_msg = Some("app was not found".into());
            }
        } else {
            err_msg = Some("cron job has no app".into());
        }
        if let Some(msg) = &err_msg {
            status = format!("error: {msg}");
            level = "error";
            self.fire_alert(
                "cron_failure",
                format!("Cron job '{}' failed", job.name),
                serde_json::json!({
                    "cron_id": job.id,
                    "name": job.name,
                    "project_id": job.project_id,
                    "schedule": job.schedule,
                    "error": msg,
                }),
            );
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
        self.enforce_service_rate_limit(
            &format!("cron-trig:{id}:{}", caller.actor),
            None,
            30,
            &format!("cron job '{}'", job.name),
        )?;
        {
            let mut guard = self
                .limits
                .in_flight_crons
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if !guard.insert(job.id.clone()) {
                return Err(FerroError::Conflict(format!(
                    "cron job '{}' is already running",
                    job.name
                )));
            }
        }
        let _guard = CronInFlightGuard {
            set: self.limits.in_flight_crons.clone(),
            job_id: job.id.clone(),
        };
        self.execute_cron_job_internal(job, &caller.actor, true)
            .await
    }

    /// Evaluate all enabled cron jobs and execute any job that is due in the current schedule window.
    pub async fn run_due_cron_jobs_once(&self) -> Result<usize> {
        let jobs = self.registry.list_cron_jobs().await?;
        let now_ms = Utc::now().timestamp_millis();
        let now_secs = now_ms / 1000;
        let current_min = now_secs / 60;
        let mut due_jobs = Vec::new();

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
                let inserted = {
                    let mut guard = self
                        .limits
                        .in_flight_crons
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    guard.insert(job.id.clone())
                };
                if inserted {
                    let guard = CronInFlightGuard {
                        set: self.limits.in_flight_crons.clone(),
                        job_id: job.id.clone(),
                    };
                    due_jobs.push((job, guard));
                }
            }
        }

        let executed = AtomicUsize::new(0);
        let first_err: std::sync::Mutex<Option<FerroError>> = std::sync::Mutex::new(None);
        stream::iter(due_jobs)
            .for_each_concurrent(8, |(job, guard)| {
                let svc = self.clone();
                let executed = &executed;
                let first_err = &first_err;
                async move {
                    let _guard = guard;
                    match svc.execute_cron_job_internal(job, "scheduler", false).await {
                        Ok(_) => {
                            executed.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            let mut err_slot = first_err.lock().unwrap_or_else(|x| x.into_inner());
                            if err_slot.is_none() {
                                *err_slot = Some(e);
                            }
                        }
                    }
                }
            })
            .await;

        if let Some(err) = first_err.into_inner().unwrap_or_else(|x| x.into_inner()) {
            return Err(err);
        }
        Ok(executed.load(Ordering::Relaxed))
    }
}
