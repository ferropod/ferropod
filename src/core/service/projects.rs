use std::collections::HashMap;

use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use uuid::Uuid;

use crate::core::model::{
    Bindings, Caller, Deployment, FerroError, LogEntry, Project, RequestCtx, Result, Route,
    RouteEntry, TargetEnv, TokenScope, WorkerHttpRequest, WorkerHttpResponse,
};

use super::PlatformService;

/// Input payload for creating a new immutable deployment version.
#[derive(Debug, Clone, Default)]
pub struct CreateDeploymentInput {
    pub artifact_path: Option<String>,
    pub artifact_bytes: Option<Bytes>,
    pub assets_bucket: Option<String>,
    pub assets_prefix: Option<String>,
    pub spa_fallback: bool,
    pub bindings: Bindings,
    pub crons: Vec<String>,
    pub target_env: Option<TargetEnv>,
}

pub(crate) fn normalize_host(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_port = if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            &trimmed[..=end + 1]
        } else {
            trimmed
        }
    } else {
        match trimmed.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
                host
            }
            _ => trimmed,
        }
    };
    without_port.trim_end_matches('.').to_ascii_lowercase()
}

impl PlatformService {
    /// Reload the hot in-memory routing table from `ProjectStore` (serialized to prevent stale overwrites).
    pub async fn refresh_routes(&self) -> Result<()> {
        let _guard = self.route_refresh_lock.lock().await;
        let entries = self.projects.load_route_entries().await?;
        let mut map = HashMap::with_capacity(entries.len());
        for entry in entries {
            map.insert(normalize_host(&entry.host), entry);
        }
        let mut guard = self.routes.write().await;
        *guard = map;
        Ok(())
    }

    /// Look up an active route entry from the hot in-memory routing table.
    pub async fn lookup_route(&self, host: &str) -> Option<RouteEntry> {
        let key = normalize_host(host);
        if key.is_empty() {
            return None;
        }
        self.routes.read().await.get(&key).cloned()
    }

    /// Check whether a domain is present in the active routing table.
    pub async fn is_domain_allowed(&self, domain: &str) -> bool {
        let key = normalize_host(domain);
        if key.is_empty() {
            return false;
        }
        self.routes.read().await.contains_key(&key)
    }

    /// Return all normalized domains currently bound in the active routing table.
    pub async fn allowed_domains(&self) -> Vec<String> {
        let mut domains: Vec<String> = self.routes.read().await.keys().cloned().collect();
        domains.sort();
        domains
    }

    pub async fn create_project(
        &self,
        caller: &Caller,
        name: &str,
        health_check_path: Option<&str>,
    ) -> Result<Project> {
        self.create_grouped_project(caller, name, health_check_path, None)
            .await
    }

    pub async fn create_grouped_project(
        &self,
        caller: &Caller,
        name: &str,
        health_check_path: Option<&str>,
        group: Option<&str>,
    ) -> Result<Project> {
        let group = super::normalize_resource_group(group)?;
        Self::require_admin(caller)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "project name cannot be empty".into(),
            ));
        }
        let proj = self
            .projects
            .create_project(name, health_check_path, group)
            .await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "project.create",
            serde_json::json!({ "id": proj.id, "name": proj.name }),
        );
        Ok(proj)
    }

    pub async fn get_project(&self, caller: &Caller, id_or_name: &str) -> Result<Project> {
        let proj = self.get_project_internal(id_or_name).await?;
        Self::require_project_read(caller, &proj.id)?;
        Ok(proj)
    }

    pub async fn list_projects(&self, caller: &Caller) -> Result<Vec<Project>> {
        let all = self.projects.list_projects().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all.into_iter().filter(|p| p.id == *pid).collect()),
        }
    }

    pub async fn delete_project(&self, caller: &Caller, id_or_name: &str) -> Result<()> {
        Self::require_admin(caller)?;
        let proj = self.get_project_internal(id_or_name).await?;
        self.secrets.delete_project_secrets(&proj.id).await?;
        self.projects.delete_project(&proj.id).await?;
        self.runtime.delete_project_artifacts(&proj.id).await?;
        self.refresh_routes().await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "project.delete",
            serde_json::json!({ "id": proj.id, "name": proj.name }),
        );
        Ok(())
    }

    pub async fn create_deployment(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        input: CreateDeploymentInput,
    ) -> Result<Deployment> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;

        // Validate that caller is authorized to bind all referenced resources.
        if let Some(bucket) = input.assets_bucket.as_deref() {
            let meta = self
                .objects
                .get_bucket(bucket)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("bucket '{bucket}'")))?;
            if let Some(owner) = meta.project_id.as_deref() {
                if owner != proj.id {
                    return Err(FerroError::Forbidden(format!(
                        "bucket '{bucket}' belongs to a different app"
                    )));
                }
            }
            Self::require_resource_read(
                caller,
                meta.project_id.as_deref(),
                &format!("bucket '{bucket}'"),
            )?;
        }
        for bucket in input.bindings.bucket.values() {
            let meta = self
                .objects
                .get_bucket(bucket)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("bucket '{bucket}'")))?;
            Self::require_resource_write(
                &Caller::project("deployment", proj.id.clone()),
                meta.project_id.as_deref(),
                &format!("bucket '{bucket}'"),
            )?;
        }
        for ns in input.bindings.kv.values() {
            let meta = self
                .registry
                .get_kv_namespace(ns)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("kv namespace '{ns}'")))?;
            Self::require_resource_write(
                &Caller::project("deployment", proj.id.clone()),
                meta.project_id.as_deref(),
                &format!("kv namespace '{ns}'"),
            )?;
        }
        for db_ref in input.bindings.db.values() {
            let meta = self
                .registry
                .get_user_db(db_ref)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("user db '{db_ref}'")))?;
            Self::require_resource_write(
                &Caller::project("deployment", proj.id.clone()),
                meta.project_id.as_deref(),
                &format!("database '{db_ref}'"),
            )?;
        }
        for q_name in input.bindings.queue.values() {
            let cfg = self
                .queue
                .get_queue(q_name)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("queue '{q_name}'")))?;
            Self::require_resource_write(
                &Caller::project("deployment", proj.id.clone()),
                cfg.project_id.as_deref(),
                &format!("queue '{q_name}'"),
            )?;
        }
        for s_ref in input.bindings.search.values() {
            let meta = self
                .search
                .get_index(s_ref)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("search index '{s_ref}'")))?;
            Self::require_resource_write(
                &Caller::project("deployment", proj.id.clone()),
                meta.project_id.as_deref(),
                &format!("search index '{s_ref}'"),
            )?;
        }

        let mut artifact_path = input.artifact_path;
        if let Some(bytes) = input.artifact_bytes {
            let rel = artifact_path.as_deref().unwrap_or("worker.cwasm");
            let (stored_path, _precompiled) =
                self.runtime.store_artifact(&proj.id, rel, bytes).await?;
            artifact_path = Some(stored_path);
        }

        let status = match input.target_env {
            Some(TargetEnv::Prod) => "active",
            Some(TargetEnv::Preview) => "preview",
            None => "ready",
        };

        let dep = Deployment {
            id: format!("dep_{}", Uuid::new_v4().simple()),
            project_id: proj.id.clone(),
            artifact_path,
            assets_bucket: input.assets_bucket,
            assets_prefix: input.assets_prefix,
            spa_fallback: input.spa_fallback,
            bindings: input.bindings,
            crons: input.crons,
            status: status.to_string(),
            created_at: Utc::now().timestamp(),
        };

        if let Some(ref rel) = dep.artifact_path {
            let clean = rel.trim_start_matches('/');
            if clean.is_empty() {
                return Err(FerroError::InvalidArgument(
                    "artifact path cannot be empty".into(),
                ));
            }
            self.runtime.preload_deployment(&dep).await?;
        }

        let saved = self
            .projects
            .create_deployment(dep, input.target_env)
            .await?;
        if input.target_env.is_some() {
            self.refresh_routes().await?;
        }
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "deployment.create",
            serde_json::json!({
                "deployment_id": saved.id,
                "target_env": input.target_env.map(|e| e.as_str()),
            }),
        );
        Ok(saved)
    }

    pub async fn create_deployment_with_env(
        &self,
        caller: &Caller,
        runtime_id: &str,
        base_deployment_id: &str,
        env: HashMap<String, String>,
    ) -> Result<Deployment> {
        let runtime = self.get_project_internal(runtime_id).await?;
        Self::require_project_write(caller, &runtime.id)?;
        let deployment = self
            .projects
            .get_deployment(base_deployment_id)
            .await?
            .filter(|deployment| deployment.project_id == runtime.id)
            .ok_or_else(|| FerroError::NotFound(format!("deployment '{base_deployment_id}'")))?;
        let mut bindings = deployment.bindings;
        bindings.env = env;
        self.create_deployment(
            caller,
            &runtime.id,
            CreateDeploymentInput {
                artifact_path: deployment.artifact_path,
                assets_bucket: deployment.assets_bucket,
                assets_prefix: deployment.assets_prefix,
                spa_fallback: deployment.spa_fallback,
                bindings,
                crons: deployment.crons,
                ..Default::default()
            },
        )
        .await
    }

    pub async fn list_deployments(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
    ) -> Result<Vec<Deployment>> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_read(caller, &proj.id)?;
        self.projects.list_deployments(&proj.id).await
    }

    pub async fn promote_deployment(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        deployment_id: Option<&str>,
    ) -> Result<Deployment> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;

        let target_id = match deployment_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => id.to_string(),
            None => proj.preview_deployment_id.clone().ok_or_else(|| {
                FerroError::InvalidArgument("project has no preview deployment to promote".into())
            })?,
        };

        let dep = self
            .projects
            .get_deployment(&target_id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("deployment '{target_id}'")))?;
        if dep.project_id != proj.id {
            return Err(FerroError::Forbidden(format!(
                "deployment '{target_id}' does not belong to project '{}'",
                proj.id
            )));
        }

        if dep.artifact_path.is_some() {
            self.runtime.preload_deployment(&dep).await?;
        }
        self.projects
            .set_active_deployment(&proj.id, &dep.id)
            .await?;
        self.refresh_routes().await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "deployment.promote",
            serde_json::json!({ "deployment_id": dep.id }),
        );
        Ok(dep)
    }

    pub async fn rollback_deployment(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        deployment_id: Option<&str>,
    ) -> Result<Deployment> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;

        let dep = match deployment_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => {
                let d = self
                    .projects
                    .get_deployment(id)
                    .await?
                    .ok_or_else(|| FerroError::NotFound(format!("deployment '{id}'")))?;
                if d.project_id != proj.id {
                    return Err(FerroError::Forbidden(format!(
                        "deployment '{id}' does not belong to project '{}'",
                        proj.id
                    )));
                }
                d
            }
            None => self
                .projects
                .previous_production_deployment(&proj.id)
                .await?
                .ok_or_else(|| {
                    FerroError::InvalidArgument("no previous deployment to roll back to".into())
                })?,
        };

        if dep.artifact_path.is_some() {
            self.runtime.preload_deployment(&dep).await?;
        }
        self.projects
            .set_active_deployment(&proj.id, &dep.id)
            .await?;
        self.refresh_routes().await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "deployment.rollback",
            serde_json::json!({ "deployment_id": dep.id }),
        );
        Ok(dep)
    }

    pub async fn upsert_route(
        &self,
        caller: &Caller,
        host: &str,
        project_id_or_name: &str,
        target_env: TargetEnv,
    ) -> Result<Route> {
        let norm_host = normalize_host(host);
        if norm_host.is_empty()
            || norm_host.chars().any(|c| {
                c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@' | '?' | '#')
            })
        {
            return Err(FerroError::InvalidArgument(format!(
                "invalid route host '{host}'"
            )));
        }
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;

        let route = Route {
            host: norm_host,
            project_id: proj.id.clone(),
            target_env,
        };
        self.projects
            .upsert_route(route.clone(), caller.project_id())
            .await?;
        self.refresh_routes().await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "route.upsert",
            serde_json::json!({ "host": route.host, "target_env": target_env.as_str() }),
        );
        Ok(route)
    }

    pub async fn delete_route(&self, caller: &Caller, host: &str) -> Result<()> {
        let norm_host = normalize_host(host);
        if norm_host.is_empty() {
            return Err(FerroError::InvalidArgument("host cannot be empty".into()));
        }
        let existing = self
            .projects
            .get_route(&norm_host)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("route '{norm_host}'")))?;
        Self::require_project_write(caller, &existing.project_id)?;

        let deleted = self
            .projects
            .delete_route(&norm_host, caller.project_id())
            .await?;
        if !deleted {
            return Err(FerroError::NotFound(format!("route '{norm_host}'")));
        }
        self.refresh_routes().await?;
        self.audit(
            Some(&existing.project_id),
            &caller.actor,
            "route.delete",
            serde_json::json!({ "host": norm_host }),
        );
        Ok(())
    }

    pub async fn list_routes(&self, caller: &Caller) -> Result<Vec<Route>> {
        let all = self.projects.list_routes().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => {
                Ok(all.into_iter().filter(|r| r.project_id == *pid).collect())
            }
        }
    }

    pub async fn upload_project_artifact(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        rel_path: &str,
        bytes: Bytes,
    ) -> Result<(String, bool, usize)> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        let size = bytes.len();
        let (stored_path, precompiled) = self
            .runtime
            .store_artifact(&proj.id, rel_path, bytes)
            .await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "artifact.upload",
            serde_json::json!({
                "artifact_path": stored_path,
                "precompiled": precompiled,
                "size": size,
            }),
        );
        Ok((stored_path, precompiled, size))
    }

    pub async fn invoke_worker_http(
        &self,
        ctx: RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse> {
        let project_id = ctx.project_id.clone();
        let request_id = ctx.request_id.clone();
        match self.runtime.handle_http(ctx, req).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                self.logs.emit(LogEntry {
                    id: None,
                    ts: Utc::now().timestamp_millis(),
                    project_id: Some(project_id),
                    request_id: Some(request_id),
                    source: "runtime".into(),
                    level: "error".into(),
                    message: format!("Worker HTTP invocation error: {err}"),
                    meta_json: None,
                });
                Err(err)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke_project_http(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        target_env: Option<TargetEnv>,
        method: &str,
        path: &str,
        headers: Vec<(String, Vec<u8>)>,
        body: Bytes,
    ) -> Result<(String, WorkerHttpResponse)> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_read(caller, &proj.id)?;

        let dep_id = match target_env.unwrap_or(TargetEnv::Prod) {
            TargetEnv::Prod => proj.active_deployment_id.clone(),
            TargetEnv::Preview => proj.preview_deployment_id.clone(),
        }
        .ok_or_else(|| {
            FerroError::NotFound(format!(
                "project '{}' has no deployment for target env",
                proj.name
            ))
        })?;

        let dep = self
            .projects
            .get_deployment(&dep_id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("deployment '{dep_id}'")))?;

        let norm_path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        let path_only = norm_path.split('?').next().unwrap_or("/");
        if crate::gateway::local_api::has_unsafe_path_segments(path_only) {
            return Err(FerroError::InvalidArgument("invalid request path".into()));
        }

        let method_upper = method.trim().to_ascii_uppercase();
        if let Some(bucket) = dep.assets_bucket.as_deref() {
            if method_upper == "GET" || method_upper == "HEAD" {
                let path_only = norm_path.split('?').next().unwrap_or("/");
                let clean = path_only.trim_start_matches('/');
                let rel = if clean.is_empty() {
                    "index.html".to_string()
                } else if clean.ends_with('/') {
                    format!("{clean}index.html")
                } else {
                    clean.to_string()
                };
                let key = match dep
                    .assets_prefix
                    .as_deref()
                    .map(|p| p.trim_matches('/'))
                    .filter(|p| !p.is_empty())
                {
                    Some(pfx) => format!("{pfx}/{rel}"),
                    None => rel,
                };
                if let Ok(Some((meta, stream, _, _))) =
                    self.static_get_object_stream(bucket, &key, None).await
                {
                    let body_stream = if method_upper == "HEAD" {
                        drop(stream);
                        Box::pin(futures_util::stream::empty())
                    } else {
                        let bytes = stream
                            .try_fold(Vec::new(), |mut acc, chunk| async move {
                                acc.extend_from_slice(&chunk);
                                Ok(acc)
                            })
                            .await
                            .map_err(|e| {
                                FerroError::Internal(anyhow::anyhow!("read static asset: {e}"))
                            })?;
                        Box::pin(futures_util::stream::once(async move {
                            Ok::<_, std::io::Error>(Bytes::from(bytes))
                        })) as crate::core::model::ByteStream
                    };
                    return Ok((
                        dep.id.clone(),
                        WorkerHttpResponse {
                            status: 200,
                            headers: vec![("content-type".into(), meta.content_type.into_bytes())],
                            body: body_stream,
                        },
                    ));
                }
            }
        }

        if dep.artifact_path.is_none() {
            return Err(FerroError::NotFound(format!(
                "deployment '{}' has no worker artifact",
                dep.id
            )));
        }

        let request_id = format!("inv_{}", Uuid::new_v4().simple());
        let ctx = RequestCtx::new(proj.id.clone(), request_id, dep.clone());
        let req = WorkerHttpRequest {
            method: method_upper,
            uri: norm_path,
            headers,
            body: Box::pin(futures_util::stream::once(async move {
                Ok::<_, std::io::Error>(body)
            })),
        };
        let resp = self.invoke_worker_http(ctx, req).await?;
        Ok((dep.id, resp))
    }
}
