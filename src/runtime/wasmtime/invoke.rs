use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use http::Method;

use crate::core::model::{
    Deployment, FerroError, Job, JobOutcome, RequestCtx, Result, WorkerHttpRequest,
    WorkerHttpResponse,
};
use crate::core::traits::WorkerRuntime;

use super::{queue_wit, scheduled_wit, wit_qc, wit_sched, WasmtimeRuntime};

pub(super) fn parse_cron_http_target(raw_target: &str) -> (String, String) {
    let trimmed = raw_target.trim();
    match trimmed.split_once(char::is_whitespace) {
        Some((m, p)) => {
            let method = Method::from_bytes(m.trim().as_bytes())
                .ok()
                .filter(|m| {
                    matches!(
                        *m,
                        Method::GET
                            | Method::POST
                            | Method::PUT
                            | Method::DELETE
                            | Method::PATCH
                            | Method::HEAD
                    )
                })
                .map(|m| m.to_string())
                .unwrap_or_else(|| "POST".into());
            let p = p.trim();
            let path = if p.starts_with('/') {
                p
            } else {
                "/_internal/cron"
            };
            (method, path.to_string())
        }
        None => (
            "POST".into(),
            if trimmed.starts_with('/') {
                trimmed.to_string()
            } else {
                "/_internal/cron".to_string()
            },
        ),
    }
}

#[async_trait]
impl WorkerRuntime for WasmtimeRuntime {
    async fn store_artifact(
        &self,
        project_id: &str,
        rel_path: &str,
        artifact: Bytes,
    ) -> Result<(String, bool)> {
        let (stored, precompiled, _) = self
            .compile_or_store_artifact(project_id, rel_path, artifact)
            .await?;
        Ok((stored, precompiled))
    }

    async fn compile_and_load(&self, deployment: &Deployment, artifact: Bytes) -> Result<()> {
        let raw_rel = deployment
            .artifact_path
            .as_deref()
            .unwrap_or("worker.cwasm");
        let (cwasm_rel, _, comp) = self
            .compile_or_store_artifact(&deployment.project_id, raw_rel, artifact)
            .await?;
        self.insert_cached(deployment.id.clone(), comp, Some(cwasm_rel))
            .await;
        Ok(())
    }

    async fn preload_deployment(&self, deployment: &Deployment) -> Result<()> {
        if let Some(raw_path) = deployment.artifact_path.as_deref() {
            let (_, disk_path, _, cwasm_disk) =
                self.resolve_project_artifact_paths(&deployment.project_id, raw_path)?;
            let has_disk = tokio::fs::try_exists(&cwasm_disk).await.unwrap_or(false)
                || tokio::fs::try_exists(&disk_path).await.unwrap_or(false);
            if !has_disk {
                return Err(FerroError::NotFound(format!(
                    "artifact '{raw_path}' not found under artifacts directory"
                )));
            }
            let _ = self.get_or_load_component(deployment).await?;
        }
        Ok(())
    }

    async fn delete_project_artifacts(&self, project_id: &str) -> Result<()> {
        if project_id.is_empty()
            || project_id == "."
            || project_id == ".."
            || project_id.contains('/')
            || project_id.contains('\\')
        {
            return Err(FerroError::InvalidArgument(format!(
                "invalid project id '{project_id}'"
            )));
        }
        self.invalidate_project_cache(project_id).await;
        let dir = self.config.artifacts_dir().join(project_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(FerroError::Internal(anyhow!(
                "remove project artifacts '{}': {e}",
                dir.display()
            ))),
        }
    }

    async fn gc_unreferenced_artifacts(
        &self,
        referenced_paths: &HashSet<String>,
        grace_secs: u64,
    ) -> Result<usize> {
        let artifacts_dir = self.config.artifacts_dir();
        if !tokio::fs::try_exists(&artifacts_dir).await.unwrap_or(false) {
            return Ok(0);
        }

        let mut keep_rel = HashSet::with_capacity(referenced_paths.len() * 2);
        for raw in referenced_paths {
            if let Ok(clean) = Self::sanitize_rel_path(raw) {
                keep_rel.insert(Self::to_cwasm_rel_path(&clean));
                keep_rel.insert(clean);
            }
        }

        let mut stack = vec![artifacts_dir.clone()];
        let mut subdirs = Vec::new();
        let mut removed_rels = Vec::new();
        let now = SystemTime::now();

        while let Some(dir) = stack.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(rd) => rd,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    return Err(FerroError::Internal(anyhow!(
                        "read artifacts dir '{}': {e}",
                        dir.display()
                    )))
                }
            };
            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                FerroError::Internal(anyhow!("iterate artifacts dir '{}': {e}", dir.display()))
            })? {
                let path = entry.path();
                let meta = match entry.metadata().await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if meta.is_dir() {
                    subdirs.push(path.clone());
                    stack.push(path);
                    continue;
                }
                if !meta.is_file() {
                    continue;
                }
                let fname = entry.file_name();
                let fname_lossy = fname.to_string_lossy();
                let effective_grace = if fname_lossy.contains(".tmp.") {
                    grace_secs.max(60)
                } else {
                    grace_secs
                };
                if effective_grace > 0 {
                    if let Ok(modified) = meta.modified() {
                        if now.duration_since(modified).unwrap_or_default()
                            < Duration::from_secs(effective_grace)
                        {
                            continue;
                        }
                    }
                }
                let Ok(rel) = path.strip_prefix(&artifacts_dir) else {
                    continue;
                };
                let rel_str = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                if !keep_rel.contains(&rel_str) {
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => removed_rels.push(rel_str),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            return Err(FerroError::Internal(anyhow!(
                                "remove unreferenced artifact '{}': {e}",
                                path.display()
                            )));
                        }
                    }
                }
            }
        }

        for subdir in subdirs.into_iter().rev() {
            let _ = tokio::fs::remove_dir(&subdir).await;
        }

        let removed = removed_rels.len();
        if !removed_rels.is_empty() {
            let removed_set: HashSet<String> = removed_rels.into_iter().collect();
            let mut guard = self.cache.lock().await;
            guard.retain(|k, v| {
                !removed_set.contains(k)
                    && !v
                        .artifact_key
                        .as_deref()
                        .is_some_and(|a| removed_set.contains(a))
            });
        }
        Ok(removed)
    }

    async fn handle_http(
        &self,
        ctx: RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse> {
        let component = self.get_or_load_component(&ctx.deployment).await?;
        self.invoke_http_component(&component, &ctx, req).await
    }

    async fn handle_queue(&self, ctx: RequestCtx, jobs: Vec<Job>) -> Result<Vec<JobOutcome>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let component = self.get_or_load_component(&ctx.deployment).await?;
        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| FerroError::Internal(anyhow!("instantiate_pre failed: {e:#}")))?;

        let queue_name = jobs[0].queue.clone();

        if let Ok(queue_pre) = queue_wit::QueueWorkerPre::new(instance_pre) {
            let (mut store, stdout, stderr) = self.build_store(&ctx)?;
            let wit_jobs: Vec<wit_qc::QueueJob> = jobs
                .iter()
                .map(|j| wit_qc::QueueJob {
                    id: j.id,
                    queue: j.queue.clone(),
                    payload: j.payload.clone(),
                    attempts: j.attempts,
                    max_attempts: j.max_attempts,
                    dedup_key: j.dedup_key.clone(),
                    created_at: j.created_at,
                })
                .collect();

            let eff_timeout_ms = ctx
                .timeout_ms
                .filter(|t| *t > 0)
                .unwrap_or_else(|| self.default_epoch_timeout_ms());
            let timeout_dur = Duration::from_millis(eff_timeout_ms + 500);
            let exec = tokio::time::timeout(timeout_dur, async {
                let worker = queue_pre
                    .instantiate_async(&mut store)
                    .await
                    .map_err(Self::map_exec_err)?;
                worker
                    .ferropod_platform_queue_consumer()
                    .call_handle_batch(&mut store, &queue_name, &wit_jobs)
                    .await
                    .map_err(Self::map_exec_err)
            })
            .await;

            self.flush_stdio_logs(&ctx, &stdout, &stderr);

            let outcomes = match exec {
                Ok(res) => res?,
                Err(_) => {
                    return Err(FerroError::Internal(anyhow!(
                        "queue worker execution timed out"
                    )));
                }
            };

            let mapped = outcomes
                .into_iter()
                .map(|o| match o {
                    wit_qc::JobOutcome::Ack => JobOutcome::Ack,
                    wit_qc::JobOutcome::Retry(r) => JobOutcome::Retry {
                        delay_secs: r.delay_secs,
                        error: r.error,
                    },
                    wit_qc::JobOutcome::Dead(err) => JobOutcome::Dead { error: err },
                })
                .collect();
            return Ok(mapped);
        }

        // HTTP fallback for workers that only export `wasi:http/incoming-handler`.
        let payload_json = serde_json::to_vec(&jobs)
            .map_err(|e| FerroError::Internal(anyhow!("serialize queue batch: {e}")))?;
        let req = WorkerHttpRequest {
            method: "POST".into(),
            uri: format!("/_internal/queue/{queue_name}"),
            headers: vec![
                ("content-type".into(), b"application/json".to_vec()),
                ("host".into(), b"localhost".to_vec()),
            ],
            body: Box::pin(futures_util::stream::once(async move {
                Ok::<_, std::io::Error>(Bytes::from(payload_json))
            })),
        };
        let resp = self.invoke_http_component(&component, &ctx, req).await?;
        let ok = (200..300).contains(&resp.status);
        let outcome = if ok {
            JobOutcome::Ack
        } else {
            JobOutcome::Retry {
                delay_secs: 5,
                error: format!("queue HTTP handler returned status {}", resp.status),
            }
        };
        Ok(vec![outcome; jobs.len()])
    }

    async fn handle_cron(&self, ctx: RequestCtx, expr: &str) -> Result<()> {
        let component = self.get_or_load_component(&ctx.deployment).await?;
        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| FerroError::Internal(anyhow!("instantiate_pre failed: {e:#}")))?;

        let cron_job = ctx.cron_job.clone();
        if let Ok(sched_pre) = scheduled_wit::ScheduledWorkerPre::new(instance_pre) {
            let (mut store, stdout, stderr) = self.build_store(&ctx)?;
            let event = match &cron_job {
                Some(cj) => wit_sched::CronEvent {
                    cron_id: cj.id.clone(),
                    name: cj.name.clone(),
                    schedule: expr.to_string(),
                    target: cj.target.clone(),
                    scheduled_time_ms: Utc::now().timestamp_millis(),
                },
                None => wit_sched::CronEvent {
                    cron_id: ctx.request_id.clone(),
                    name: "cron".into(),
                    schedule: expr.to_string(),
                    target: "/".into(),
                    scheduled_time_ms: Utc::now().timestamp_millis(),
                },
            };

            let eff_timeout_ms = ctx
                .timeout_ms
                .filter(|t| *t > 0)
                .unwrap_or_else(|| self.default_epoch_timeout_ms());
            let timeout_dur = Duration::from_millis(eff_timeout_ms + 500);
            let exec = tokio::time::timeout(timeout_dur, async {
                let worker = sched_pre
                    .instantiate_async(&mut store)
                    .await
                    .map_err(Self::map_exec_err)?;
                worker
                    .ferropod_platform_scheduled()
                    .call_handle_cron(&mut store, &event)
                    .await
                    .map_err(Self::map_exec_err)
            })
            .await;

            self.flush_stdio_logs(&ctx, &stdout, &stderr);

            let guest_res = match exec {
                Ok(res) => res?,
                Err(_) => {
                    return Err(FerroError::Internal(anyhow!(
                        "scheduled worker execution timed out"
                    )));
                }
            };

            return guest_res.map_err(|e| FerroError::Internal(anyhow!("cron handler error: {e}")));
        }

        // HTTP fallback for workers that only export `wasi:http/incoming-handler`.
        let raw_target = cron_job
            .as_ref()
            .map(|c| c.target.as_str())
            .unwrap_or("/_internal/cron");
        let (method, path) = parse_cron_http_target(raw_target);
        let req = WorkerHttpRequest {
            method,
            uri: path,
            headers: vec![
                ("host".into(), b"localhost".to_vec()),
                ("x-ferropod-cron".into(), expr.as_bytes().to_vec()),
            ],
            body: Box::pin(futures_util::stream::empty()),
        };
        let resp = self.invoke_http_component(&component, &ctx, req).await?;
        if (200..300).contains(&resp.status) {
            Ok(())
        } else {
            Err(FerroError::Internal(anyhow!(
                "cron HTTP handler returned status {}",
                resp.status
            )))
        }
    }
}
