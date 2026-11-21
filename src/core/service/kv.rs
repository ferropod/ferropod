use bytes::Bytes;

use crate::core::model::{
    Caller, FerroError, KvKeyMeta, KvNamespaceMeta, KvValue, Result, TokenScope,
};

use super::PlatformService;

impl PlatformService {
    async fn resolve_kv_namespace_for_read(&self, caller: &Caller, ns: &str) -> Result<()> {
        let meta = self.registry.get_kv_namespace(ns).await?;
        let owner = meta.as_ref().and_then(|m| m.project_id.as_deref());
        Self::require_resource_read(caller, owner, &format!("kv namespace '{ns}'"))
    }

    async fn resolve_kv_namespace_for_write(&self, caller: &Caller, ns: &str) -> Result<()> {
        let meta = self.registry.get_kv_namespace(ns).await?;
        let owner = meta.as_ref().and_then(|m| m.project_id.as_deref());
        Self::require_resource_write(caller, owner, &format!("kv namespace '{ns}'"))
    }

    pub async fn create_kv_namespace(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "kv namespace name cannot be empty".into(),
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
                    "readonly token cannot create kv namespaces".into(),
                ));
            }
        };

        let meta = self
            .registry
            .create_kv_namespace(name, resolved_pid.as_deref())
            .await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "kv.namespace.create",
            serde_json::json!({ "name": meta.name }),
        );
        Ok(meta)
    }

    pub async fn list_kv_namespaces(&self, caller: &Caller) -> Result<Vec<KvNamespaceMeta>> {
        let all = self.registry.list_kv_namespaces().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|m| m.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn kv_get(&self, caller: &Caller, ns: &str, key: &str) -> Result<Option<KvValue>> {
        self.resolve_kv_namespace_for_read(caller, ns).await?;
        self.kv.get(ns, key).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn kv_put(
        &self,
        caller: &Caller,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
        ephemeral: bool,
    ) -> Result<()> {
        self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.kv
            .put(ns, key, val, metadata, ttl_secs, ephemeral)
            .await
    }

    pub async fn kv_put_if_absent(
        &self,
        caller: &Caller,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
    ) -> Result<bool> {
        self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.kv
            .put_if_absent(ns, key, val, metadata, ttl_secs)
            .await
    }

    pub async fn kv_incr(
        &self,
        caller: &Caller,
        ns: &str,
        key: &str,
        delta: i64,
        ttl_secs: Option<u64>,
    ) -> Result<i64> {
        self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.kv.incr(ns, key, delta, ttl_secs).await
    }

    pub async fn kv_delete(&self, caller: &Caller, ns: &str, key: &str) -> Result<()> {
        self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.kv.delete(ns, key).await
    }

    pub async fn kv_list(
        &self,
        caller: &Caller,
        ns: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<KvKeyMeta>> {
        self.resolve_kv_namespace_for_read(caller, ns).await?;
        self.kv.list(ns, prefix, limit).await
    }
}
