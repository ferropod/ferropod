use std::io::SeekFrom;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::core::model::{BucketMeta, ByteRange, ByteStream, FerroError, ObjectMeta, Result};
use crate::core::traits::{BlobMetaStore, ObjectStore};

const CHUNK_SIZE: usize = 64 * 1024;

/// Content-addressed local blob storage backed by `BlobMetaStore` metadata and streaming `/data/blobs` I/O.
#[derive(Clone)]
pub struct FsBlobStore {
    blobs_dir: PathBuf,
    meta: Arc<dyn BlobMetaStore>,
    gc_grace_secs: i64,
}

impl FsBlobStore {
    pub fn new(
        blobs_dir: PathBuf,
        meta: Arc<dyn BlobMetaStore>,
        gc_grace_secs: i64,
    ) -> Result<Self> {
        std::fs::create_dir_all(&blobs_dir)
            .map_err(|e| FerroError::Internal(anyhow!("failed to create blobs dir: {e}")))?;
        std::fs::create_dir_all(blobs_dir.join("_tmp"))
            .map_err(|e| FerroError::Internal(anyhow!("failed to create blobs tmp dir: {e}")))?;
        Ok(Self {
            blobs_dir,
            meta,
            gc_grace_secs,
        })
    }

    fn validate_name(name: &str) -> Result<()> {
        if name.is_empty()
            || name.len() > 128
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err(FerroError::InvalidArgument(format!(
                "invalid bucket name '{name}'"
            )));
        }
        Ok(())
    }

    fn normalize_key(key: &str) -> Result<String> {
        let trimmed = key.trim_start_matches('/');
        if trimmed.is_empty() || trimmed.contains('\0') || trimmed.split('/').any(|s| s == "..") {
            return Err(FerroError::InvalidArgument(format!(
                "invalid object key '{key}'"
            )));
        }
        Ok(trimmed.to_string())
    }

    fn disk_path(&self, rel_path: &str) -> PathBuf {
        self.blobs_dir.join(rel_path)
    }
}

#[async_trait]
impl ObjectStore for FsBlobStore {
    async fn create_bucket(
        &self,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
    ) -> Result<BucketMeta> {
        Self::validate_name(name)?;
        self.meta.create_bucket(name, project_id, public_read).await
    }

    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>> {
        self.meta.get_bucket(name).await
    }

    async fn list_buckets(&self) -> Result<Vec<BucketMeta>> {
        self.meta.list_buckets().await
    }

    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()> {
        self.meta.set_bucket_public(name, public_read).await
    }

    async fn delete_bucket(&self, name: &str) -> Result<()> {
        let existing = self.meta.list_objects_meta(name, None, 1).await?;
        if !existing.is_empty() {
            return Err(FerroError::Conflict(format!(
                "bucket '{name}' is not empty"
            )));
        }
        self.meta.delete_bucket(name).await
    }

    async fn put_stream(
        &self,
        bucket: &str,
        key: &str,
        mut stream: ByteStream,
        content_type: &str,
        custom_meta: Option<String>,
    ) -> Result<ObjectMeta> {
        Self::validate_name(bucket)?;
        let key_norm = Self::normalize_key(key)?;
        if self.meta.get_bucket(bucket).await?.is_none() {
            return Err(FerroError::NotFound(format!("bucket '{bucket}'")));
        }

        let tmp_path = self
            .blobs_dir
            .join("_tmp")
            .join(format!("upload_{}", Uuid::new_v4().simple()));
        let mut tmp_file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("create temp blob: {e}")))?;

        let mut hasher = Sha256::new();
        let mut total_size = 0u64;

        let write_res: std::result::Result<(), FerroError> = async {
            while let Some(chunk) = stream.next().await {
                let data = chunk.map_err(|e| FerroError::Internal(anyhow!("read body stream: {e}")))?;
                total_size += data.len() as u64;
                hasher.update(&data);
                tmp_file
                    .write_all(&data)
                    .await
                    .map_err(|e| FerroError::Internal(anyhow!("write temp blob: {e}")))?;
            }
            tmp_file
                .flush()
                .await
                .map_err(|e| FerroError::Internal(anyhow!("flush temp blob: {e}")))?;
            Ok(())
        }
        .await;

        if let Err(e) = write_res {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(e);
        }
        drop(tmp_file);

        let sha_hex = hex::encode(hasher.finalize());
        let rel_path = format!("{}/{}/{}", &sha_hex[0..2], &sha_hex[2..4], sha_hex);
        let final_path = self.disk_path(&rel_path);
        if let Some(parent) = final_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| FerroError::Internal(anyhow!("create blob subdir: {e}")))?;
        }
        tokio::fs::rename(&tmp_path, &final_path)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("finalize blob rename: {e}")))?;

        let meta = ObjectMeta {
            bucket: bucket.to_string(),
            key: key_norm,
            sha256: sha_hex,
            blob_path: rel_path.clone(),
            size: total_size,
            content_type: content_type.to_string(),
            custom_meta,
            updated_at: Utc::now().timestamp(),
        };

        if let Err(e) = self
            .meta
            .upsert_object_meta(meta.clone(), self.gc_grace_secs)
            .await
        {
            let _ = self
                .meta
                .schedule_orphan_gc(&rel_path, self.gc_grace_secs)
                .await;
            return Err(e);
        }
        Ok(meta)
    }

    async fn head(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>> {
        let key_norm = Self::normalize_key(key)?;
        self.meta.get_object_meta(bucket, &key_norm).await
    }

    async fn get_stream(
        &self,
        bucket: &str,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<(ObjectMeta, ByteStream, u64, Option<(u64, u64)>)>> {
        let key_norm = Self::normalize_key(key)?;
        let Some(meta) = self.meta.get_object_meta(bucket, &key_norm).await? else {
            return Ok(None);
        };
        let path = self.disk_path(&meta.blob_path);
        let mut file = tokio::fs::File::open(&path)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("open blob {}: {e}", meta.blob_path)))?;

        if let Some(r) = range {
            if meta.size == 0 || r.start >= meta.size {
                return Err(FerroError::InvalidArgument(
                    "requested range not satisfiable".into(),
                ));
            }
            let end = r.end.unwrap_or(meta.size - 1).min(meta.size - 1);
            if end < r.start {
                return Err(FerroError::InvalidArgument(
                    "requested range end precedes start".into(),
                ));
            }
            let len = end - r.start + 1;
            file.seek(SeekFrom::Start(r.start))
                .await
                .map_err(|e| FerroError::Internal(anyhow!("seek blob: {e}")))?;
            let limited = file.take(len);
            let stream: ByteStream = Box::pin(ReaderStream::with_capacity(limited, CHUNK_SIZE));
            Ok(Some((meta, stream, len, Some((r.start, end)))))
        } else {
            let size = meta.size;
            let stream: ByteStream = Box::pin(ReaderStream::with_capacity(file, CHUNK_SIZE));
            Ok(Some((meta, stream, size, None)))
        }
    }

    async fn delete(&self, bucket: &str, key: &str) -> Result<bool> {
        let key_norm = Self::normalize_key(key)?;
        self.meta
            .delete_object_meta(bucket, &key_norm, self.gc_grace_secs)
            .await
    }

    async fn list(&self, bucket: &str, prefix: Option<&str>, limit: usize) -> Result<Vec<ObjectMeta>> {
        self.meta.list_objects_meta(bucket, prefix, limit).await
    }

    async fn run_gc_batch(&self, limit: usize) -> Result<u64> {
        let now = Utc::now().timestamp();
        let due = self.meta.list_due_blob_gc(now, limit).await?;
        let mut removed = 0u64;
        for rel_path in due {
            if self.meta.claim_blob_gc_delete(&rel_path).await? {
                let full = self.disk_path(&rel_path);
                if tokio::fs::remove_file(&full).await.is_ok() {
                    removed += 1;
                }
            }
        }
        Ok(removed)
    }
}
