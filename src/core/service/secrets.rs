use crate::core::model::{Caller, FerroError, Result};

use super::PlatformService;

impl PlatformService {
    pub async fn set_secret(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        let key = key.trim();
        if key.is_empty() {
            return Err(FerroError::InvalidArgument(
                "secret key cannot be empty".into(),
            ));
        }
        let (nonce, ciphertext) = self.crypto.encrypt_secret(value.as_bytes())?;
        self.secrets
            .set_secret(&proj.id, key, &nonce, &ciphertext)
            .await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "secret.set",
            serde_json::json!({ "key": key }),
        );
        Ok(())
    }

    pub async fn get_secret(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        key: &str,
    ) -> Result<Option<String>> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        let Some((nonce, ciphertext)) = self.secrets.get_secret(&proj.id, key).await? else {
            return Ok(None);
        };
        let plaintext = self.crypto.decrypt_secret(&nonce, &ciphertext)?;
        let text = String::from_utf8(plaintext)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("invalid utf8 secret: {e}")))?;
        Ok(Some(text))
    }

    pub async fn list_secret_keys(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
    ) -> Result<Vec<String>> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        self.secrets.list_secret_keys(&proj.id).await
    }

    pub async fn delete_secret(
        &self,
        caller: &Caller,
        project_id_or_name: &str,
        key: &str,
    ) -> Result<()> {
        let proj = self.get_project_internal(project_id_or_name).await?;
        Self::require_project_write(caller, &proj.id)?;
        self.secrets.delete_secret(&proj.id, key).await?;
        self.audit(
            Some(&proj.id),
            &caller.actor,
            "secret.delete",
            serde_json::json!({ "key": key }),
        );
        Ok(())
    }
}
