use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::core::model::{
    Bindings, Caller, Deployment, FerroError, Project, Result, Route, RouteEntry, TargetEnv,
    TokenScope,
};

use super::PlatformService;

/// Input payload for creating a new immutable deployment version.
#[derive(Debug, Clone, Default)]
pub struct CreateDeploymentInput {
    pub artifact_path: Option<String>,
    pub assets_bucket: Option<String>,
    pub assets_prefix: Option<String>,
    pub spa_fallback: bool,
    pub bindings: Bindings,
    pub crons: Vec<String>,
    pub target_env: Option<TargetEnv>,
}

fn normalize_host(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_port = match trimmed.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') && port.chars().all(|c| c.is_ascii_digit()) => {
            host
        }
        _ => trimmed,
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

    /// Check whether a domain is present in the active routing table (used by `/_internal/tls/allow`).
    pub async fn is_domain_allowed(&self, domain: &str) -> bool {
        let key = normalize_host(domain);
        if key.is_empty() {
            return false;
        }
        self.routes.read().await.contains_key(&key)
    }

    pub async fn create_project(
        &self,
        caller: &Caller,
        name: &str,
        health_check_path: Option<&str>,
    ) -> Result<Project> {
        Self::require_admin(caller)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "project name cannot be empty".into(),
            ));
        }
        let proj = self.projects.create_project(name, health_check_path).await?;
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
                caller,
                meta.project_id.as_deref(),
                &format!("bucket '{bucket}'"),
            )?;
        }
        for ns in input.bindings.kv.values() {
            let meta = self.registry.get_kv_namespace(ns).await?;
            let owner = meta.as_ref().and_then(|m| m.project_id.as_deref());
            Self::require_resource_write(caller, owner, &format!("kv namespace '{ns}'"))?;
        }
        for db_ref in input.bindings.db.values() {
            let meta = self
                .registry
                .get_user_db(db_ref)
                .await?
                .ok_or_else(|| FerroError::NotFound(format!("user db '{db_ref}'")))?;
            Self::require_resource_write(
                caller,
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
                caller,
                cfg.project_id.as_deref(),
                &format!("queue '{q_name}'"),
            )?;
        }

        let status = match input.target_env {
            Some(TargetEnv::Prod) => "active",
            Some(TargetEnv::Preview) => "preview",
            None => "ready",
        };

        let dep = Deployment {
            id: format!("dep_{}", Uuid::new_v4().simple()),
            project_id: proj.id.clone(),
            artifact_path: input.artifact_path,
            assets_bucket: input.assets_bucket,
            assets_prefix: input.assets_prefix,
            spa_fallback: input.spa_fallback,
            bindings: input.bindings,
            crons: input.crons,
            status: status.to_string(),
            created_at: Utc::now().timestamp(),
        };

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
            None => {
                let deps = self.projects.list_deployments(&proj.id).await?;
                deps.into_iter()
                    .find(|d| Some(d.id.as_str()) != proj.active_deployment_id.as_deref())
                    .ok_or_else(|| {
                        FerroError::InvalidArgument("no previous deployment to roll back to".into())
                    })?
            }
        };

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
        if norm_host.is_empty() || norm_host.contains('/') || norm_host.contains(' ') {
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
            TokenScope::Project(pid) => Ok(all.into_iter().filter(|r| r.project_id == *pid).collect()),
        }
    }
}
