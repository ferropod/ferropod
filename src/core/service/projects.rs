use std::collections::HashMap;

use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use uuid::Uuid;

use crate::core::model::{
    Bindings, Caller, Deployment, FerroError, LogEntry, Project, ProjectOpts, RequestCtx, Result,
    Route, RouteEntry, RouteOpts, TargetEnv, TokenScope, WorkerHttpRequest, WorkerHttpResponse,
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
        self.create_project_with_opts(
            caller,
            name,
            ProjectOpts {
                group: group.map(str::to_owned),
                health_check_path: health_check_path.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn create_project_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        mut opts: ProjectOpts,
    ) -> Result<Project> {
        opts.group = super::normalize_resource_group(opts.group.as_deref())?.map(str::to_owned);
        Self::require_admin(caller)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "project name cannot be empty".into(),
            ));
        }
        if let Some(mc) = opts.max_concurrency {
            if mc < 1 {
                return Err(FerroError::InvalidArgument(
                    "max_concurrency must be at least 1".into(),
                ));
            }
        }
        if let Some(t) = opts.timeout_ms {
            if t < 50 {
                return Err(FerroError::InvalidArgument(
                    "timeout_ms must be at least 50ms".into(),
                ));
            }
        }
        if let Some(mem) = opts.wasm_max_memory_mb {
            if mem < 1 {
                return Err(FerroError::InvalidArgument(
                    "wasm_max_memory_mb must be at least 1 MiB".into(),
                ));
            }
        }
        let proj = self.projects.create_project_with_opts(name, opts).await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "project.create",
            serde_json::json!({ "id": proj.id, "name": proj.name }),
        );
        Ok(proj)
    }

    pub async fn update_project(
        &self,
        caller: &Caller,
        id_or_name: &str,
        opts: ProjectOpts,
    ) -> Result<Project> {
        let mut proj = self.get_project_internal(id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        if let Some(g) = opts.group {
            proj.group = super::normalize_resource_group(Some(&g))?.map(str::to_owned);
        }
        if let Some(desc) = opts.description {
            let trimmed = desc.trim();
            proj.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(hcp) = opts.health_check_path {
            let trimmed = hcp.trim();
            if !trimmed.is_empty() {
                proj.health_check_path = if trimmed.starts_with('/') {
                    trimmed.to_string()
                } else {
                    format!("/{trimmed}")
                };
            }
        }
        if let Some(mc) = opts.max_concurrency {
            if mc < 1 {
                return Err(FerroError::InvalidArgument(
                    "max_concurrency must be at least 1".into(),
                ));
            }
            proj.max_concurrency = mc;
        }
        if let Some(t) = opts.timeout_ms {
            proj.timeout_ms = if t == 0 { None } else { Some(t.max(50)) };
        }
        if let Some(mem) = opts.wasm_max_memory_mb {
            proj.wasm_max_memory_mb = if mem == 0 { None } else { Some(mem) };
        }
        if let Some(allow) = opts.allow_private_outbound {
            proj.allow_private_outbound = allow;
        }
        proj.updated_at = Utc::now().timestamp();
        self.projects.update_project(&proj).await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "project.update",
            serde_json::json!({
                "id": proj.id,
                "max_concurrency": proj.max_concurrency,
                "timeout_ms": proj.timeout_ms,
                "wasm_max_memory_mb": proj.wasm_max_memory_mb,
                "allow_private_outbound": proj.allow_private_outbound,
            }),
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
        self.notify_tls_reload();
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
        self.promote_deployment_with_health(caller, project_id_or_name, deployment_id, false)
            .await
    }

    pub async fn promote_deployment_with_health(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        deployment_id: Option<&str>,
        verify_health: bool,
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
            if verify_health {
                let hc_path = if proj.health_check_path.starts_with('/') {
                    proj.health_check_path.clone()
                } else {
                    format!("/{}", proj.health_check_path)
                };
                let probe_ctx = RequestCtx::new(
                    proj.id.clone(),
                    format!("hc_{}", Uuid::new_v4().simple()),
                    dep.clone(),
                )
                .with_project_limits(&proj);
                let probe_req = WorkerHttpRequest {
                    method: "GET".to_string(),
                    uri: hc_path.clone(),
                    headers: Vec::new(),
                    body: Box::pin(futures_util::stream::empty()),
                };
                let probe_resp = self
                    .invoke_worker_http(probe_ctx, probe_req)
                    .await
                    .map_err(|e| {
                        FerroError::InvalidArgument(format!("health check '{hc_path}' failed: {e}"))
                    })?;
                if probe_resp.status >= 500 {
                    return Err(FerroError::InvalidArgument(format!(
                        "health check '{hc_path}' returned HTTP {}",
                        probe_resp.status
                    )));
                }
            }
        }
        self.projects
            .set_active_deployment(&proj.id, &dep.id)
            .await?;
        self.health_tracker.mark_promoted(&dep.id);
        self.refresh_routes().await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "deployment.promote",
            serde_json::json!({ "deployment_id": dep.id, "verify_health": verify_health }),
        );
        Ok(dep)
    }

    /// Automatically revert a newly promoted production deployment if a 5xx error spike occurs.
    pub async fn auto_rollback_deployment(
        &self,
        project_id: &str,
        failed_dep_id: &str,
    ) -> Result<Option<Deployment>> {
        let Some(proj) = self.projects.get_project(project_id).await? else {
            return Ok(None);
        };
        if proj.active_deployment_id.as_deref() != Some(failed_dep_id) {
            return Ok(None);
        }
        let Some(prev) = self
            .projects
            .previous_production_deployment(&proj.id)
            .await?
        else {
            return Ok(None);
        };
        if prev.artifact_path.is_some() {
            let _ = self.runtime.preload_deployment(&prev).await;
        }
        self.projects
            .set_active_deployment(&proj.id, &prev.id)
            .await?;
        self.health_tracker.mark_promoted(&prev.id);
        self.refresh_routes().await?;
        self.logs.emit(LogEntry {
            id: None,
            ts: Utc::now().timestamp_millis(),
            project_id: Some(proj.id.clone()),
            request_id: None,
            source: "system".into(),
            level: "warn".into(),
            message: format!(
                "Auto-rolled back app '{}' from {} to {} due to 5xx spike",
                proj.name, failed_dep_id, prev.id
            ),
            meta_json: None,
        });
        self.audit(
            Some(&proj.id),
            "auto-rollback",
            "deployment.auto_rollback",
            serde_json::json!({ "failed_deployment_id": failed_dep_id, "restored_deployment_id": prev.id }),
        );
        self.fire_alert(
            "auto_rollback",
            format!("Auto-rolled back app '{}' to {}", proj.name, prev.id),
            serde_json::json!({
                "project_id": proj.id,
                "project_name": proj.name,
                "failed_deployment_id": failed_dep_id,
                "restored_deployment_id": prev.id,
            }),
        );
        Ok(Some(prev))
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

    async fn apply_route_opts(
        &self,
        proj: &Project,
        mut route: Route,
        opts: RouteOpts,
    ) -> Result<Route> {
        if let Some(upstream_raw) = opts.proxy_upstream {
            let trimmed = upstream_raw.trim().trim_end_matches('/').to_string();
            if trimmed.is_empty() {
                route.proxy_upstream = None;
                route.proxy_prefix = None;
            } else {
                if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
                    return Err(FerroError::InvalidArgument(
                        "proxy_upstream must start with http:// or https://".into(),
                    ));
                }
                let parsed: http::Uri = trimmed.parse().map_err(|e| {
                    FerroError::InvalidArgument(format!("invalid proxy_upstream: {e}"))
                })?;
                if parsed.authority().is_none() {
                    return Err(FerroError::InvalidArgument(
                        "proxy_upstream must include a host authority".into(),
                    ));
                }
                route.proxy_upstream = Some(trimmed);
                if route.proxy_prefix.is_none() {
                    route.proxy_prefix = Some("/api".to_string());
                }
            }
        }
        if let Some(prefix_raw) = opts.proxy_prefix {
            let trimmed = prefix_raw.trim().trim_end_matches('/').to_string();
            if trimmed.is_empty() {
                route.proxy_prefix = None;
            } else {
                if !trimmed.starts_with('/') || trimmed.len() <= 1 {
                    return Err(FerroError::InvalidArgument(
                        "proxy_prefix must be a non-root path starting with '/'".into(),
                    ));
                }
                let lower = trimmed.to_ascii_lowercase();
                if crate::config::RESERVED_LOCAL_API_PREFIXES
                    .iter()
                    .any(|r| lower == *r || lower.starts_with(&format!("{r}/")))
                    || lower == "/_mcp"
                    || lower == "/_ai"
                    || lower.starts_with("/_ai/")
                    || lower == "/_realtime"
                    || lower.starts_with("/_realtime/")
                {
                    return Err(FerroError::InvalidArgument(
                        "proxy_prefix cannot shadow reserved FerroPod routes".into(),
                    ));
                }
                route.proxy_prefix = Some(trimmed);
            }
        }
        if let Some(rpm) = opts.rate_limit_rpm {
            route.rate_limit_rpm = if rpm == 0 { None } else { Some(rpm) };
        }
        if opts.clear_basic_auth == Some(true) {
            route.basic_auth_user = None;
            route.basic_auth_pass_hash = None;
            route.basic_auth_enabled = false;
        } else if let (Some(user), Some(pass)) = (
            opts.basic_auth_user
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            opts.basic_auth_password
                .as_deref()
                .filter(|s| !s.is_empty()),
        ) {
            route.basic_auth_user = Some(user.to_string());
            route.basic_auth_pass_hash = Some(crate::gateway::guard::hash_basic_auth(user, pass));
            route.basic_auth_enabled = true;
        }
        if let Some(w) = opts.canary_weight {
            if w > 100 {
                return Err(FerroError::InvalidArgument(
                    "canary_weight must be between 0 and 100".into(),
                ));
            }
            route.canary_weight = w;
        }
        if let Some(cid_raw) = opts.canary_deployment_id {
            let trimmed = cid_raw.trim().to_string();
            if trimmed.is_empty() {
                route.canary_deployment_id = None;
                route.canary_weight = 0;
            } else {
                let dep = self
                    .projects
                    .get_deployment(&trimmed)
                    .await?
                    .ok_or_else(|| FerroError::NotFound(format!("deployment '{trimmed}'")))?;
                if dep.project_id != proj.id {
                    return Err(FerroError::Forbidden(format!(
                        "canary deployment '{trimmed}' belongs to a different app"
                    )));
                }
                route.canary_deployment_id = Some(dep.id);
            }
        }
        Ok(route)
    }

    pub async fn upsert_route(
        &self,
        caller: &Caller,
        host: &str,
        project_id_or_name: &str,
        target_env: TargetEnv,
    ) -> Result<Route> {
        self.upsert_route_with_opts(
            caller,
            host,
            project_id_or_name,
            target_env,
            RouteOpts::default(),
        )
        .await
    }

    pub async fn upsert_route_with_opts(
        &self,
        caller: &Caller,
        host: &str,
        project_id_or_name: &str,
        target_env: TargetEnv,
        opts: RouteOpts,
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

        let base = Route {
            host: norm_host,
            project_id: proj.id.clone(),
            target_env,
            ..Default::default()
        };
        let route = self.apply_route_opts(&proj, base, opts).await?;
        self.projects
            .upsert_route(route.clone(), caller.project_id())
            .await?;
        self.refresh_routes().await?;
        self.notify_tls_reload();
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "route.upsert",
            serde_json::json!({ "host": route.host, "target_env": target_env.as_str() }),
        );
        Ok(route)
    }

    pub async fn update_route_opts(
        &self,
        caller: &Caller,
        host: &str,
        project_id_or_name: Option<&str>,
        target_env: Option<TargetEnv>,
        opts: RouteOpts,
    ) -> Result<Route> {
        let norm_host = normalize_host(host);
        if norm_host.is_empty() {
            return Err(FerroError::InvalidArgument("host cannot be empty".into()));
        }
        let mut existing = self
            .projects
            .get_route(&norm_host)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("route '{norm_host}'")))?;
        Self::require_project_write(caller, &existing.project_id)?;

        let proj = if let Some(pid_or_name) = project_id_or_name {
            let p = self.get_project_internal(pid_or_name).await?;
            Self::require_project_write(caller, &p.id)?;
            existing.project_id = p.id.clone();
            p
        } else {
            self.get_project_internal(&existing.project_id).await?
        };
        if let Some(env) = target_env {
            existing.target_env = env;
        }
        let route = self.apply_route_opts(&proj, existing, opts).await?;
        self.projects
            .upsert_route(route.clone(), caller.project_id())
            .await?;
        self.refresh_routes().await?;
        self.notify_tls_reload();
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "route.update",
            serde_json::json!({
                "host": route.host,
                "target_env": route.target_env.as_str(),
                "rate_limit_rpm": route.rate_limit_rpm,
                "canary_weight": route.canary_weight,
            }),
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
        self.notify_tls_reload();
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
        mut ctx: RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse> {
        let project_id = ctx.project_id.clone();
        let request_id = ctx.request_id.clone();
        let _app_permit = self.try_acquire_app_permit(&project_id).ok_or_else(|| {
            self.metrics
                .rate_limited_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            FerroError::RateLimited(format!("app '{project_id}' concurrency limit reached"))
        })?;
        if ctx.timeout_ms.is_none()
            && ctx.wasm_max_memory_mb.is_none()
            && !ctx.allow_private_outbound
        {
            if let Ok(Some(proj)) = self.projects.get_project(&project_id).await {
                ctx = ctx.with_project_limits(&proj);
            }
        }
        match self.runtime.handle_http(ctx, req).await {
            Ok(resp) => Ok(resp),
            Err(err) => {
                self.metrics
                    .worker_errors_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let msg = err.to_string();
                let lower = msg.to_ascii_lowercase();
                if lower.contains("oom")
                    || lower.contains("memory")
                    || lower.contains("allocation")
                    || lower.contains("resource limit")
                {
                    self.metrics
                        .worker_oom_total
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    self.fire_alert(
                        "worker_oom",
                        format!("Worker OOM in app '{project_id}'"),
                        serde_json::json!({
                            "project_id": project_id,
                            "request_id": request_id,
                            "error": msg,
                        }),
                    );
                }
                self.logs.emit(LogEntry {
                    id: None,
                    ts: Utc::now().timestamp_millis(),
                    project_id: Some(project_id),
                    request_id: Some(request_id),
                    source: "runtime".into(),
                    level: "error".into(),
                    message: format!("Worker HTTP invocation error: {msg}"),
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
        let ctx =
            RequestCtx::new(proj.id.clone(), request_id, dep.clone()).with_project_limits(&proj);
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
