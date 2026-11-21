use chrono::Utc;

use crate::core::crypto::Crypto;
use crate::core::model::{ApiToken, Caller, FerroError, Result, TokenScope};

use super::PlatformService;

impl PlatformService {
    pub async fn create_token(
        &self,
        caller: &Caller,
        name: &str,
        scope: TokenScope,
        expires_at: Option<i64>,
    ) -> Result<(ApiToken, String)> {
        Self::require_admin(caller)?;
        let raw = Crypto::generate_token();
        let hash = Crypto::hash_token(&raw);
        let tok = self
            .tokens
            .create_token(name, &hash, scope.clone(), expires_at)
            .await?;
        self.audit(
            None,
            &caller.actor,
            "token.create",
            serde_json::json!({ "id": tok.id, "name": tok.name, "scope": scope.as_str() }),
        );
        Ok((tok, raw))
    }

    pub async fn authenticate_token(&self, raw_token: &str) -> Result<ApiToken> {
        let hash = Crypto::hash_token(raw_token);
        let now = Utc::now().timestamp();
        self.tokens
            .verify_token_hash(&hash, now)
            .await?
            .ok_or_else(|| FerroError::Unauthorized("invalid or expired token".into()))
    }

    pub async fn list_tokens(&self, caller: &Caller) -> Result<Vec<ApiToken>> {
        Self::require_admin(caller)?;
        self.tokens.list_tokens().await
    }

    pub async fn revoke_token(&self, caller: &Caller, id: &str) -> Result<()> {
        Self::require_admin(caller)?;
        self.tokens.revoke_token(id).await?;
        self.audit(
            None,
            &caller.actor,
            "token.revoke",
            serde_json::json!({ "id": id }),
        );
        Ok(())
    }
}
