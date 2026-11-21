use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;

use crate::core::model::{
    ApiToken, BucketMeta, ByteRange, ByteStream, CronJob, DbFtsIndexMeta, Deployment, EnqueueOpts,
    Job, JobOutcome, JobStatus, KvKeyMeta, KvNamespaceMeta, KvValue, LogEntry, LogQuery,
    ObjectMeta, Project, QueryResult, QueueConfig, QueueStats, RequestCtx, Result, Route,
    RouteEntry, SearchFieldSchema, SearchIndexMeta, SearchQueryInput, SearchQueryResult,
    SqlValue, TargetEnv, TokenScope, UserDbMeta, WorkerHttpRequest, WorkerHttpResponse,
};

#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn create_project(
        &self,
        name: &str,
        health_check_path: Option<&str>,
        group: Option<&str>,
    ) -> Result<Project>;
    async fn get_project(&self, id_or_name: &str) -> Result<Option<Project>>;
    async fn list_projects(&self) -> Result<Vec<Project>>;
    async fn delete_project(&self, id: &str) -> Result<()>;

    async fn create_deployment(
        &self,
        deployment: Deployment,
        target_env: Option<TargetEnv>,
    ) -> Result<Deployment>;
    async fn get_deployment(&self, id: &str) -> Result<Option<Deployment>>;
    async fn list_deployments(&self, project_id: &str) -> Result<Vec<Deployment>>;
    async fn set_active_deployment(&self, project_id: &str, deployment_id: &str) -> Result<()>;
    async fn set_preview_deployment(&self, project_id: &str, deployment_id: &str) -> Result<()>;

    async fn upsert_route(&self, route: Route, expected_owner: Option<&str>) -> Result<()>;
    async fn get_route(&self, host: &str) -> Result<Option<Route>>;
    async fn delete_route(&self, host: &str, expected_owner: Option<&str>) -> Result<bool>;
    async fn list_routes(&self) -> Result<Vec<Route>>;
    async fn load_route_entries(&self) -> Result<Vec<RouteEntry>>;

    async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        let _ = dest_path;
        Ok(())
    }
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn set_secret(
        &self,
        project_id: &str,
        key: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<()>;
    async fn get_secret(&self, project_id: &str, key: &str) -> Result<Option<(Vec<u8>, Vec<u8>)>>;
    async fn list_secret_keys(&self, project_id: &str) -> Result<Vec<String>>;
    async fn delete_secret(&self, project_id: &str, key: &str) -> Result<()>;
    async fn delete_project_secrets(&self, project_id: &str) -> Result<()>;
}

#[async_trait]
pub trait TokenStore: Send + Sync {
    async fn create_token(
        &self,
        name: &str,
        token_hash: &str,
        scope: TokenScope,
        expires_at: Option<i64>,
    ) -> Result<ApiToken>;
    async fn verify_token_hash(&self, token_hash: &str, now_secs: i64) -> Result<Option<ApiToken>>;
    async fn list_tokens(&self) -> Result<Vec<ApiToken>>;
    async fn revoke_token(&self, id: &str) -> Result<()>;
}

#[async_trait]
pub trait RegistryStore: Send + Sync {
    async fn register_user_db(
        &self,
        id: &str,
        name: &str,
        project_id: Option<&str>,
        group: Option<&str>,
    ) -> Result<()>;
    async fn delete_user_db(&self, id: &str) -> Result<()>;
    async fn get_user_db(&self, id_or_name: &str) -> Result<Option<UserDbMeta>>;
    async fn list_user_dbs(&self) -> Result<Vec<UserDbMeta>>;

    async fn create_kv_namespace(
        &self,
        name: &str,
        project_id: Option<&str>,
        group: Option<&str>,
    ) -> Result<KvNamespaceMeta>;
    async fn get_kv_namespace(&self, name: &str) -> Result<Option<KvNamespaceMeta>>;
    async fn list_kv_namespaces(&self) -> Result<Vec<KvNamespaceMeta>>;
    async fn delete_kv_namespace(&self, name: &str) -> Result<()>;

    async fn create_cron_job(&self, job: CronJob) -> Result<CronJob>;
    async fn get_cron_job(&self, id: &str) -> Result<Option<CronJob>>;
    async fn list_cron_jobs(&self) -> Result<Vec<CronJob>>;
    async fn update_cron_job(&self, job: &CronJob) -> Result<()>;
    async fn delete_cron_job(&self, id: &str) -> Result<()>;

    async fn create_db_fts_index(&self, meta: DbFtsIndexMeta) -> Result<DbFtsIndexMeta>;
    async fn list_db_fts_indexes(&self, db_id: &str) -> Result<Vec<DbFtsIndexMeta>>;
    async fn delete_db_fts_index(&self, db_id: &str, name: &str) -> Result<()>;
}

#[async_trait]
pub trait BlobMetaStore: Send + Sync {
    async fn create_bucket(
        &self,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
        group: Option<&str>,
    ) -> Result<BucketMeta>;
    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>>;
    async fn list_buckets(&self) -> Result<Vec<BucketMeta>>;
    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()>;
    async fn delete_bucket(&self, name: &str) -> Result<()>;
    async fn upsert_object_meta(&self, meta: ObjectMeta, gc_grace_secs: i64) -> Result<()>;
    async fn get_object_meta(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>>;
    async fn delete_object_meta(
        &self,
        bucket: &str,
        key: &str,
        gc_grace_secs: i64,
    ) -> Result<bool>;
    async fn list_objects_meta(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectMeta>>;
    async fn list_objects_meta_page(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ObjectMeta>, Option<String>)>;
    async fn list_due_blob_gc(&self, now_secs: i64, limit: usize) -> Result<Vec<String>>;
    async fn claim_blob_gc_delete(&self, blob_path: &str) -> Result<bool>;
    async fn schedule_orphan_gc(&self, blob_path: &str, gc_grace_secs: i64) -> Result<()>;
}

#[async_trait]
pub trait KvStore: Send + Sync {
    async fn get(&self, ns: &str, key: &str) -> Result<Option<KvValue>>;
    async fn put(
        &self,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
        ephemeral: bool,
    ) -> Result<()>;
    async fn put_if_absent(
        &self,
        ns: &str,
        key: &str,
        val: Bytes,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
    ) -> Result<bool>;
    async fn incr(&self, ns: &str, key: &str, delta: i64, ttl_secs: Option<u64>) -> Result<i64>;
    async fn delete(&self, ns: &str, key: &str) -> Result<()>;
    async fn delete_namespace(&self, ns: &str) -> Result<()>;
    async fn list(&self, ns: &str, prefix: Option<&str>, limit: usize) -> Result<Vec<KvKeyMeta>>;
    async fn list_page(
        &self,
        ns: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<KvKeyMeta>, Option<String>)>;
    async fn cleanup_expired_batch(&self, batch_limit: usize) -> Result<u64>;

    async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        let _ = dest_path;
        Ok(())
    }
}

#[async_trait]
pub trait QueueStore: Send + Sync {
    async fn create_queue(&self, cfg: QueueConfig) -> Result<()>;
    async fn get_queue(&self, name: &str) -> Result<Option<QueueConfig>>;
    async fn list_queues(&self) -> Result<Vec<QueueConfig>>;
    async fn delete_queue(&self, name: &str) -> Result<()>;
    async fn enqueue(&self, queue: &str, payload: Bytes, opts: EnqueueOpts) -> Result<i64>;
    async fn claim_batch(&self, queue: &str, limit: usize, lease_secs: i64) -> Result<Vec<Job>>;
    async fn ack_batch(&self, job_ids: &[i64]) -> Result<()>;
    async fn nack(&self, job_id: i64, retry_after_secs: u64, err: &str) -> Result<Option<JobStatus>>;
    async fn mark_dead(&self, job_id: i64, err: &str) -> Result<()>;
    async fn reap_expired(&self) -> Result<u64>;
    async fn inspect(&self, queue: &str, status: JobStatus, limit: usize) -> Result<Vec<Job>>;
    async fn dlq_replay(&self, queue: &str, job_ids: Option<&[i64]>) -> Result<u64>;
    async fn purge(&self, queue: &str) -> Result<u64>;
    async fn cleanup_expired_batch(
        &self,
        pending_max_age_secs: i64,
        dlq_max_age_secs: i64,
        batch_limit: usize,
    ) -> Result<u64>;
    async fn stats(&self, queue: &str) -> Result<QueueStats>;

    async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        let _ = dest_path;
        Ok(())
    }
}

#[async_trait]
pub trait ObjectStore: Send + Sync {
    async fn create_bucket(
        &self,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
        group: Option<&str>,
    ) -> Result<BucketMeta>;
    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>>;
    async fn list_buckets(&self) -> Result<Vec<BucketMeta>>;
    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()>;
    async fn delete_bucket(&self, name: &str) -> Result<()>;
    async fn put_stream(
        &self,
        bucket: &str,
        key: &str,
        stream: ByteStream,
        content_type: &str,
        custom_meta: Option<String>,
    ) -> Result<ObjectMeta>;
    async fn head(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>>;
    async fn get_stream(
        &self,
        bucket: &str,
        key: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<(ObjectMeta, ByteStream, u64, Option<(u64, u64)>)>>;
    async fn delete(&self, bucket: &str, key: &str) -> Result<bool>;
    async fn list(&self, bucket: &str, prefix: Option<&str>, limit: usize) -> Result<Vec<ObjectMeta>>;
    async fn list_page(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ObjectMeta>, Option<String>)>;
    async fn run_gc_batch(&self, limit: usize) -> Result<u64>;
}

#[async_trait]
pub trait DbManager: Send + Sync {
    async fn create_db(&self, db_id: &str) -> Result<()>;
    async fn delete_db(&self, db_id: &str) -> Result<()>;
    async fn query(&self, db_id: &str, sql: &str, params: Vec<SqlValue>) -> Result<QueryResult>;
    async fn execute_batch(
        &self,
        db_id: &str,
        stmts: Vec<(String, Vec<SqlValue>)>,
    ) -> Result<u64>;
    async fn schema(&self, db_id: &str) -> Result<String>;
    async fn snapshot_to(&self, db_id: &str, dest_path: &Path) -> Result<()>;
    async fn checkpoint_all(&self) -> Result<()>;
}

#[async_trait]
pub trait LogStore: Send + Sync {
    fn emit(&self, entry: LogEntry);
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<LogEntry>;
    async fn query(&self, q: LogQuery) -> Result<Vec<LogEntry>>;
    async fn flush(&self) -> Result<()>;
    async fn cleanup_old_files(&self, retention_days: u32) -> Result<u64>;
}

/// Worker runtime abstraction (implemented by Wasmtime runtime).
#[async_trait]
pub trait WorkerRuntime: Send + Sync {
    async fn store_artifact(
        &self,
        project_id: &str,
        rel_path: &str,
        artifact: Bytes,
    ) -> Result<(String, bool)>;
    async fn compile_and_load(&self, deployment: &Deployment, artifact: Bytes) -> Result<()>;
    async fn preload_deployment(&self, deployment: &Deployment) -> Result<()> {
        let _ = deployment;
        Ok(())
    }
    async fn handle_http(
        &self,
        ctx: RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse>;
    async fn handle_queue(&self, ctx: RequestCtx, jobs: Vec<Job>) -> Result<Vec<JobOutcome>>;
    async fn handle_cron(&self, ctx: RequestCtx, expr: &str) -> Result<()>;
}

#[async_trait]
pub trait SearchStore: Send + Sync {
    async fn create_index(
        &self,
        name: &str,
        project_id: Option<&str>,
        tokenizer: &str,
        schema: Option<Vec<SearchFieldSchema>>,
    ) -> Result<SearchIndexMeta>;
    async fn get_index(&self, id_or_name: &str) -> Result<Option<SearchIndexMeta>>;
    async fn list_indexes(&self) -> Result<Vec<SearchIndexMeta>>;
    async fn delete_index(&self, id: &str) -> Result<()>;
    async fn commit_index(&self, id: &str) -> Result<SearchIndexMeta>;
    async fn list_docs(&self, index_id: &str) -> Result<Vec<serde_json::Value>>;
    async fn upsert_doc(
        &self,
        index_id: &str,
        doc: serde_json::Value,
    ) -> Result<serde_json::Value>;
    async fn delete_doc(&self, index_id: &str, doc_id: &str) -> Result<bool>;
    async fn search(
        &self,
        index_id: &str,
        input: SearchQueryInput,
    ) -> Result<SearchQueryResult>;

    async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        let _ = dest_path;
        Ok(())
    }
}
