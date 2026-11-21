use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::Utc;
use turso::{transaction::TransactionBehavior, Value as TursoValue};
use uuid::Uuid;

use crate::core::model::{
    Deployment, FerroError, Project, Result, Route, RouteEntry, TargetEnv,
};
use crate::core::traits::ProjectStore;
use crate::storage::tx::{
    checkpoint_db, connect_db, copy_db_with_sidecars, map_turso_err, with_retry,
};

use super::{opt_text, opt_text_param, row_to_deployment, row_to_project, TursoCoreStore};

#[async_trait]
impl ProjectStore for TursoCoreStore {
    async fn create_project(
        &self,
        name: &str,
        health_check_path: Option<&str>,
        group: Option<&str>,
    ) -> Result<Project> {
        let now = Utc::now().timestamp();
        let id = format!("prj_{}", Uuid::new_v4().simple());
        let hcp = health_check_path.unwrap_or("/health").to_string();
        let name_s = name.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id = id.clone();
            let name_s = name_s.clone();
            let hcp = hcp.clone();
            let group = group.map(str::to_owned);
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO projects (id, name, max_concurrency, health_check_path, created_at, updated_at, resource_group)
                     VALUES (?1, ?2, 64, ?3, ?4, ?4, ?5)",
                    (id, name_s, hcp, now, group),
                )
                .await
            }
        })
        .await?;

        Ok(Project {
            id,
            name: name.to_string(),
            active_deployment_id: None,
            preview_deployment_id: None,
            max_concurrency: 64,
            access_policy_json: None,
            health_check_path: health_check_path.unwrap_or("/health").to_string(),
            created_at: now,
            updated_at: now,
            group: group.map(str::to_owned),
        })
    }

    async fn get_project(&self, id_or_name: &str) -> Result<Option<Project>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, active_deployment_id, preview_deployment_id, max_concurrency, access_policy_json, health_check_path, created_at, updated_at, resource_group
                 FROM projects WHERE id = ?1 OR name = ?1 LIMIT 1",
                [id_or_name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_project(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, active_deployment_id, preview_deployment_id, max_concurrency, access_policy_json, health_check_path, created_at, updated_at, resource_group
                 FROM projects ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_project(&row)?);
        }
        Ok(out)
    }

    async fn delete_project(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                tx.execute("DELETE FROM production_history WHERE project_id = ?1", [id_s.as_str()]).await?;
                tx.execute("DELETE FROM routes WHERE project_id = ?1", [id_s.as_str()])
                    .await?;
                tx.execute("DELETE FROM secrets WHERE project_id = ?1", [id_s.as_str()])
                    .await?;
                tx.execute("DELETE FROM deployments WHERE project_id = ?1", [id_s.as_str()])
                    .await?;
                tx.execute("DELETE FROM projects WHERE id = ?1", [id_s.as_str()])
                    .await?;
                tx.commit().await
            }
        })
        .await
    }

    async fn create_deployment(
        &self,
        deployment: Deployment,
        target_env: Option<TargetEnv>,
    ) -> Result<Deployment> {
        let bindings_json = serde_json::to_string(&deployment.bindings)
            .map_err(|e| FerroError::Internal(anyhow!("serialize bindings: {e}")))?;
        let crons_json = serde_json::to_string(&deployment.crons)
            .map_err(|e| FerroError::Internal(anyhow!("serialize crons: {e}")))?;
        let dep_c = deployment.clone();
        with_retry(|| {
            let db = self.db.clone();
            let dep = dep_c.clone();
            let bindings_json = bindings_json.clone();
            let crons_json = crons_json.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;

                match target_env {
                    Some(TargetEnv::Prod) => {
                        tx.execute(
                            "INSERT INTO production_history (project_id, deployment_id)
                             SELECT id, active_deployment_id FROM projects
                             WHERE id = ?1 AND active_deployment_id IS NOT NULL",
                            [dep.project_id.as_str()],
                        ).await?;
                        tx.execute(
                            "UPDATE deployments SET status = 'ready' WHERE project_id = ?1 AND status = 'active'",
                            [dep.project_id.as_str()],
                        )
                        .await?;
                    }
                    Some(TargetEnv::Preview) => {
                        tx.execute(
                            "UPDATE deployments SET status = 'ready' WHERE project_id = ?1 AND status = 'preview'",
                            [dep.project_id.as_str()],
                        )
                        .await?;
                    }
                    None => {}
                }

                tx.execute(
                    "INSERT INTO deployments (id, project_id, artifact_path, assets_bucket, assets_prefix, spa_fallback, bindings_json, crons_json, status, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    vec![
                        TursoValue::Text(dep.id.clone()),
                        TursoValue::Text(dep.project_id.clone()),
                        opt_text_param(dep.artifact_path.clone()),
                        opt_text_param(dep.assets_bucket.clone()),
                        opt_text_param(dep.assets_prefix.clone()),
                        TursoValue::Integer(if dep.spa_fallback { 1 } else { 0 }),
                        TursoValue::Text(bindings_json),
                        TursoValue::Text(crons_json),
                        TursoValue::Text(dep.status.clone()),
                        TursoValue::Integer(dep.created_at),
                    ],
                )
                .await?;

                match target_env {
                    Some(TargetEnv::Prod) => {
                        let affected = tx
                            .execute(
                                "UPDATE projects SET active_deployment_id = ?1, updated_at = ?2 WHERE id = ?3",
                                (dep.id.as_str(), dep.created_at, dep.project_id.as_str()),
                            )
                            .await?;
                        if affected == 0 {
                            let _ = tx.rollback().await;
                            return Ok(Err(FerroError::NotFound(format!(
                                "project '{}'",
                                dep.project_id
                            ))));
                        }
                    }
                    Some(TargetEnv::Preview) => {
                        let affected = tx
                            .execute(
                                "UPDATE projects SET preview_deployment_id = ?1, updated_at = ?2 WHERE id = ?3",
                                (dep.id.as_str(), dep.created_at, dep.project_id.as_str()),
                            )
                            .await?;
                        if affected == 0 {
                            let _ = tx.rollback().await;
                            return Ok(Err(FerroError::NotFound(format!(
                                "project '{}'",
                                dep.project_id
                            ))));
                        }
                    }
                    None => {}
                }

                tx.commit().await?;
                Ok(Ok(dep))
            }
        })
        .await?
    }

    async fn get_deployment(&self, id: &str) -> Result<Option<Deployment>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, project_id, artifact_path, assets_bucket, assets_prefix, spa_fallback, bindings_json, crons_json, status, created_at
                 FROM deployments WHERE id = ?1",
                [id],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(row_to_deployment(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_deployments(&self, project_id: &str) -> Result<Vec<Deployment>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, project_id, artifact_path, assets_bucket, assets_prefix, spa_fallback, bindings_json, crons_json, status, created_at
                 FROM deployments WHERE project_id = ?1 ORDER BY created_at DESC",
                [project_id],
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_deployment(&row)?);
        }
        Ok(out)
    }

    async fn previous_production_deployment(&self, project_id: &str) -> Result<Option<Deployment>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn.query(
            "SELECT d.id, d.project_id, d.artifact_path, d.assets_bucket, d.assets_prefix,
                    d.spa_fallback, d.bindings_json, d.crons_json, d.status, d.created_at
             FROM production_history h JOIN deployments d ON d.id = h.deployment_id
             JOIN projects p ON p.id = h.project_id
             WHERE h.project_id = ?1 AND h.deployment_id != p.active_deployment_id
             ORDER BY h.id DESC LIMIT 1",
            [project_id],
        ).await.map_err(map_turso_err)?;
        match rows.next().await.map_err(map_turso_err)? {
            Some(row) => Ok(Some(row_to_deployment(&row)?)),
            None => Ok(None),
        }
    }

    async fn set_active_deployment(&self, project_id: &str, deployment_id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let pid = project_id.to_string();
        let did = deployment_id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let pid = pid.clone();
            let did = did.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                tx.execute(
                    "INSERT INTO production_history (project_id, deployment_id)
                     SELECT id, active_deployment_id FROM projects
                     WHERE id = ?1 AND active_deployment_id IS NOT NULL AND active_deployment_id != ?2",
                    (pid.as_str(), did.as_str()),
                ).await?;
                tx.execute(
                    "UPDATE deployments SET status = 'ready' WHERE project_id = ?1 AND status = 'active'",
                    [pid.as_str()],
                )
                .await?;
                tx.execute(
                    "UPDATE deployments SET status = 'active' WHERE id = ?1 AND project_id = ?2",
                    (did.as_str(), pid.as_str()),
                )
                .await?;
                tx.execute(
                    "UPDATE projects SET active_deployment_id = ?1, updated_at = ?2 WHERE id = ?3",
                    (did.as_str(), now, pid.as_str()),
                )
                .await?;
                tx.commit().await
            }
        })
        .await
    }

    async fn set_preview_deployment(&self, project_id: &str, deployment_id: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        let pid = project_id.to_string();
        let did = deployment_id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let pid = pid.clone();
            let did = did.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                tx.execute(
                    "UPDATE deployments SET status = 'ready' WHERE project_id = ?1 AND status = 'preview'",
                    [pid.as_str()],
                )
                .await?;
                tx.execute(
                    "UPDATE deployments SET status = 'preview' WHERE id = ?1 AND project_id = ?2",
                    (did.as_str(), pid.as_str()),
                )
                .await?;
                tx.execute(
                    "UPDATE projects SET preview_deployment_id = ?1, updated_at = ?2 WHERE id = ?3",
                    (did.as_str(), now, pid.as_str()),
                )
                .await?;
                tx.commit().await
            }
        })
        .await
    }

    async fn upsert_route(&self, route: Route, expected_owner: Option<&str>) -> Result<()> {
        let expected_owner_s = expected_owner.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let route = route.clone();
            let expected_owner_s = expected_owner_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;

                if let Some(owner_pid) = expected_owner_s.as_deref() {
                    let mut rows = tx
                        .query(
                            "SELECT project_id FROM routes WHERE host = ?1 LIMIT 1",
                            [route.host.as_str()],
                        )
                        .await?;
                    let existing_pid = match rows.next().await? {
                        Some(r) => Some(r.get::<String>(0)?),
                        None => None,
                    };
                    drop(rows);
                    if let Some(existing) = existing_pid {
                        if existing != owner_pid {
                            let _ = tx.rollback().await;
                            return Ok(Err(FerroError::Forbidden(
                                "route host is owned by another project".into(),
                            )));
                        }
                    }
                }

                tx.execute(
                    "INSERT INTO routes (host, project_id, target_env) VALUES (?1, ?2, ?3)
                     ON CONFLICT(host) DO UPDATE SET project_id = excluded.project_id, target_env = excluded.target_env",
                    (route.host, route.project_id, route.target_env.as_str()),
                )
                .await?;
                tx.commit().await?;
                Ok(Ok(()))
            }
        })
        .await?
    }

    async fn get_route(&self, host: &str) -> Result<Option<Route>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT host, project_id, target_env FROM routes WHERE host = ?1 LIMIT 1",
                [host],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let env_str: String = row.get(2).map_err(map_turso_err)?;
            Ok(Some(Route {
                host: row.get(0).map_err(map_turso_err)?,
                project_id: row.get(1).map_err(map_turso_err)?,
                target_env: TargetEnv::parse(&env_str)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn delete_route(&self, host: &str, expected_owner: Option<&str>) -> Result<bool> {
        let host_s = host.to_string();
        let expected_owner_s = expected_owner.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let host_s = host_s.clone();
            let expected_owner_s = expected_owner_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT project_id FROM routes WHERE host = ?1 LIMIT 1",
                        [host_s.as_str()],
                    )
                    .await?;
                let existing_pid = match rows.next().await? {
                    Some(r) => Some(r.get::<String>(0)?),
                    None => None,
                };
                drop(rows);

                let Some(existing) = existing_pid else {
                    tx.commit().await?;
                    return Ok(Ok(false));
                };

                if let Some(owner_pid) = expected_owner_s.as_deref() {
                    if existing != owner_pid {
                        let _ = tx.rollback().await;
                        return Ok(Err(FerroError::Forbidden(
                            "route host is owned by another project".into(),
                        )));
                    }
                }

                let affected = tx
                    .execute("DELETE FROM routes WHERE host = ?1", [host_s.as_str()])
                    .await?;
                tx.commit().await?;
                Ok(Ok(affected > 0))
            }
        })
        .await?
    }

    async fn list_routes(&self) -> Result<Vec<Route>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query("SELECT host, project_id, target_env FROM routes ORDER BY host ASC", ())
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let env_str: String = row.get(2).map_err(map_turso_err)?;
            out.push(Route {
                host: row.get(0).map_err(map_turso_err)?,
                project_id: row.get(1).map_err(map_turso_err)?,
                target_env: TargetEnv::parse(&env_str)?,
            });
        }
        Ok(out)
    }

    async fn load_route_entries(&self) -> Result<Vec<RouteEntry>> {
        let mut conn = connect_db(&self.db)?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .await
            .map_err(map_turso_err)?;

        let mut proj_deps: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        {
            let mut rows = tx
                .query(
                    "SELECT id, active_deployment_id, preview_deployment_id FROM projects",
                    (),
                )
                .await
                .map_err(map_turso_err)?;
            while let Some(row) = rows.next().await.map_err(map_turso_err)? {
                let id: String = row.get(0).map_err(map_turso_err)?;
                let active_id = opt_text(&row, 1)?;
                let preview_id = opt_text(&row, 2)?;
                proj_deps.insert(id, (active_id, preview_id));
            }
        }

        let mut routes: Vec<Route> = Vec::new();
        {
            let mut rows = tx
                .query("SELECT host, project_id, target_env FROM routes ORDER BY host ASC", ())
                .await
                .map_err(map_turso_err)?;
            while let Some(row) = rows.next().await.map_err(map_turso_err)? {
                let env_str: String = row.get(2).map_err(map_turso_err)?;
                routes.push(Route {
                    host: row.get(0).map_err(map_turso_err)?,
                    project_id: row.get(1).map_err(map_turso_err)?,
                    target_env: TargetEnv::parse(&env_str)?,
                });
            }
        }

        let mut needed_ids: HashSet<String> = HashSet::new();
        for r in &routes {
            if let Some((active_id, preview_id)) = proj_deps.get(&r.project_id) {
                let dep_id = match r.target_env {
                    TargetEnv::Prod => active_id.as_deref(),
                    TargetEnv::Preview => preview_id.as_deref(),
                };
                if let Some(id) = dep_id {
                    needed_ids.insert(id.to_string());
                }
            }
        }

        let mut dep_map: HashMap<String, Deployment> = HashMap::with_capacity(needed_ids.len());
        let needed_vec: Vec<String> = needed_ids.into_iter().collect();
        for chunk in needed_vec.chunks(500) {
            let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT id, project_id, artifact_path, assets_bucket, assets_prefix, spa_fallback, bindings_json, crons_json, status, created_at
                 FROM deployments WHERE id IN ({})",
                placeholders.join(", ")
            );
            let params: Vec<TursoValue> =
                chunk.iter().cloned().map(TursoValue::Text).collect();
            let mut rows = tx.query(&sql, params).await.map_err(map_turso_err)?;
            while let Some(row) = rows.next().await.map_err(map_turso_err)? {
                let dep = row_to_deployment(&row)?;
                dep_map.insert(dep.id.clone(), dep);
            }
        }

        tx.commit().await.map_err(map_turso_err)?;

        let mut entries = Vec::with_capacity(routes.len());
        for r in routes {
            let active_deployment = proj_deps.get(&r.project_id).and_then(|(active, preview)| {
                let dep_id = match r.target_env {
                    TargetEnv::Prod => active.as_deref(),
                    TargetEnv::Preview => preview.as_deref(),
                };
                dep_id.and_then(|id| dep_map.get(id).cloned())
            });
            entries.push(RouteEntry {
                host: r.host.to_ascii_lowercase(),
                project_id: r.project_id,
                target_env: r.target_env,
                active_deployment,
            });
        }
        Ok(entries)
    }

    async fn checkpoint(&self) -> Result<()> {
        checkpoint_db(&self.db).await;
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        checkpoint_db(&self.db).await;
        copy_db_with_sidecars(&self.db_path, dest_path).await
    }
}
