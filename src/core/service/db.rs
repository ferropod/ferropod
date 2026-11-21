use std::path::Path;

use uuid::Uuid;

use crate::core::model::{Caller, FerroError, QueryResult, Result, SqlValue, TokenScope, UserDbMeta};

use super::PlatformService;

impl PlatformService {
    async fn resolve_user_db(&self, id_or_name: &str) -> Result<UserDbMeta> {
        self.registry
            .get_user_db(id_or_name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("user db '{id_or_name}'")))
    }

    pub async fn create_user_db(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "database name cannot be empty".into(),
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
                    "readonly token cannot create databases".into(),
                ));
            }
        };

        let id = format!("db_{}", Uuid::new_v4().simple());
        self.registry
            .register_user_db(&id, name, resolved_pid.as_deref())
            .await?;
        if let Err(e) = self.db.create_db(&id).await {
            let _ = self.registry.delete_user_db(&id).await;
            return Err(e);
        }
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "db.create",
            serde_json::json!({ "id": id, "name": name }),
        );
        Ok(id)
    }

    pub async fn list_user_dbs(&self, caller: &Caller) -> Result<Vec<UserDbMeta>> {
        let all = self.registry.list_user_dbs().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|d| d.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn delete_user_db(&self, caller: &Caller, id_or_name: &str) -> Result<()> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.delete_db(&meta.id).await?;
        self.registry.delete_user_db(&meta.id).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "db.delete",
            serde_json::json!({ "id": meta.id, "name": meta.name }),
        );
        Ok(())
    }

    pub async fn schema_user_db(&self, caller: &Caller, id_or_name: &str) -> Result<String> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.schema(&meta.id).await
    }

    pub async fn query_user_db(
        &self,
        caller: &Caller,
        id_or_name: &str,
        sql: &str,
        params: Vec<SqlValue>,
    ) -> Result<QueryResult> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.query(&meta.id, sql, params).await
    }

    pub async fn execute_user_db_batch(
        &self,
        caller: &Caller,
        id_or_name: &str,
        stmts: Vec<(String, Vec<SqlValue>)>,
    ) -> Result<u64> {
        let meta = self.resolve_user_db(id_or_name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("database '{}'", meta.name),
        )?;
        self.db.execute_batch(&meta.id, stmts).await
    }

    pub async fn snapshot_user_dbs(&self, caller: &Caller, out_dir: &Path) -> Result<usize> {
        Self::require_admin(caller)?;
        tokio::fs::create_dir_all(out_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("create dbs snapshot dir: {e}")))?;
        let dbs = self.registry.list_user_dbs().await?;
        let mut count = 0usize;
        for db in dbs {
            let safe_name: String = db
                .name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
                .collect();
            let dest = out_dir.join(format!("{safe_name}__{}.db", db.id));
            self.db.snapshot_to(&db.id, &dest).await?;
            count += 1;
        }
        Ok(count)
    }
}
