use async_trait::async_trait;
use chrono::Utc;
use turso::Value as TursoValue;
use uuid::Uuid;

use crate::core::model::{ApiToken, Result, TokenScope};
use crate::core::traits::{SecretStore, TokenStore};
use crate::storage::tx::{connect_db, map_turso_err, with_retry};

use super::{opt_i64, TursoCoreStore};

#[async_trait]
impl SecretStore for TursoCoreStore {
    async fn set_secret(
        &self,
        project_id: &str,
        key: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<()> {
        let now = Utc::now().timestamp();
        let pid = project_id.to_string();
        let key_s = key.to_string();
        let nonce_v = nonce.to_vec();
        let ct_v = ciphertext.to_vec();
        with_retry(|| {
            let db = self.db.clone();
            let pid = pid.clone();
            let key_s = key_s.clone();
            let nonce_v = nonce_v.clone();
            let ct_v = ct_v.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO secrets (project_id, key, nonce, ciphertext, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(project_id, key) DO UPDATE SET
                        nonce = excluded.nonce,
                        ciphertext = excluded.ciphertext,
                        updated_at = excluded.updated_at",
                    (pid, key_s, nonce_v, ct_v, now),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn get_secret(&self, project_id: &str, key: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT nonce, ciphertext FROM secrets WHERE project_id = ?1 AND key = ?2",
                (project_id, key),
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let nonce: Vec<u8> = row.get(0).map_err(map_turso_err)?;
            let ct: Vec<u8> = row.get(1).map_err(map_turso_err)?;
            Ok(Some((nonce, ct)))
        } else {
            Ok(None)
        }
    }

    async fn list_secret_keys(&self, project_id: &str) -> Result<Vec<String>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT key FROM secrets WHERE project_id = ?1 ORDER BY key ASC",
                [project_id],
            )
            .await
            .map_err(map_turso_err)?;
        let mut keys = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            keys.push(row.get(0).map_err(map_turso_err)?);
        }
        Ok(keys)
    }

    async fn delete_secret(&self, project_id: &str, key: &str) -> Result<()> {
        let pid = project_id.to_string();
        let key_s = key.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let pid = pid.clone();
            let key_s = key_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "DELETE FROM secrets WHERE project_id = ?1 AND key = ?2",
                    (pid, key_s),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn delete_project_secrets(&self, project_id: &str) -> Result<()> {
        let pid = project_id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let pid = pid.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM secrets WHERE project_id = ?1", [pid])
                    .await
            }
        })
        .await?;
        Ok(())
    }
}

#[async_trait]
impl TokenStore for TursoCoreStore {
    async fn create_token(
        &self,
        name: &str,
        token_hash: &str,
        scope: TokenScope,
        expires_at: Option<i64>,
    ) -> Result<ApiToken> {
        let now = Utc::now().timestamp();
        let id = format!("tok_{}", Uuid::new_v4().simple());
        let scope_s = scope.as_str();
        let name_s = name.to_string();
        let hash_s = token_hash.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id = id.clone();
            let name_s = name_s.clone();
            let hash_s = hash_s.clone();
            let scope_s = scope_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO api_tokens (id, name, token_hash, scope, expires_at, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    vec![
                        TursoValue::Text(id),
                        TursoValue::Text(name_s),
                        TursoValue::Text(hash_s),
                        TursoValue::Text(scope_s),
                        expires_at
                            .map(TursoValue::Integer)
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(now),
                    ],
                )
                .await
            }
        })
        .await?;

        Ok(ApiToken {
            id,
            name: name.to_string(),
            scope,
            expires_at,
            last_used_at: None,
            created_at: now,
        })
    }

    async fn verify_token_hash(&self, token_hash: &str, now_secs: i64) -> Result<Option<ApiToken>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, scope, expires_at, last_used_at, created_at
                 FROM api_tokens WHERE token_hash = ?1 LIMIT 1",
                [token_hash],
            )
            .await
            .map_err(map_turso_err)?;
        let Some(row) = rows.next().await.map_err(map_turso_err)? else {
            return Ok(None);
        };
        let expires_at = opt_i64(&row, 3)?;
        if let Some(exp) = expires_at {
            if now_secs > exp {
                return Ok(None);
            }
        }
        let id: String = row.get(0).map_err(map_turso_err)?;
        let scope_s: String = row.get(2).map_err(map_turso_err)?;
        let token = ApiToken {
            id: id.clone(),
            name: row.get(1).map_err(map_turso_err)?,
            scope: TokenScope::parse(&scope_s)?,
            expires_at,
            last_used_at: opt_i64(&row, 4)?,
            created_at: row.get(5).map_err(map_turso_err)?,
        };
        drop(rows);
        let _ = conn
            .execute(
                "UPDATE api_tokens SET last_used_at = ?1 WHERE id = ?2",
                (now_secs, id),
            )
            .await;
        Ok(Some(token))
    }

    async fn list_tokens(&self) -> Result<Vec<ApiToken>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, name, scope, expires_at, last_used_at, created_at
                 FROM api_tokens ORDER BY created_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let scope_s: String = row.get(2).map_err(map_turso_err)?;
            out.push(ApiToken {
                id: row.get(0).map_err(map_turso_err)?,
                name: row.get(1).map_err(map_turso_err)?,
                scope: TokenScope::parse(&scope_s)?,
                expires_at: opt_i64(&row, 3)?,
                last_used_at: opt_i64(&row, 4)?,
                created_at: row.get(5).map_err(map_turso_err)?,
            });
        }
        Ok(out)
    }

    async fn revoke_token(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM api_tokens WHERE id = ?1", [id_s])
                    .await
            }
        })
        .await?;
        Ok(())
    }
}
