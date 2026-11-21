use chrono::Utc;

use crate::core::model::{
    BlobAuth, BucketMeta, ByteRange, ByteStream, Caller, FerroError, ObjectMeta, Result, TokenScope,
};

use super::PlatformService;

impl PlatformService {
    async fn get_bucket_or_err(&self, name: &str) -> Result<BucketMeta> {
        self.objects
            .get_bucket(name)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("bucket '{name}'")))
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
                if meta.public_read && key.is_some() {
                    return Ok(meta);
                }
                Self::require_resource_read(
                    caller,
                    meta.project_id.as_deref(),
                    &format!("bucket '{bucket}'"),
                )?;
                Ok(meta)
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
                Ok(meta)
            }
            BlobAuth::Anonymous => {
                if meta.public_read && key.is_some() {
                    Ok(meta)
                } else {
                    Err(FerroError::Unauthorized(format!(
                        "authentication required for bucket '{bucket}'"
                    )))
                }
            }
        }
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
                Ok(meta)
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
                Ok(meta)
            }
            BlobAuth::Anonymous => Err(FerroError::Unauthorized(format!(
                "authentication required to write to bucket '{bucket}'"
            ))),
        }
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
        let group = super::normalize_resource_group(group)?;
        let name = name.trim();
        if name.is_empty() || name.contains('/') {
            return Err(FerroError::InvalidArgument(format!(
                "invalid bucket name '{name}'"
            )));
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
                    "readonly token cannot create buckets".into(),
                ));
            }
        };

        let meta = self
            .objects
            .create_bucket(name, resolved_pid.as_deref(), public_read, group)
            .await?;
        self.audit(
            resolved_pid.as_deref(),
            &caller.actor,
            "bucket.create",
            serde_json::json!({ "name": meta.name, "public_read": public_read }),
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
        let key = key.trim_start_matches('/');
        if key.is_empty() {
            return Err(FerroError::InvalidArgument(
                "object key cannot be empty".into(),
            ));
        }
        let ttl = expires_secs.clamp(1, 86_400) as i64;
        let expires_at = Utc::now().timestamp() + ttl;
        let sig = self.crypto.sign_blob_op(&op_norm, bucket, key, expires_at);
        let url = format!("/_blob/{bucket}/{key}?op={op_norm}&exp={expires_at}&sig={sig}");
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
        self.authorize_blob_write(auth, bucket, key, true).await?;
        self.objects
            .put_stream(bucket, key, stream, content_type, custom_meta)
            .await
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

    /// Internal static-site asset HEAD bypasses bucket auth because the deployment explicitly bound `assets_bucket`.
    pub async fn static_head_object(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>> {
        self.objects.head(bucket, key).await
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
