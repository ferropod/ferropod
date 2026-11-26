use bytes::Bytes;

use crate::core::model::{
    Caller, FerroError, KvKeyMeta, KvNamespaceMeta, KvNamespaceOpts, KvValue, Result, TokenScope,
    DEFAULT_KV_TTL_SECS,
};

use super::PlatformService;

impl PlatformService {
    async fn resolve_kv_namespace_for_read(
        &self,
        caller: &Caller,
        ns: &str,
    ) -> Result<Option<KvNamespaceMeta>> {
        let meta = self.registry.get_kv_namespace(ns).await?;
        let owner = meta.as_ref().and_then(|m| m.project_id.as_deref());
        Self::require_resource_read(caller, owner, &format!("kv namespace '{ns}'"))?;
        Ok(meta)
    }

    async fn resolve_kv_namespace_for_write(
        &self,
        caller: &Caller,
        ns: &str,
    ) -> Result<Option<KvNamespaceMeta>> {
        let meta = self.registry.get_kv_namespace(ns).await?;
        let owner = meta.as_ref().and_then(|m| m.project_id.as_deref());
        Self::require_resource_write(caller, owner, &format!("kv namespace '{ns}'"))?;
        Ok(meta)
    }

    pub async fn create_kv_namespace(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        self.create_grouped_kv_namespace(caller, name, project_id, None)
            .await
    }

    pub async fn create_grouped_kv_namespace(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        group: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        self.create_kv_namespace_with_opts(
            caller,
            name,
            KvNamespaceOpts {
                group: group.map(str::to_owned),
                project_id: project_id.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn create_kv_namespace_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        mut opts: KvNamespaceOpts,
    ) -> Result<KvNamespaceMeta> {
        opts.group = super::normalize_resource_group(opts.group.as_deref())?.map(str::to_owned);
        let name = name.trim();
        if name.is_empty() {
            return Err(FerroError::InvalidArgument(
                "kv namespace name cannot be empty".into(),
            ));
        }
        if opts.default_ttl_secs == Some(0) {
            return Err(FerroError::InvalidArgument(
                "kv namespace default_ttl_secs must be greater than 0 (KV entries must have an expiration time)".into(),
            ));
        }
        let resolved_pid = match (&caller.scope, opts.project_id.as_deref()) {
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
        opts.project_id = resolved_pid.clone();
        opts.default_ttl_secs = Some(opts.default_ttl_secs.unwrap_or(DEFAULT_KV_TTL_SECS).max(1));

        let meta = self
            .registry
            .create_kv_namespace_with_opts(name, opts)
            .await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "kv.namespace.create",
            serde_json::json!({
                "name": meta.name,
                "default_ttl_secs": meta.default_ttl_secs,
                "default_ephemeral": meta.default_ephemeral,
            }),
        );
        Ok(meta)
    }

    pub async fn update_kv_namespace(
        &self,
        caller: &Caller,
        name: &str,
        opts: KvNamespaceOpts,
    ) -> Result<KvNamespaceMeta> {
        let mut meta = self
            .registry
            .get_kv_namespace(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("kv namespace '{name}'")))?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("kv namespace '{name}'"),
        )?;
        if let Some(ttl) = opts.default_ttl_secs {
            if ttl == 0 {
                return Err(FerroError::InvalidArgument(
                    "kv namespace default_ttl_secs must be greater than 0 (KV entries must have an expiration time)".into(),
                ));
            }
            meta.default_ttl_secs = ttl;
        }
        if let Some(g) = opts.group {
            meta.group = super::normalize_resource_group(Some(&g))?.map(str::to_owned);
        }
        if let Some(desc) = opts.description {
            let trimmed = desc.trim();
            meta.description = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(eph) = opts.default_ephemeral {
            meta.default_ephemeral = eph;
        }
        if let Some(mvb) = opts.max_value_bytes {
            meta.max_value_bytes = if mvb == 0 { None } else { Some(mvb) };
        }
        if let Some(mk) = opts.max_keys {
            meta.max_keys = if mk == 0 { None } else { Some(mk) };
        }
        if let Some(rpm) = opts.rate_limit_rpm {
            meta.rate_limit_rpm = if rpm == 0 { None } else { Some(rpm) };
        }
        self.registry.update_kv_namespace(&meta).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "kv.namespace.update",
            serde_json::json!({
                "name": meta.name,
                "default_ttl_secs": meta.default_ttl_secs,
                "default_ephemeral": meta.default_ephemeral,
            }),
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

    pub async fn delete_kv_namespace(&self, caller: &Caller, name: &str) -> Result<()> {
        let meta = self
            .registry
            .get_kv_namespace(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("kv namespace '{name}'")))?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("kv namespace '{name}'"),
        )?;
        self.kv.delete_namespace(name).await?;
        self.registry.delete_kv_namespace(name).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "kv.namespace.delete",
            serde_json::json!({ "name": name }),
        );
        Ok(())
    }

    fn enforce_kv_rate_limit(
        &self,
        meta: Option<&KvNamespaceMeta>,
        ns: &str,
        actor: &str,
    ) -> Result<()> {
        self.enforce_service_rate_limit(
            &format!("kv:{ns}:{actor}"),
            meta.and_then(|m| m.rate_limit_rpm),
            super::DEFAULT_KV_RATE_LIMIT_RPM,
            &format!("kv namespace '{ns}'"),
        )
    }

    pub async fn kv_get(&self, caller: &Caller, ns: &str, key: &str) -> Result<Option<KvValue>> {
        let meta = self.resolve_kv_namespace_for_read(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.kv.get(ns, key).await
    }

    async fn check_kv_quotas(
        &self,
        ns: &str,
        key: &str,
        val_len: usize,
        meta: Option<&KvNamespaceMeta>,
    ) -> Result<()> {
        if let Some(m) = meta {
            if let Some(max_bytes) = m.max_value_bytes {
                if val_len > max_bytes {
                    return Err(FerroError::InvalidArgument(format!(
                        "kv value size ({val_len} bytes) exceeds namespace max_value_bytes ({max_bytes})"
                    )));
                }
            }
            if let Some(max_keys) = m.max_keys {
                if self.kv.get(ns, key).await?.is_none()
                    && self.kv.count_keys(ns).await? >= max_keys
                {
                    return Err(FerroError::InvalidArgument(format!(
                        "kv namespace '{ns}' has reached its max_keys quota ({max_keys})"
                    )));
                }
            }
        }
        Ok(())
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
        let meta = self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.check_kv_quotas(ns, key, val.len(), meta.as_ref())
            .await?;
        let ns_default_ttl = meta
            .as_ref()
            .map(|m| m.default_ttl_secs.max(1))
            .unwrap_or(DEFAULT_KV_TTL_SECS);
        let effective_ttl = Some(ttl_secs.unwrap_or(ns_default_ttl));
        let effective_ephemeral = ephemeral || meta.as_ref().is_some_and(|m| m.default_ephemeral);
        self.kv
            .put(ns, key, val, metadata, effective_ttl, effective_ephemeral)
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
        let meta = self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.check_kv_quotas(ns, key, val.len(), meta.as_ref())
            .await?;
        let ns_default_ttl = meta
            .as_ref()
            .map(|m| m.default_ttl_secs.max(1))
            .unwrap_or(DEFAULT_KV_TTL_SECS);
        let effective_ttl = Some(ttl_secs.unwrap_or(ns_default_ttl));
        self.kv
            .put_if_absent(ns, key, val, metadata, effective_ttl)
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
        let meta = self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.check_kv_quotas(ns, key, 8, meta.as_ref()).await?;
        let ns_default_ttl = meta
            .as_ref()
            .map(|m| m.default_ttl_secs.max(1))
            .unwrap_or(DEFAULT_KV_TTL_SECS);
        let effective_ttl = match ttl_secs {
            Some(t) => Some(t),
            None => {
                if self
                    .kv
                    .get(ns, key)
                    .await?
                    .and_then(|v| v.expires_at)
                    .is_some()
                {
                    None
                } else {
                    Some(ns_default_ttl)
                }
            }
        };
        self.kv.incr(ns, key, delta, effective_ttl).await
    }

    pub async fn kv_delete(&self, caller: &Caller, ns: &str, key: &str) -> Result<()> {
        let meta = self.resolve_kv_namespace_for_write(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.kv.delete(ns, key).await
    }

    pub async fn kv_list(
        &self,
        caller: &Caller,
        ns: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<KvKeyMeta>> {
        let meta = self.resolve_kv_namespace_for_read(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.kv.list(ns, prefix, limit).await
    }

    pub async fn kv_list_page(
        &self,
        caller: &Caller,
        ns: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<KvKeyMeta>, Option<String>)> {
        let meta = self.resolve_kv_namespace_for_read(caller, ns).await?;
        self.enforce_kv_rate_limit(meta.as_ref(), ns, &caller.actor)?;
        self.kv.list_page(ns, prefix, cursor, limit).await
    }
}
