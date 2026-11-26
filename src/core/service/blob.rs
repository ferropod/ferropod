use std::collections::BTreeMap;

use chrono::Utc;

use crate::core::model::{
    BlobAuth, BlobBrowseResult, BlobFolderEntry, BucketMeta, BucketOpts, ByteRange, ByteStream,
    Caller, FerroError, ObjectMeta, Result, TokenScope,
};

use super::PlatformService;

fn normalize_folder_prefix(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    let inner = trimmed.trim_end_matches('/');
    if inner.is_empty() {
        return Ok(String::new());
    }
    if inner.contains('\0')
        || inner.contains('\\')
        || inner
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(FerroError::InvalidArgument(format!(
            "invalid folder prefix '{raw}'"
        )));
    }
    Ok(format!("{inner}/"))
}

fn content_type_allowed(allowed_list: &str, actual: &str) -> bool {
    let actual_essence = actual
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let patterns: Vec<String> = allowed_list
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|pat| {
        if pat == "*/*" || pat == "*" {
            true
        } else if let Some(prefix) = pat.strip_suffix("/*") {
            actual_essence.starts_with(&format!("{prefix}/"))
        } else {
            actual_essence == *pat
        }
    })
}

impl PlatformService {
    pub async fn get_bucket_meta(&self, name: &str) -> Result<Option<BucketMeta>> {
        self.objects.get_bucket(name).await
    }

    async fn get_bucket_or_err(&self, name: &str) -> Result<BucketMeta> {
        self.objects
            .get_bucket(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("bucket '{name}'")))
    }

    fn enforce_bucket_caller_rate_limit(
        &self,
        meta: &BucketMeta,
        auth: &BlobAuth<'_>,
    ) -> Result<()> {
        if let BlobAuth::Caller(caller) = auth {
            self.enforce_service_rate_limit(
                &format!("blob-actor:{}:{}", meta.name, caller.actor),
                meta.rate_limit_rpm,
                super::DEFAULT_BLOB_RATE_LIMIT_RPM,
                &format!("bucket '{}'", meta.name),
            )?;
        }
        Ok(())
    }

    async fn authorize_blob_read(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: Option<&str>,
    ) -> Result<BucketMeta> {
        let meta = self.get_bucket_or_err(bucket).await?;
        match auth {
            BlobAuth::Caller(caller) => {
                if !(meta.public_read && key.is_some()) {
                    Self::require_resource_read(
                        caller,
                        meta.project_id.as_deref(),
                        &format!("bucket '{bucket}'"),
                    )?;
                }
            }
            BlobAuth::Presigned { op, exp, sig } => {
                if *op != "get" {
                    return Err(FerroError::Unauthorized(
                        "presigned URL operation mismatch (expected 'get')".into(),
                    ));
                }
                let k = key.ok_or_else(|| {
                    FerroError::Unauthorized(
                        "presigned URL cannot be used for bucket listing".into(),
                    )
                })?;
                self.crypto
                    .verify_blob_op("get", bucket, k, *exp, sig, Utc::now().timestamp())?;
            }
            BlobAuth::Anonymous => {
                if !(meta.public_read && key.is_some()) {
                    return Err(FerroError::Unauthorized(format!(
                        "authentication required for bucket '{bucket}'"
                    )));
                }
            }
        }
        self.enforce_bucket_caller_rate_limit(&meta, auth)?;
        Ok(meta)
    }

    async fn authorize_blob_write(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: &str,
        allow_presigned_put: bool,
    ) -> Result<BucketMeta> {
        let meta = self.get_bucket_or_err(bucket).await?;
        match auth {
            BlobAuth::Caller(caller) => {
                Self::require_resource_write(
                    caller,
                    meta.project_id.as_deref(),
                    &format!("bucket '{bucket}'"),
                )?;
            }
            BlobAuth::Presigned { op, exp, sig } => {
                if !allow_presigned_put || *op != "put" {
                    return Err(FerroError::Unauthorized(
                        "presigned URL operation mismatch (expected 'put')".into(),
                    ));
                }
                self.crypto.verify_blob_op(
                    "put",
                    bucket,
                    key,
                    *exp,
                    sig,
                    Utc::now().timestamp(),
                )?;
            }
            BlobAuth::Anonymous => {
                return Err(FerroError::Unauthorized(format!(
                    "authentication required to write to bucket '{bucket}'"
                )));
            }
        }
        self.enforce_bucket_caller_rate_limit(&meta, auth)?;
        Ok(meta)
    }

    pub async fn create_bucket(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
    ) -> Result<BucketMeta> {
        self.create_grouped_bucket(caller, name, project_id, public_read, None)
            .await
    }

    pub async fn create_grouped_bucket(
        &self,
        caller: &Caller,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
        group: Option<&str>,
    ) -> Result<BucketMeta> {
        self.create_bucket_with_opts(
            caller,
            name,
            BucketOpts {
                group: group.map(str::to_owned),
                project_id: project_id.map(str::to_owned),
                public_read: Some(public_read),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn create_bucket_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        mut opts: BucketOpts,
    ) -> Result<BucketMeta> {
        opts.group = super::normalize_resource_group(opts.group.as_deref())?.map(str::to_owned);
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return Err(FerroError::InvalidArgument(format!(
                "invalid bucket name '{name}'"
            )));
        }
        let resolved_pid = self
            .resolve_resource_owner(caller, opts.project_id.as_deref(), "buckets")
            .await?;
        opts.project_id = resolved_pid.clone();
        opts.rate_limit_rpm = opts.rate_limit_rpm.filter(|v| *v > 0);

        let meta = self.objects.create_bucket_with_opts(name, opts).await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "bucket.create",
            serde_json::json!({ "name": meta.name, "public_read": meta.public_read }),
        );
        Ok(meta)
    }

    pub async fn update_bucket_with_opts(
        &self,
        caller: &Caller,
        name: &str,
        opts: BucketOpts,
    ) -> Result<BucketMeta> {
        let mut meta = self.get_bucket_or_err(name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("bucket '{name}'"),
        )?;
        if let Some(pub_read) = opts.public_read {
            meta.public_read = pub_read;
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
        if let Some(mos) = opts.max_object_size_mb {
            meta.max_object_size_mb = if mos == 0 { None } else { Some(mos) };
        }
        if let Some(q) = opts.quota_mb {
            meta.quota_mb = if q == 0 { None } else { Some(q) };
        }
        if let Some(act) = opts.allowed_content_types {
            let trimmed = act.trim();
            meta.allowed_content_types = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(cc) = opts.cache_control {
            let trimmed = cc.trim();
            meta.cache_control = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(cors) = opts.cors_origins {
            let trimmed = cors.trim();
            meta.cors_origins = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(rpm) = opts.rate_limit_rpm {
            meta.rate_limit_rpm = if rpm == 0 { None } else { Some(rpm) };
        }
        self.objects.update_bucket(&meta).await?;
        self.invalidate_bucket_cache_control(&meta.name);
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "bucket.update",
            serde_json::json!({ "name": meta.name, "public_read": meta.public_read }),
        );
        Ok(meta)
    }

    pub async fn list_buckets(&self, caller: &Caller) -> Result<Vec<BucketMeta>> {
        let all = self.objects.list_buckets().await?;
        match &caller.scope {
            TokenScope::Admin | TokenScope::Readonly => Ok(all),
            TokenScope::Project(pid) => Ok(all
                .into_iter()
                .filter(|b| b.project_id.as_deref() == Some(pid.as_str()))
                .collect()),
        }
    }

    pub async fn set_bucket_public(
        &self,
        caller: &Caller,
        name: &str,
        public_read: bool,
    ) -> Result<()> {
        let meta = self.get_bucket_or_err(name).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("bucket '{name}'"),
        )?;
        self.objects.set_bucket_public(name, public_read).await?;
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "bucket.update",
            serde_json::json!({ "name": name, "public_read": public_read }),
        );
        Ok(())
    }

    pub async fn delete_bucket(&self, caller: &Caller, name: &str) -> Result<()> {
        let meta = self
            .objects
            .get_bucket(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("bucket '{name}'")))?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("bucket '{name}'"),
        )?;
        self.objects.delete_bucket(name).await?;
        self.invalidate_bucket_cache_control(name);
        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "bucket.delete",
            serde_json::json!({ "name": name }),
        );
        Ok(())
    }

    pub async fn presign_blob_url(
        &self,
        caller: &Caller,
        op: &str,
        bucket: &str,
        key: &str,
        expires_secs: u64,
    ) -> Result<(String, i64)> {
        let op_norm = op.trim().to_ascii_lowercase();
        let meta = self.get_bucket_or_err(bucket).await?;
        match op_norm.as_str() {
            "get" => Self::require_resource_read(
                caller,
                meta.project_id.as_deref(),
                &format!("bucket '{bucket}'"),
            )?,
            "put" => Self::require_resource_write(
                caller,
                meta.project_id.as_deref(),
                &format!("bucket '{bucket}'"),
            )?,
            _ => {
                return Err(FerroError::InvalidArgument(format!(
                    "unsupported presign op '{op}', expected 'get' or 'put'"
                )));
            }
        }
        let key = crate::storage::blob_fs::FsBlobStore::validate_object_key(key)?;
        let ttl = expires_secs.clamp(1, 86_400) as i64;
        let expires_at = Utc::now().timestamp() + ttl;
        let sig = self.crypto.sign_blob_op(&op_norm, bucket, &key, expires_at);
        let encoded_key = encode_url_path_segments(&key);
        let url = format!("/_blob/{bucket}/{encoded_key}?op={op_norm}&exp={expires_at}&sig={sig}");
        Ok((url, expires_at))
    }

    pub async fn put_object_stream(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: &str,
        stream: ByteStream,
        content_type: &str,
        custom_meta: Option<String>,
    ) -> Result<ObjectMeta> {
        let b_meta = self.authorize_blob_write(auth, bucket, key, true).await?;
        if let Some(ref allowed) = b_meta.allowed_content_types {
            if !content_type_allowed(allowed, content_type) {
                return Err(FerroError::InvalidArgument(format!(
                    "content-type '{content_type}' is not allowed in bucket '{bucket}' (allowed: {allowed})"
                )));
            }
        }
        let obj = self
            .objects
            .put_stream(bucket, key, stream, content_type, custom_meta)
            .await?;
        if let Some(max_mb) = b_meta.max_object_size_mb.filter(|m| *m > 0) {
            if obj.size > (max_mb as u64).saturating_mul(1024 * 1024) {
                let _ = self.objects.delete(bucket, key).await;
                return Err(FerroError::InvalidArgument(format!(
                    "object size ({} bytes) exceeds bucket max_object_size_mb ({max_mb} MiB)",
                    obj.size
                )));
            }
        }
        if let Some(quota_mb) = b_meta.quota_mb.filter(|q| *q > 0) {
            let total = self.objects.total_bucket_bytes(bucket).await?;
            if total > (quota_mb as u64).saturating_mul(1024 * 1024) {
                let _ = self.objects.delete(bucket, key).await;
                return Err(FerroError::InvalidArgument(format!(
                    "bucket '{bucket}' exceeded storage quota ({quota_mb} MiB)"
                )));
            }
        }
        Ok(obj)
    }

    pub async fn head_object(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: &str,
    ) -> Result<Option<ObjectMeta>> {
        self.authorize_blob_read(auth, bucket, Some(key)).await?;
        self.objects.head(bucket, key).await
    }

    pub async fn get_object_stream(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<(ObjectMeta, ByteStream, u64, Option<(u64, u64)>)>> {
        self.authorize_blob_read(auth, bucket, Some(key)).await?;
        self.objects.get_stream(bucket, key, range).await
    }

    pub async fn delete_object(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        key: &str,
    ) -> Result<bool> {
        self.authorize_blob_write(auth, bucket, key, false).await?;
        self.objects.delete(bucket, key).await
    }

    pub async fn list_objects(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectMeta>> {
        self.authorize_blob_read(auth, bucket, None).await?;
        self.objects.list(bucket, prefix, limit).await
    }

    pub async fn list_objects_page(
        &self,
        auth: &BlobAuth<'_>,
        bucket: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ObjectMeta>, Option<String>)> {
        self.authorize_blob_read(auth, bucket, None).await?;
        self.objects.list_page(bucket, prefix, cursor, limit).await
    }

    pub async fn browse_bucket(
        &self,
        caller: &Caller,
        bucket: &str,
        prefix: Option<&str>,
        search: Option<&str>,
        max_scan: usize,
    ) -> Result<BlobBrowseResult> {
        let meta = self.get_bucket_or_err(bucket).await?;
        Self::require_resource_read(
            caller,
            meta.project_id.as_deref(),
            &format!("bucket '{bucket}'"),
        )?;
        self.enforce_service_rate_limit(
            &format!("blob-actor:{}:{}", meta.name, caller.actor),
            meta.rate_limit_rpm,
            super::DEFAULT_BLOB_RATE_LIMIT_RPM,
            &format!("bucket '{}'", meta.name),
        )?;

        let norm_prefix = normalize_folder_prefix(prefix.unwrap_or(""))?;
        let search_q = search
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);

        let scan_cap = max_scan.clamp(100, 50_000);
        let mut cursor: Option<String> = None;
        let mut scanned = 0usize;
        let mut truncated = false;
        let mut folders_map: BTreeMap<String, BlobFolderEntry> = BTreeMap::new();
        let mut objects: Vec<ObjectMeta> = Vec::new();

        loop {
            let (page, next_cursor) = self
                .objects
                .list_page(
                    bucket,
                    (!norm_prefix.is_empty()).then_some(norm_prefix.as_str()),
                    cursor.as_deref(),
                    1000,
                )
                .await?;
            scanned += page.len();

            if let Some(ref q) = search_q {
                for obj in page {
                    let rel = obj.key.strip_prefix(&norm_prefix).unwrap_or(&obj.key);
                    if rel.to_ascii_lowercase().contains(q) && objects.len() < 500 {
                        objects.push(obj);
                    }
                }
            } else {
                for obj in page {
                    let rel = obj.key.strip_prefix(&norm_prefix).unwrap_or(&obj.key);
                    if let Some(slash) = rel.find('/') {
                        let folder_name = &rel[..slash];
                        let folder_prefix = format!("{norm_prefix}{folder_name}/");
                        let entry =
                            folders_map
                                .entry(folder_prefix.clone())
                                .or_insert_with(|| BlobFolderEntry {
                                    prefix: folder_prefix,
                                    name: folder_name.to_string(),
                                    object_count: 0,
                                    total_size: 0,
                                    updated_at: obj.updated_at,
                                });
                        entry.object_count += 1;
                        entry.total_size = entry.total_size.saturating_add(obj.size);
                        if obj.updated_at > entry.updated_at {
                            entry.updated_at = obj.updated_at;
                        }
                    } else {
                        objects.push(obj);
                    }
                }
            }

            if next_cursor.is_none() {
                break;
            }
            if scanned >= scan_cap {
                truncated = true;
                break;
            }
            cursor = next_cursor;
        }

        Ok(BlobBrowseResult {
            prefix: norm_prefix,
            folders: folders_map.into_values().collect(),
            objects,
            truncated,
        })
    }

    pub async fn delete_objects_by_prefix(
        &self,
        caller: &Caller,
        bucket: &str,
        prefix: &str,
    ) -> Result<(String, u64)> {
        let meta = self.get_bucket_or_err(bucket).await?;
        Self::require_resource_write(
            caller,
            meta.project_id.as_deref(),
            &format!("bucket '{bucket}'"),
        )?;

        let norm_prefix = normalize_folder_prefix(prefix)?;
        if norm_prefix.is_empty() {
            return Err(FerroError::InvalidArgument(
                "folder prefix must not be empty".into(),
            ));
        }

        let mut deleted = 0u64;
        let mut cursor: Option<String> = None;
        loop {
            let (page, next_cursor) = self
                .objects
                .list_page(bucket, Some(&norm_prefix), cursor.as_deref(), 500)
                .await?;
            if page.is_empty() {
                break;
            }
            let mut page_deleted = 0u64;
            for obj in page {
                if self.objects.delete(bucket, &obj.key).await? {
                    deleted += 1;
                    page_deleted += 1;
                }
            }
            if page_deleted == 0 {
                cursor = next_cursor;
                if cursor.is_none() {
                    break;
                }
            } else {
                cursor = None;
            }
        }

        self.audit(
            meta.project_id.as_deref(),
            &caller.actor,
            "bucket.delete_prefix",
            serde_json::json!({
                "bucket": bucket,
                "prefix": norm_prefix,
                "deleted": deleted,
            }),
        );

        Ok((norm_prefix, deleted))
    }

    fn invalidate_bucket_cache_control(&self, bucket: &str) {
        let mut cache = self
            .caches
            .bucket_cache_control
            .write()
            .unwrap_or_else(|e| e.into_inner());
        cache.remove(bucket);
    }

    /// Resolve a bucket's optional `Cache-Control` header with a lightweight in-memory cache.
    pub async fn static_bucket_cache_control(&self, bucket: &str) -> Option<String> {
        {
            let cache = self
                .caches
                .bucket_cache_control
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(cached) = cache.get(bucket) {
                return cached.clone();
            }
        }
        let cc = self
            .objects
            .get_bucket(bucket)
            .await
            .ok()
            .flatten()
            .and_then(|b| b.cache_control);
        let mut cache = self
            .caches
            .bucket_cache_control
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 2048 {
            cache.clear();
        }
        cache.insert(bucket.to_string(), cc.clone());
        cc
    }

    /// Internal static-site asset HEAD bypasses bucket auth because the deployment explicitly bound `assets_bucket`.
    pub async fn static_head_object(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>> {
        self.objects.head(bucket, key).await
    }

    /// Open a static-site asset stream directly from resolved [`ObjectMeta`], avoiding a second DB lookup.
    pub async fn static_open_stream_for_meta(
        &self,
        meta: ObjectMeta,
        range: Option<ByteRange>,
    ) -> Result<(ObjectMeta, ByteStream, u64, Option<(u64, u64)>)> {
        self.objects.open_stream_for_meta(meta, range).await
    }

    /// Internal static-site asset GET bypasses bucket auth because the deployment explicitly bound `assets_bucket`.
    pub async fn static_get_object_stream(
        &self,
        bucket: &str,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<(ObjectMeta, ByteStream, u64, Option<(u64, u64)>)>> {
        self.objects.get_stream(bucket, key, range).await
    }
}

fn encode_url_path_segments(key: &str) -> String {
    use std::fmt::Write;
    key.split('/')
        .map(|seg| {
            let mut out = String::with_capacity(seg.len());
            for &b in seg.as_bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    out.push(b as char);
                } else {
                    let _ = write!(&mut out, "%{b:02X}");
                }
            }
            out
        })
        .collect::<Vec<_>>()
        .join("/")
}
