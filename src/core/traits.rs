use std::path::Path;

use async_trait::async_trait;
use bytes::Bytes;

use crate::core::model::{
    AiProviderRecord, AiUsageRecord, AlertWebhookRecord, ApiToken, BucketMeta, BucketOpts,
    ByteRange, ByteStream, CronJob, DbFtsIndexMeta, Deployment, EnqueueOpts, FerroError, Job,
    JobOutcome, JobStatus, KvKeyMeta, KvNamespaceMeta, KvNamespaceOpts, KvValue, LogEntry,
    LogQuery, ObjectMeta, Project, ProjectOpts, QueryResult, QueueConfig, QueueStats, RequestCtx,
    Result, Route, RouteEntry, SearchFieldSchema, SearchIndexMeta, SearchIndexOpts,
    SearchQueryInput, SearchQueryResult, SqlValue, TargetEnv, TokenScope, UserDbMeta, UserDbOpts,
    WorkerHttpRequest, WorkerHttpResponse,
};

#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn create_project(
        &self,
        name: &str,
        health_check_path: Option<&str>,
        group: Option<&str>,
    ) -> Result<Project> {
        self.create_project_with_opts(
            name,
            ProjectOpts {
                group: group.map(str::to_owned),
                health_check_path: health_check_path.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }
    async fn create_project_with_opts(&self, name: &str, opts: ProjectOpts) -> Result<Project>;
    async fn update_project(&self, project: &Project) -> Result<()>;
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
    async fn previous_production_deployment(&self, project_id: &str) -> Result<Option<Deployment>>;
    async fn set_preview_deployment(&self, project_id: &str, deployment_id: &str) -> Result<()>;

    async fn upsert_route(&self, route: Route, expected_owner: Option<&str>) -> Result<()>;
    async fn get_route(&self, host: &str) -> Result<Option<Route>>;
    async fn delete_route(&self, host: &str, expected_owner: Option<&str>) -> Result<bool>;
    async fn list_routes(&self) -> Result<Vec<Route>>;
    async fn load_route_entries(&self) -> Result<Vec<RouteEntry>>;
    async fn list_referenced_artifact_paths(&self) -> Result<Vec<(String, String)>>;

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
    ) -> Result<()> {
        self.register_user_db_with_opts(
            id,
            name,
            UserDbOpts {
                group: group.map(str::to_owned),
                project_id: project_id.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }
    async fn register_user_db_with_opts(
        &self,
        id: &str,
        name: &str,
        opts: UserDbOpts,
    ) -> Result<()>;
    async fn update_user_db(&self, meta: &UserDbMeta) -> Result<()>;
    async fn delete_user_db(&self, id: &str) -> Result<()>;
    async fn get_user_db(&self, id_or_name: &str) -> Result<Option<UserDbMeta>>;
    async fn list_user_dbs(&self) -> Result<Vec<UserDbMeta>>;

    async fn create_kv_namespace(
        &self,
        name: &str,
        project_id: Option<&str>,
        group: Option<&str>,
    ) -> Result<KvNamespaceMeta> {
        self.create_kv_namespace_with_opts(
            name,
            KvNamespaceOpts {
                group: group.map(str::to_owned),
                project_id: project_id.map(str::to_owned),
                ..Default::default()
            },
        )
        .await
    }
    async fn create_kv_namespace_with_opts(
        &self,
        name: &str,
        opts: KvNamespaceOpts,
    ) -> Result<KvNamespaceMeta>;
    async fn update_kv_namespace(&self, meta: &KvNamespaceMeta) -> Result<()>;
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

    async fn upsert_ai_provider(&self, rec: AiProviderRecord) -> Result<AiProviderRecord> {
        let _ = rec;
        Err(FerroError::NotImplemented("ai provider store".into()))
    }
    async fn get_ai_provider(&self, name: &str) -> Result<Option<AiProviderRecord>> {
        let _ = name;
        Ok(None)
    }
    async fn list_ai_providers(&self) -> Result<Vec<AiProviderRecord>> {
        Ok(Vec::new())
    }
    async fn delete_ai_provider(&self, name: &str) -> Result<bool> {
        let _ = name;
        Ok(false)
    }
    async fn record_ai_usage(&self, rec: &AiUsageRecord) -> Result<()> {
        let _ = rec;
        Ok(())
    }
    async fn list_ai_usage(
        &self,
        provider_name: Option<&str>,
        project_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AiUsageRecord>> {
        let _ = (provider_name, project_id, limit);
        Ok(Vec::new())
    }
    async fn aggregate_ai_usage(
        &self,
        provider_name: Option<&str>,
        project_id: Option<&str>,
        month_start_ms: i64,
    ) -> Result<(usize, usize, u64, u64, u64, u64)> {
        let _ = (provider_name, project_id, month_start_ms);
        Ok((0, 0, 0, 0, 0, 0))
    }
    async fn cleanup_ai_usage_before(&self, cutoff_ms: i64) -> Result<u64> {
        let _ = cutoff_ms;
        Ok(0)
    }
    async fn sum_ai_tokens_since(&self, provider_name: &str, since_ms: i64) -> Result<u64> {
        let _ = (provider_name, since_ms);
        Ok(0)
    }

    async fn upsert_alert_webhook(&self, rec: AlertWebhookRecord) -> Result<AlertWebhookRecord> {
        let _ = rec;
        Err(FerroError::NotImplemented("alert webhook store".into()))
    }
    async fn get_alert_webhook(&self, id: &str) -> Result<Option<AlertWebhookRecord>> {
        let _ = id;
        Ok(None)
    }
    async fn list_alert_webhooks(&self) -> Result<Vec<AlertWebhookRecord>> {
        Ok(Vec::new())
    }
    async fn delete_alert_webhook(&self, id: &str) -> Result<bool> {
        let _ = id;
        Ok(false)
    }
    async fn record_alert_webhook_delivery(
        &self,
        id: &str,
        status: Option<u16>,
        error: Option<&str>,
        triggered_at: i64,
    ) -> Result<()> {
        let _ = (id, status, error, triggered_at);
        Ok(())
    }

    async fn get_platform_setting(&self, key: &str) -> Result<Option<String>> {
        let _ = key;
        Ok(None)
    }
    async fn set_platform_setting(&self, key: &str, value_json: &str) -> Result<()> {
        let _ = (key, value_json);
        Ok(())
    }
    async fn delete_platform_setting(&self, key: &str) -> Result<()> {
        let _ = key;
        Ok(())
    }
}

#[async_trait]
pub trait BlobMetaStore: Send + Sync {
    async fn create_bucket(
        &self,
        name: &str,
        project_id: Option<&str>,
        public_read: bool,
        group: Option<&str>,
    ) -> Result<BucketMeta> {
        self.create_bucket_with_opts(
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
    async fn create_bucket_with_opts(&self, name: &str, opts: BucketOpts) -> Result<BucketMeta>;
    async fn update_bucket(&self, meta: &BucketMeta) -> Result<()>;
    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>>;
    async fn list_buckets(&self) -> Result<Vec<BucketMeta>>;
    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()>;
    async fn delete_bucket(&self, name: &str) -> Result<()>;
    async fn total_bucket_bytes(&self, bucket: &str) -> Result<u64>;
    async fn upsert_object_meta(&self, meta: ObjectMeta, gc_grace_secs: i64) -> Result<()>;
    async fn get_object_meta(&self, bucket: &str, key: &str) -> Result<Option<ObjectMeta>>;
    async fn delete_object_meta(&self, bucket: &str, key: &str, gc_grace_secs: i64)
        -> Result<bool>;
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
    async fn count_keys(&self, ns: &str) -> Result<usize> {
        Ok(self.list(ns, None, 1000).await?.len())
    }
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
    async fn update_queue(&self, cfg: QueueConfig) -> Result<()>;
    async fn get_queue(&self, name: &str) -> Result<Option<QueueConfig>>;
    async fn list_queues(&self) -> Result<Vec<QueueConfig>>;
    async fn delete_queue(&self, name: &str) -> Result<()>;
    async fn enqueue(&self, queue: &str, payload: Bytes, opts: EnqueueOpts) -> Result<i64>;
    async fn claim_batch(&self, queue: &str, limit: usize, lease_secs: i64) -> Result<Vec<Job>>;
    async fn ack_batch(&self, job_ids: &[i64]) -> Result<()>;
    async fn nack(
        &self,
        job_id: i64,
        retry_after_secs: u64,
        err: &str,
    ) -> Result<Option<JobStatus>>;
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
    ) -> Result<BucketMeta> {
        self.create_bucket_with_opts(
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
    async fn create_bucket_with_opts(&self, name: &str, opts: BucketOpts) -> Result<BucketMeta>;
    async fn update_bucket(&self, meta: &BucketMeta) -> Result<()>;
    async fn get_bucket(&self, name: &str) -> Result<Option<BucketMeta>>;
    async fn list_buckets(&self) -> Result<Vec<BucketMeta>>;
    async fn set_bucket_public(&self, name: &str, public_read: bool) -> Result<()>;
    async fn delete_bucket(&self, name: &str) -> Result<()>;
    async fn total_bucket_bytes(&self, bucket: &str) -> Result<u64>;
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
    async fn list(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ObjectMeta>>;
    async fn list_page(
        &self,
        bucket: &str,
        prefix: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<ObjectMeta>, Option<String>)>;
    async fn run_gc_batch(&self, limit: usize) -> Result<u64>;
}

pub type RowVisitor<'a> = dyn FnMut(&[String], Vec<serde_json::Value>) + Send + 'a;

#[async_trait]
pub trait DbManager: Send + Sync {
    async fn create_db(&self, db_id: &str) -> Result<()>;
    async fn delete_db(&self, db_id: &str) -> Result<()>;
    async fn query(&self, db_id: &str, sql: &str, params: Vec<SqlValue>) -> Result<QueryResult>;
    async fn visit_rows(&self, db_id: &str, sql: &str, visitor: &mut RowVisitor<'_>) -> Result<()> {
        let result = self.query(db_id, sql, Vec::new()).await?;
        for row in result.rows {
            visitor(&result.columns, row);
        }
        Ok(())
    }
    async fn execute_batch(&self, db_id: &str, stmts: Vec<(String, Vec<SqlValue>)>) -> Result<u64>;
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
    async fn delete_project_artifacts(&self, project_id: &str) -> Result<()> {
        let _ = project_id;
        Ok(())
    }
    async fn gc_unreferenced_artifacts(
        &self,
        referenced_paths: &std::collections::HashSet<String>,
        grace_secs: u64,
    ) -> Result<usize> {
        let _ = (referenced_paths, grace_secs);
        Ok(0)
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
    ) -> Result<SearchIndexMeta> {
        self.create_index_with_opts(
            name,
            SearchIndexOpts {
                project_id: project_id.map(str::to_owned),
                tokenizer: Some(tokenizer.to_owned()),
                schema,
                ..Default::default()
            },
        )
        .await
    }
    async fn create_index_with_opts(
        &self,
        name: &str,
        opts: SearchIndexOpts,
    ) -> Result<SearchIndexMeta>;
    async fn update_index(&self, meta: &SearchIndexMeta) -> Result<()>;
    async fn get_index(&self, id_or_name: &str) -> Result<Option<SearchIndexMeta>>;
    async fn list_indexes(&self) -> Result<Vec<SearchIndexMeta>>;
    async fn delete_index(&self, id: &str) -> Result<()>;
    async fn commit_index(&self, id: &str) -> Result<SearchIndexMeta>;
    async fn list_docs(&self, index_id: &str) -> Result<Vec<serde_json::Value>>;
    async fn list_docs_page(
        &self,
        index_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<serde_json::Value>> {
        Ok(self
            .list_docs(index_id)
            .await?
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect())
    }
    async fn upsert_doc(&self, index_id: &str, doc: serde_json::Value)
        -> Result<serde_json::Value>;
    async fn delete_doc(&self, index_id: &str, doc_id: &str) -> Result<bool>;
    async fn search(&self, index_id: &str, input: SearchQueryInput) -> Result<SearchQueryResult>;

    async fn checkpoint(&self) -> Result<()> {
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        let _ = dest_path;
        Ok(())
    }
}
