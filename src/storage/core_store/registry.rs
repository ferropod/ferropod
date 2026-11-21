use async_trait::async_trait;
use chrono::Utc;

use crate::core::model::{KvNamespaceMeta, Result, UserDbMeta};
use crate::core::traits::RegistryStore;
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{opt_text, TursoCoreStore};

#[async_trait]
impl RegistryStore for TursoCoreStore {
    async fn register_user_db(&self, id: &str, name: &str, project_id: Option<&str>) -> Result<()> {
        let now = Utc::now().timestamp();
        let id_s = id.to_string();
        let name_s = name.to_string();
        let proj_s = project_id.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            let name_s = name_s.clone();
            let proj_s = proj_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO user_databases (id, name, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                    (id_s, name_s, proj_s, now),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn delete_user_db(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM user_databases WHERE id = ?1", [id_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn get_user_db(&self, id_or_name: &str) -> Result<Option<UserDbMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, project_id, created_at FROM user_databases WHERE id = ?1 OR name = ?1 LIMIT 1",
                [id_or_name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(UserDbMeta {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                created_at: row.get(3).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_user_dbs(&self) -> Result<Vec<UserDbMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, project_id, created_at FROM user_databases ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(UserDbMeta {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                project_id: opt_text(&row, 2)?,
                created_at: row.get(3).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }

    async fn create_kv_namespace(
        &self,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        let now = Utc::now().timestamp();
        let name_s = name.to_string();
        let proj_s = project_id.map(str::to_string);
        with_retry(|| {
            let db = self.db.clone();
            let name_s = name_s.clone();
            let proj_s = proj_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO kv_namespaces (name, project_id, created_at) VALUES (?1, ?2, ?3)",
                    (name_s, proj_s, now),
                )
                .await
            }
        })
        .await?;

        Ok(KvNamespaceMeta {
            name: name.to_string(),
            project_id: project_id.map(str::to_string),
            created_at: now,
        })
    }

    async fn get_kv_namespace(&self, name: &str) -> Result<Option<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, created_at FROM kv_namespaces WHERE name = ?1 LIMIT 1",
                [name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(KvNamespaceMeta {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                created_at: row.get(2).map_err(map_turso_err)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_kv_namespaces(&self) -> Result<Vec<KvNamespaceMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, created_at FROM kv_namespaces ORDER BY name ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(KvNamespaceMeta {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                created_at: row.get(2).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }
}
