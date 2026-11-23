use std::collections::HashMap;
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, patch, post, put},
    Json, Router,
};
use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use futures_util::stream::Stream;
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::core::model::{
    AiProviderOpts, AlertWebhookOpts, AutoBackupConfig, Bindings, BucketOpts, CronJobOpts,
    EnqueueOpts, FerroError, JobStatus, KvNamespaceOpts, LogQuery, PlatformSettingsPatch,
    ProjectOpts, QueueConfig, RouteOpts, SearchFieldSchema, SearchIndexOpts, SearchQueryInput,
    SqlValue, TargetEnv, TokenScope, UserDbOpts, DEFAULT_KV_TTL_SECS,
};
use crate::core::service::{AiProxyResult, CreateDeploymentInput, PlatformService};
use crate::gateway::guard::{authenticate_caller, ApiError};
use crate::update;

#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusResp {
    status: &'static str,
    version: &'static str,
    profile: String,
    actor: String,
    scope: String,
    instance_name: Option<String>,
    panel_domain: Option<String>,
    base_domain: Option<String>,
    projects: usize,
    routes: usize,
    queues: usize,
    buckets: usize,
    dbs: usize,
    kv_namespaces: usize,
    search_indexes: usize,
    ai_providers: usize,
    alert_webhooks: usize,
}

#[derive(Debug, Deserialize)]
struct CreateProjectReq {
    group: Option<String>,
    name: String,
    description: Option<String>,
    health_check_path: Option<String>,
    max_concurrency: Option<i64>,
    timeout_ms: Option<u64>,
    wasm_max_memory_mb: Option<usize>,
    allow_private_outbound: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateProjectReq {
    group: Option<String>,
    description: Option<String>,
    health_check_path: Option<String>,
    max_concurrency: Option<i64>,
    timeout_ms: Option<u64>,
    wasm_max_memory_mb: Option<usize>,
    allow_private_outbound: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateDeploymentReq {
    artifact_path: Option<String>,
    #[serde(default)]
    artifact_base64: Option<String>,
    assets_bucket: Option<String>,
    assets_prefix: Option<String>,
    #[serde(default)]
    spa_fallback: bool,
    #[serde(default)]
    bindings: Bindings,
    #[serde(default)]
    crons: Vec<String>,
    target_env: Option<TargetEnv>,
}

#[derive(Debug, Deserialize)]
struct InvokeProjectReq {
    method: Option<String>,
    #[serde(default = "default_invoke_path")]
    path: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    body: Option<String>,
    target_env: Option<TargetEnv>,
}

fn default_invoke_path() -> String {
    "/".to_string()
}

#[derive(Debug, Deserialize)]
struct PromoteReq {
    deployment_id: Option<String>,
    #[serde(default)]
    verify_health: bool,
}

#[derive(Debug, Deserialize)]
struct UpsertRouteReq {
    host: String,
    project: String,
    #[serde(default = "default_prod")]
    target_env: TargetEnv,
    proxy_prefix: Option<String>,
    proxy_upstream: Option<String>,
    rate_limit_rpm: Option<u32>,
    basic_auth_user: Option<String>,
    basic_auth_password: Option<String>,
    clear_basic_auth: Option<bool>,
    canary_deployment_id: Option<String>,
    canary_weight: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct UpdateRouteReq {
    project: Option<String>,
    target_env: Option<TargetEnv>,
    proxy_prefix: Option<String>,
    proxy_upstream: Option<String>,
    rate_limit_rpm: Option<u32>,
    basic_auth_user: Option<String>,
    basic_auth_password: Option<String>,
    clear_basic_auth: Option<bool>,
    canary_deployment_id: Option<String>,
    canary_weight: Option<u8>,
}

fn default_prod() -> TargetEnv {
    TargetEnv::Prod
}

#[derive(Debug, Deserialize)]
struct SetSecretReq {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct CreateTokenReq {
    name: String,
    scope: String,
    expires_in_days: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CreateKvNamespaceReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    default_ttl_secs: Option<u64>,
    #[serde(default)]
    default_ephemeral: bool,
    max_value_bytes: Option<usize>,
    max_keys: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct UpdateKvNamespaceReq {
    group: Option<String>,
    description: Option<String>,
    default_ttl_secs: Option<u64>,
    default_ephemeral: Option<bool>,
    max_value_bytes: Option<usize>,
    max_keys: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct KvPutReq {
    value: String,
    metadata: Option<String>,
    ttl_secs: Option<u64>,
    #[serde(default)]
    ephemeral: bool,
}

#[derive(Debug, Deserialize)]
struct KvIncrReq {
    delta: i64,
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct KvListQuery {
    prefix: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct CreateQueueReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    consumer_project_id: Option<String>,
    description: Option<String>,
    max_concurrency: Option<i64>,
    rate_limit_per_sec: Option<i64>,
    batch_size: Option<i64>,
    lease_secs: Option<i64>,
    max_attempts: Option<i64>,
    retry_backoff_secs: Option<i64>,
    dlq_retention_secs: Option<i64>,
    #[serde(default)]
    ephemeral: bool,
    max_pending_jobs: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct EnqueueReq {
    payload: serde_json::Value,
    delay_secs: Option<u64>,
    priority: Option<i32>,
    dedup_key: Option<String>,
    max_attempts: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct InspectQueueQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ReplayDlqReq {
    job_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
struct CreateBucketReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    #[serde(default)]
    public_read: bool,
    max_object_size_mb: Option<usize>,
    quota_mb: Option<usize>,
    allowed_content_types: Option<String>,
    cache_control: Option<String>,
    cors_origins: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateBucketReq {
    group: Option<String>,
    description: Option<String>,
    public_read: Option<bool>,
    max_object_size_mb: Option<usize>,
    quota_mb: Option<usize>,
    allowed_content_types: Option<String>,
    cache_control: Option<String>,
    cors_origins: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PresignReq {
    key: String,
    op: String,
    #[serde(default = "default_presign_ttl")]
    expires_secs: u64,
}

fn default_presign_ttl() -> u64 {
    300
}

#[derive(Debug, Deserialize)]
struct CreateDbReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    init_sql: Option<String>,
    max_rows: Option<usize>,
    max_size_mb: Option<usize>,
    foreign_keys: Option<bool>,
    read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateDbReq {
    group: Option<String>,
    description: Option<String>,
    max_rows: Option<usize>,
    max_size_mb: Option<usize>,
    foreign_keys: Option<bool>,
    read_only: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DbQueryReq {
    sql: String,
    #[serde(default)]
    params: Vec<SqlValue>,
}

#[derive(Debug, Deserialize)]
struct DbExecuteReq {
    statements: Vec<DbStatementItem>,
}

#[derive(Debug, Deserialize)]
struct DbStatementItem {
    sql: String,
    #[serde(default)]
    params: Vec<SqlValue>,
}

pub fn router() -> Router<PlatformService> {
    Router::new()
        .route("/openapi.json", get(api_openapi))
        .route("/llms.txt", get(api_llms_txt))
        .route("/status", get(status))
        .route("/metrics", get(metrics))
        .route("/metrics/prometheus", get(metrics_prometheus))
        .route("/resource-groups", get(list_resource_groups))
        .route("/system/info", get(system_info))
        .route("/system/update/check", get(system_update_check))
        .route("/system/releases", get(system_releases))
        .route("/system/update/apply", post(system_update_apply))
        .route(
            "/settings",
            get(get_platform_settings).patch(update_platform_settings),
        )
        .route("/settings/reset", post(reset_platform_settings))
        .route("/settings/dns-check", post(settings_dns_check))
        .route("/settings/alerts/test", post(settings_test_alert))
        .route(
            "/alerts/webhooks",
            get(alert_webhooks_list).post(alert_webhooks_create),
        )
        .route(
            "/alerts/webhooks/{id}",
            patch(alert_webhooks_update).delete(alert_webhooks_delete),
        )
        .route("/alerts/webhooks/{id}/test", post(alert_webhooks_test))
        .route("/realtime/channels", get(realtime_list_channels))
        .route("/backups", get(list_backups).post(create_backup))
        .route(
            "/backups/schedule",
            get(get_backup_schedule).patch(update_backup_schedule),
        )
        .route("/backups/{name}", delete(delete_backup))
        .route("/backups/{name}/download", get(download_backup))
        .route("/backups/{name}/restore", post(restore_backup))
        .route("/crons", get(cron_list).post(cron_create))
        .route("/crons/{id}", patch(cron_update).delete(cron_delete))
        .route("/crons/{id}/run", post(cron_run))
        .merge(runtime_routes("/apps"))
        .merge(runtime_routes("/runtimes"))
        .merge(runtime_routes("/projects"))
        .route("/routes", get(list_routes).post(upsert_route))
        .route("/routes/{host}", patch(update_route).delete(delete_route))
        .route("/tokens", get(list_tokens).post(create_token))
        .route("/tokens/{id}", delete(revoke_token))
        .route("/kv", get(kv_list_namespaces).post(kv_create_namespace))
        .route(
            "/kv/{ns}",
            get(kv_list)
                .patch(kv_update_namespace)
                .delete(kv_delete_namespace),
        )
        .route("/kv/{ns}/{key}", get(kv_get).put(kv_put).delete(kv_delete))
        .route("/kv/{ns}/{key}/incr", post(kv_incr))
        .route("/queues", get(queue_list).post(queue_create))
        .route("/queues/{name}", patch(queue_update).delete(queue_delete))
        .route("/queues/{name}/enqueue", post(queue_enqueue))
        .route("/queues/{name}/stats", get(queue_stats))
        .route("/queues/{name}/inspect", get(queue_inspect))
        .route("/queues/{name}/dlq/replay", post(queue_dlq_replay))
        .route("/queues/{name}/purge", post(queue_purge))
        .route("/buckets", get(bucket_list).post(bucket_create))
        .route(
            "/buckets/{name}",
            patch(bucket_update).delete(bucket_delete),
        )
        .route("/buckets/{name}/presign", post(bucket_presign))
        .route("/dbs", get(db_list).post(db_create))
        .route("/dbs/{id}", patch(db_update).delete(db_delete))
        .route("/dbs/{id}/schema", get(db_schema))
        .route("/dbs/{id}/query", post(db_query))
        .route("/dbs/{id}/execute", post(db_execute))
        .route("/dbs/{id}/fts", get(db_fts_list).post(db_fts_create))
        .route("/dbs/{id}/fts/{name}", delete(db_fts_drop))
        .route("/dbs/{id}/fts/{name}/query", post(db_fts_query))
        .route(
            "/search",
            get(search_list_indexes).post(search_create_index),
        )
        .route(
            "/search/{id}",
            patch(search_update_index).delete(search_delete_index),
        )
        .route("/search/{id}/commit", post(search_commit_index))
        .route(
            "/search/{id}/docs",
            get(search_list_docs).post(search_upsert_doc),
        )
        .route("/search/{id}/docs/{doc_id}", delete(search_delete_doc))
        .route("/search/{id}/query", post(search_query_index))
        .route("/logs", get(logs_query))
        .route("/logs/stream", get(logs_stream))
        .route(
            "/ai/providers",
            get(ai_list_providers).post(ai_create_provider),
        )
        .route(
            "/ai/providers/{name}",
            patch(ai_update_provider).delete(ai_delete_provider),
        )
        .route("/ai/usage", get(ai_usage_summary))
        .route("/ai/cache/clear", post(ai_clear_cache))
}

async fn list_resource_groups(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (runtimes, dbs, namespaces, queues, buckets, search_indexes) = tokio::try_join!(
        svc.list_projects(&caller),
        svc.list_user_dbs(&caller),
        svc.list_kv_namespaces(&caller),
        svc.queue_list(&caller),
        svc.list_buckets(&caller),
        svc.list_search_indexes(&caller),
    )
    .map_err(ApiError)?;
    let groups: std::collections::BTreeSet<_> = runtimes
        .into_iter()
        .map(|r| r.group)
        .chain(dbs.into_iter().map(|r| r.group))
        .chain(namespaces.into_iter().map(|r| r.group))
        .chain(queues.into_iter().map(|r| r.group))
        .chain(buckets.into_iter().map(|r| r.group))
        .chain(search_indexes.into_iter().map(|r| r.group))
        .flatten()
        .collect();
    Ok(Json(groups))
}

fn runtime_routes(prefix: &str) -> Router<PlatformService> {
    Router::new()
        .route(prefix, get(list_projects).post(create_project))
        .route(
            &format!("{prefix}/{{id}}"),
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .route(
            &format!("{prefix}/{{id}}/deployments"),
            get(list_deployments).post(create_deployment),
        )
        .route(
            &format!("{prefix}/{{id}}/deployments/{{deployment_id}}/env"),
            post(create_deployment_with_env),
        )
        .route(
            &format!("{prefix}/{{id}}/artifacts/{{*path}}"),
            put(upload_artifact).layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route(&format!("{prefix}/{{id}}/invoke"), post(invoke_project))
        .route(
            &format!("{prefix}/{{id}}/promote"),
            post(promote_deployment),
        )
        .route(
            &format!("{prefix}/{{id}}/rollback"),
            post(rollback_deployment),
        )
        .route(
            &format!("{prefix}/{{id}}/secrets"),
            get(list_secrets).post(set_secret),
        )
        .route(
            &format!("{prefix}/{{id}}/secrets/{{key}}"),
            delete(delete_secret),
        )
}

async fn api_openapi() -> impl IntoResponse {
    Json(crate::web::openapi::openapi_document())
}

async fn api_llms_txt() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        crate::web::openapi::llms_txt(),
    )
}

async fn status(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (projects, routes, queues, buckets, dbs, kv_namespaces, search_indexes, ai_providers) =
        tokio::try_join!(
            svc.list_projects(&caller),
            svc.list_routes(&caller),
            svc.queue_list(&caller),
            svc.list_buckets(&caller),
            svc.list_user_dbs(&caller),
            svc.list_kv_namespaces(&caller),
            svc.list_search_indexes(&caller),
            svc.list_ai_providers(&caller),
        )
        .map_err(ApiError)?;
    let alert_webhooks = if caller.scope.allows_admin() {
        svc.list_alert_webhooks(&caller)
            .await
            .map(|l| l.len())
            .unwrap_or(0)
    } else {
        0
    };
    let eff = svc.effective_settings();
    Ok(Json(StatusResp {
        status: "ok",
        version: update::CURRENT_VERSION,
        profile: format!("{:?}", svc.config().profile),
        actor: caller.actor.clone(),
        scope: caller.scope.as_str(),
        instance_name: eff.instance_name,
        panel_domain: eff.panel_domain,
        base_domain: eff.base_domain,
        projects: projects.len(),
        routes: routes.len(),
        queues: queues.len(),
        buckets: buckets.len(),
        dbs: dbs.len(),
        kv_namespaces: kv_namespaces.len(),
        search_indexes: search_indexes.len(),
        ai_providers: ai_providers.len(),
        alert_webhooks,
    }))
}

#[derive(Debug, Serialize)]
struct MetricsResp {
    uptime_secs: u64,
    rss_bytes: u64,
    mem_total_bytes: u64,
    mem_avail_bytes: u64,
    mem_budget_bytes: u64,
    load_1m: f64,
    load_5m: f64,
    load_15m: f64,
    cpu_cores: usize,
    open_fds: usize,
    profile: String,
    timestamp_ms: i64,
    http_requests_2xx: u64,
    http_requests_3xx: u64,
    http_requests_4xx: u64,
    http_requests_5xx: u64,
    http_avg_latency_ms: f64,
    active_http_requests: u64,
    rate_limited_total: u64,
    worker_errors_total: u64,
    worker_oom_total: u64,
}

fn process_start_instant() -> &'static std::time::Instant {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now)
}

fn read_proc_rss_bytes() -> u64 {
    let Ok(content) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            if let Some(kb_str) = rest.split_whitespace().next() {
                if let Ok(kb) = kb_str.parse::<u64>() {
                    return kb.saturating_mul(1024);
                }
            }
        }
    }
    0
}

fn read_proc_meminfo() -> (u64, u64) {
    let Ok(content) = std::fs::read_to_string("/proc/meminfo") else {
        return (0, 0);
    };
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            if let Some(kb) = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
            {
                total = kb.saturating_mul(1024);
            }
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            if let Some(kb) = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
            {
                avail = kb.saturating_mul(1024);
            }
        }
    }
    (total, avail)
}

fn read_proc_loadavg() -> (f64, f64, f64) {
    let Ok(content) = std::fs::read_to_string("/proc/loadavg") else {
        return (0.0, 0.0, 0.0);
    };
    let mut parts = content.split_whitespace();
    let l1 = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let l5 = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let l15 = parts
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    (l1, l5, l15)
}

fn read_proc_open_fds() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|rd| rd.count())
        .unwrap_or(0)
}

async fn metrics(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let _caller = authenticate_caller(&svc, &headers, None).await?;
    let profile = format!("{:?}", svc.config().profile);
    let rss_bytes = read_proc_rss_bytes();
    let (mem_total_bytes, mem_avail_bytes) = read_proc_meminfo();
    let (load_1m, load_5m, load_15m) = read_proc_loadavg();
    let open_fds = read_proc_open_fds();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let detected_budget = crate::config::detect_system_memory_bytes();
    let mem_budget_bytes = if profile.eq_ignore_ascii_case("lowmem") {
        detected_budget.min(1024 * 1024 * 1024)
    } else if detected_budget > 0 {
        detected_budget
    } else if mem_total_bytes > 0 {
        mem_total_bytes
    } else {
        512 * 1024 * 1024
    };
    let gm = svc.metrics();
    let dur_cnt = gm
        .http_duration_ms_count
        .load(std::sync::atomic::Ordering::Relaxed);
    let dur_sum = gm
        .http_duration_ms_sum
        .load(std::sync::atomic::Ordering::Relaxed);
    let http_avg_latency_ms = if dur_cnt > 0 {
        ((dur_sum as f64 / dur_cnt as f64) * 100.0).round() / 100.0
    } else {
        0.0
    };
    Ok(Json(MetricsResp {
        uptime_secs: process_start_instant().elapsed().as_secs(),
        rss_bytes,
        mem_total_bytes,
        mem_avail_bytes,
        mem_budget_bytes,
        load_1m,
        load_5m,
        load_15m,
        cpu_cores,
        open_fds,
        profile,
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
        http_requests_2xx: gm
            .http_requests_2xx
            .load(std::sync::atomic::Ordering::Relaxed),
        http_requests_3xx: gm
            .http_requests_3xx
            .load(std::sync::atomic::Ordering::Relaxed),
        http_requests_4xx: gm
            .http_requests_4xx
            .load(std::sync::atomic::Ordering::Relaxed),
        http_requests_5xx: gm
            .http_requests_5xx
            .load(std::sync::atomic::Ordering::Relaxed),
        http_avg_latency_ms,
        active_http_requests: gm
            .active_http_requests
            .load(std::sync::atomic::Ordering::Relaxed),
        rate_limited_total: gm
            .rate_limited_total
            .load(std::sync::atomic::Ordering::Relaxed),
        worker_errors_total: gm
            .worker_errors_total
            .load(std::sync::atomic::Ordering::Relaxed),
        worker_oom_total: gm
            .worker_oom_total
            .load(std::sync::atomic::Ordering::Relaxed),
    }))
}

async fn metrics_prometheus(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    use std::fmt::Write as _;
    use std::sync::atomic::Ordering;

    let caller = authenticate_caller(&svc, &headers, None).await?;
    let uptime = process_start_instant().elapsed().as_secs();
    let rss = read_proc_rss_bytes();
    let open_fds = read_proc_open_fds();
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);

    let (projects, routes, queues) = tokio::try_join!(
        svc.list_projects(&caller),
        svc.list_routes(&caller),
        svc.queue_list(&caller),
    )
    .map_err(ApiError)?;

    let m = svc.metrics();
    let r2xx = m.http_requests_2xx.load(Ordering::Relaxed);
    let r3xx = m.http_requests_3xx.load(Ordering::Relaxed);
    let r4xx = m.http_requests_4xx.load(Ordering::Relaxed);
    let r5xx = m.http_requests_5xx.load(Ordering::Relaxed);
    let dur_sum = m.http_duration_ms_sum.load(Ordering::Relaxed);
    let dur_cnt = m.http_duration_ms_count.load(Ordering::Relaxed);
    let active = m.active_http_requests.load(Ordering::Relaxed);
    let rate_limited = m.rate_limited_total.load(Ordering::Relaxed);
    let worker_errs = m.worker_errors_total.load(Ordering::Relaxed);
    let worker_oom = m.worker_oom_total.load(Ordering::Relaxed);

    let mut out = String::with_capacity(2048);
    let _ = writeln!(
        out,
        "# HELP ferropod_uptime_seconds Process uptime in seconds.\n# TYPE ferropod_uptime_seconds counter\nferropod_uptime_seconds {uptime}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_process_rss_bytes Resident set size in bytes.\n# TYPE ferropod_process_rss_bytes gauge\nferropod_process_rss_bytes {rss}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_open_fds Number of open file descriptors.\n# TYPE ferropod_open_fds gauge\nferropod_open_fds {open_fds}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_cpu_cores Number of logical CPU cores.\n# TYPE ferropod_cpu_cores gauge\nferropod_cpu_cores {cpu_cores}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_http_requests_total Total HTTP requests handled by status class.\n# TYPE ferropod_http_requests_total counter\nferropod_http_requests_total{{status_class=\"2xx\"}} {r2xx}\nferropod_http_requests_total{{status_class=\"3xx\"}} {r3xx}\nferropod_http_requests_total{{status_class=\"4xx\"}} {r4xx}\nferropod_http_requests_total{{status_class=\"5xx\"}} {r5xx}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_http_request_duration_ms_sum Total accumulated HTTP request duration in milliseconds.\n# TYPE ferropod_http_request_duration_ms_sum counter\nferropod_http_request_duration_ms_sum {dur_sum}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_http_request_duration_ms_count Total count of HTTP requests timed.\n# TYPE ferropod_http_request_duration_ms_count counter\nferropod_http_request_duration_ms_count {dur_cnt}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_http_active_requests Currently in-flight HTTP requests.\n# TYPE ferropod_http_active_requests gauge\nferropod_http_active_requests {active}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_rate_limited_total Total requests rejected by concurrency or route rate limiting.\n# TYPE ferropod_rate_limited_total counter\nferropod_rate_limited_total {rate_limited}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_worker_errors_total Total Wasm Worker runtime invocation errors.\n# TYPE ferropod_worker_errors_total counter\nferropod_worker_errors_total {worker_errs}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_worker_oom_total Total Wasm Worker OOM traps.\n# TYPE ferropod_worker_oom_total counter\nferropod_worker_oom_total {worker_oom}"
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_projects_total Total apps accessible by caller.\n# TYPE ferropod_projects_total gauge\nferropod_projects_total {}",
        projects.len()
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_routes_total Total routes accessible by caller.\n# TYPE ferropod_routes_total gauge\nferropod_routes_total {}",
        routes.len()
    );
    let _ = writeln!(
        out,
        "# HELP ferropod_queues_total Total queues accessible by caller.\n# TYPE ferropod_queues_total gauge\nferropod_queues_total {}",
        queues.len()
    );

    Ok((
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    ))
}

#[derive(Debug, Deserialize)]
struct CreateBackupReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
struct BackupInfo {
    name: String,
    path: String,
    created_at_ms: i64,
    file_count: usize,
    size_bytes: u64,
    files: Vec<String>,
}

fn sanitize_backup_segment(raw: &str) -> Option<String> {
    let clean: String = raw
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(40)
        .collect();
    if clean.is_empty() {
        None
    } else {
        Some(clean)
    }
}

fn scan_backup_dir(root: &std::path::Path, name: String) -> Option<BackupInfo> {
    let dir = root.join(&name);
    let meta = std::fs::metadata(&dir).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let created_at_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let mut files = Vec::new();
    let mut size_bytes = 0u64;
    let mut stack = vec![(dir.clone(), String::new())];
    while let Some((cur, prefix)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&cur) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            let fname = entry.file_name().to_string_lossy().to_string();
            let rel = if prefix.is_empty() {
                fname
            } else {
                format!("{prefix}/{fname}")
            };
            if let Ok(m) = entry.metadata() {
                if m.is_dir() {
                    stack.push((path, rel));
                } else if m.is_file() {
                    size_bytes = size_bytes.saturating_add(m.len());
                    files.push(rel);
                }
            }
        }
    }
    files.sort();
    Some(BackupInfo {
        name,
        path: dir.to_string_lossy().to_string(),
        created_at_ms,
        file_count: files.len(),
        size_bytes,
        files,
    })
}

async fn list_backups(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let backups_root = svc.config().data_dir.join("backups");
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&backups_root) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(info) = scan_backup_dir(&backups_root, name) {
                items.push(info);
            }
        }
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_at_ms));
    Ok(Json(items))
}

async fn create_backup(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateBackupReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let base_name = match req.label.as_deref().and_then(sanitize_backup_segment) {
        Some(lbl) => format!("snap_{ts}_{lbl}"),
        None => format!("snap_{ts}"),
    };
    let backups_root = svc.config().data_dir.join("backups");
    let name = if backups_root.join(&base_name).exists() {
        format!(
            "{base_name}_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        )
    } else {
        base_name
    };
    let out_dir = backups_root.join(&name);
    svc.create_snapshot(&caller, &out_dir)
        .await
        .map_err(ApiError)?;
    let info = scan_backup_dir(&backups_root, name.clone()).unwrap_or(BackupInfo {
        name,
        path: out_dir.to_string_lossy().to_string(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        file_count: 0,
        size_bytes: 0,
        files: Vec::new(),
    });
    Ok((StatusCode::CREATED, Json(info)))
}

async fn delete_backup(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let target = validate_backup_dir(&svc, &name).await?;
    tokio::fs::remove_dir_all(&target)
        .await
        .map_err(|e| ApiError(FerroError::Internal(anyhow::anyhow!("delete backup: {e}"))))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn validate_backup_dir(
    svc: &PlatformService,
    name: &str,
) -> Result<std::path::PathBuf, ApiError> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name == "."
    {
        return Err(ApiError(FerroError::InvalidArgument(
            "invalid backup name".into(),
        )));
    }
    let target = svc.config().data_dir.join("backups").join(name);
    let meta = tokio::fs::symlink_metadata(&target)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                ApiError(FerroError::NotFound(format!("backup '{name}' not found")))
            }
            _ => ApiError(FerroError::Internal(anyhow::anyhow!(
                "inspect backup '{name}': {e}"
            ))),
        })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(ApiError(FerroError::InvalidArgument(
            "invalid backup directory".into(),
        )));
    }
    Ok(target)
}

async fn download_backup(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<AuthQuery>,
) -> Result<axum::response::Response, ApiError> {
    let caller = authenticate_caller(&svc, &headers, q.token.as_deref()).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let target = validate_backup_dir(&svc, &name).await?;
    let archive_root = name.clone();
    let gz_bytes = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let mut buf = Vec::new();
        {
            let encoder = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
            let mut builder = tar::Builder::new(encoder);
            builder.append_dir_all(&archive_root, &target)?;
            let encoder = builder.into_inner()?;
            encoder.finish()?;
        }
        Ok(buf)
    })
    .await
    .map_err(|e| ApiError(FerroError::Internal(anyhow::anyhow!("archive join: {e}"))))?
    .map_err(|e| ApiError(FerroError::Internal(anyhow::anyhow!("archive backup: {e}"))))?;

    let disposition = format!("attachment; filename=\"{name}.tar.gz\"");
    Ok((
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/gzip".to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
            (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
        ],
        Bytes::from(gz_bytes),
    )
        .into_response())
}

#[derive(Debug, Default, Deserialize)]
struct RestoreBackupReq {
    #[serde(default)]
    restart: bool,
}

#[derive(Debug, Serialize)]
struct RestoreBackupResp {
    backup: String,
    restored_files: Vec<String>,
    restarting: bool,
}

async fn restore_backup(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    req: Option<Json<RestoreBackupReq>>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let target = validate_backup_dir(&svc, &name).await?;
    let restored_files = svc
        .restore_snapshot(&caller, &target)
        .await
        .map_err(ApiError)?;
    let restart = req.map(|r| r.0.restart).unwrap_or(false);
    if restart {
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(600)).await;
            update::request_restart();
        });
    }
    Ok(Json(RestoreBackupResp {
        backup: name,
        restored_files,
        restarting: restart,
    }))
}

async fn get_backup_schedule(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let cfg = svc
        .get_auto_backup_config(&caller)
        .await
        .map_err(ApiError)?;
    Ok(Json(cfg))
}

async fn update_backup_schedule(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<AutoBackupConfig>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .update_auto_backup_config(&caller, req)
        .await
        .map_err(ApiError)?;
    Ok(Json(saved))
}

async fn list_projects(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_projects(&caller).await.map_err(ApiError)?))
}

async fn create_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateProjectReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let proj = svc
        .create_project_with_opts(
            &caller,
            &req.name,
            ProjectOpts {
                group: req.group,
                description: req.description,
                health_check_path: req.health_check_path,
                max_concurrency: req.max_concurrency,
                timeout_ms: req.timeout_ms,
                wasm_max_memory_mb: req.wasm_max_memory_mb,
                allow_private_outbound: req.allow_private_outbound,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(proj)))
}

async fn update_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateProjectReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let proj = svc
        .update_project(
            &caller,
            &id,
            ProjectOpts {
                group: req.group,
                description: req.description,
                health_check_path: req.health_check_path,
                max_concurrency: req.max_concurrency,
                timeout_ms: req.timeout_ms,
                wasm_max_memory_mb: req.wasm_max_memory_mb,
                allow_private_outbound: req.allow_private_outbound,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(proj))
}

async fn get_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.get_project(&caller, &id).await.map_err(ApiError)?))
}

async fn delete_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_project(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_deployments(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_deployments(&caller, &id).await.map_err(ApiError)?,
    ))
}

async fn create_deployment(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateDeploymentReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let artifact_bytes = match req.artifact_base64 {
        Some(b64) if !b64.trim().is_empty() => {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| {
                    ApiError(FerroError::InvalidArgument(format!(
                        "invalid artifact_base64: {e}"
                    )))
                })?;
            Some(Bytes::from(decoded))
        }
        _ => None,
    };
    let dep = svc
        .create_deployment(
            &caller,
            &id,
            CreateDeploymentInput {
                artifact_path: req.artifact_path,
                artifact_bytes,
                assets_bucket: req.assets_bucket,
                assets_prefix: req.assets_prefix,
                spa_fallback: req.spa_fallback,
                bindings: req.bindings,
                crons: req.crons,
                target_env: req.target_env,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(dep)))
}

#[derive(Debug, Deserialize)]
struct DeploymentEnvReq {
    env: HashMap<String, String>,
}

async fn create_deployment_with_env(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, deployment_id)): Path<(String, String)>,
    Json(req): Json<DeploymentEnvReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let deployment = svc
        .create_deployment_with_env(&caller, &id, &deployment_id, req.env)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(deployment)))
}

async fn upload_artifact(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (artifact_path, precompiled, size) = svc
        .upload_project_artifact(&caller, &id, &path, body)
        .await
        .map_err(ApiError)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "artifact_path": artifact_path,
            "precompiled": precompiled,
            "size": size,
        })),
    ))
}

async fn invoke_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<InvokeProjectReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let started = Instant::now();
    let method = req.method.unwrap_or_else(|| "GET".to_string());
    let req_headers: Vec<(String, Vec<u8>)> = req
        .headers
        .into_iter()
        .map(|(k, v)| (k, v.into_bytes()))
        .collect();
    let req_body = req.body.map(Bytes::from).unwrap_or_default();
    let (deployment_id, resp) = svc
        .invoke_project_http(
            &caller,
            &id,
            req.target_env,
            &method,
            &req.path,
            req_headers,
            req_body,
        )
        .await
        .map_err(ApiError)?;

    let body_bytes = resp
        .body
        .try_fold(Vec::new(), |mut acc, chunk| async move {
            acc.extend_from_slice(&chunk);
            Ok(acc)
        })
        .await
        .map_err(|e| {
            ApiError(FerroError::Internal(anyhow::anyhow!(
                "read response body: {e}"
            )))
        })?;

    let mut resp_headers = serde_json::Map::new();
    for (k, v) in resp.headers {
        resp_headers.insert(
            k,
            serde_json::Value::String(String::from_utf8_lossy(&v).into_owned()),
        );
    }

    Ok(Json(serde_json::json!({
        "status": resp.status,
        "headers": resp_headers,
        "body": String::from_utf8_lossy(&body_bytes).into_owned(),
        "latency_ms": started.elapsed().as_secs_f64() * 1000.0,
        "deployment_id": deployment_id,
    })))
}

async fn promote_deployment(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PromoteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let dep = svc
        .promote_deployment_with_health(
            &caller,
            &id,
            req.deployment_id.as_deref(),
            req.verify_health,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(dep))
}

async fn rollback_deployment(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PromoteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let dep = svc
        .rollback_deployment(&caller, &id, req.deployment_id.as_deref())
        .await
        .map_err(ApiError)?;
    Ok(Json(dep))
}

async fn list_secrets(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_secret_keys(&caller, &id).await.map_err(ApiError)?,
    ))
}

async fn set_secret(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SetSecretReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.set_secret(&caller, &id, &req.key, &req.value)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_secret(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, key)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_secret(&caller, &id, &key)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_routes(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_routes(&caller).await.map_err(ApiError)?))
}

async fn upsert_route(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<UpsertRouteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let r = svc
        .upsert_route_with_opts(
            &caller,
            &req.host,
            &req.project,
            req.target_env,
            RouteOpts {
                proxy_prefix: req.proxy_prefix,
                proxy_upstream: req.proxy_upstream,
                rate_limit_rpm: req.rate_limit_rpm,
                basic_auth_user: req.basic_auth_user,
                basic_auth_password: req.basic_auth_password,
                clear_basic_auth: req.clear_basic_auth,
                canary_deployment_id: req.canary_deployment_id,
                canary_weight: req.canary_weight,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(r))
}

async fn update_route(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(host): Path<String>,
    Json(req): Json<UpdateRouteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let r = svc
        .update_route_opts(
            &caller,
            &host,
            req.project.as_deref(),
            req.target_env,
            RouteOpts {
                proxy_prefix: req.proxy_prefix,
                proxy_upstream: req.proxy_upstream,
                rate_limit_rpm: req.rate_limit_rpm,
                basic_auth_user: req.basic_auth_user,
                basic_auth_password: req.basic_auth_password,
                clear_basic_auth: req.clear_basic_auth,
                canary_deployment_id: req.canary_deployment_id,
                canary_weight: req.canary_weight,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(r))
}

async fn delete_route(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(host): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_route(&caller, &host).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_tokens(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_tokens(&caller).await.map_err(ApiError)?))
}

async fn create_token(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateTokenReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let scope = TokenScope::parse(&req.scope).map_err(ApiError)?;
    let expires_at = match req.expires_in_days {
        Some(days) => {
            if !(1..=36_500).contains(&days) {
                return Err(ApiError(FerroError::InvalidArgument(
                    "expires_in_days must be between 1 and 36500".into(),
                )));
            }
            Some(Utc::now().timestamp().saturating_add(days * 86_400))
        }
        None => None,
    };
    let (meta, raw) = svc
        .create_token(&caller, &req.name, scope, expires_at)
        .await
        .map_err(ApiError)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "token": raw, "meta": meta })),
    ))
}

async fn revoke_token(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.revoke_token(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_create_namespace(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateKvNamespaceReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if req.default_ttl_secs == Some(0) {
        return Err(ApiError(FerroError::InvalidArgument(
            "kv default_ttl_secs must be greater than 0 (KV entries must have an expiration)"
                .into(),
        )));
    }
    let ns = svc
        .create_kv_namespace_with_opts(
            &caller,
            &req.name,
            KvNamespaceOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                default_ttl_secs: Some(req.default_ttl_secs.unwrap_or(DEFAULT_KV_TTL_SECS)),
                default_ephemeral: Some(req.default_ephemeral),
                max_value_bytes: req.max_value_bytes,
                max_keys: req.max_keys,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(ns)))
}

async fn kv_update_namespace(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(ns): Path<String>,
    Json(req): Json<UpdateKvNamespaceReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_kv_namespace(
            &caller,
            &ns,
            KvNamespaceOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                default_ttl_secs: req.default_ttl_secs,
                default_ephemeral: req.default_ephemeral,
                max_value_bytes: req.max_value_bytes,
                max_keys: req.max_keys,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn kv_delete_namespace(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(ns): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_kv_namespace(&caller, &ns)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_list_namespaces(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_kv_namespaces(&caller).await.map_err(ApiError)?,
    ))
}

async fn kv_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(ns): Path<String>,
    Query(q): Query<KvListQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.kv_list(&caller, &ns, q.prefix.as_deref(), q.limit.unwrap_or(100))
            .await
            .map_err(ApiError)?,
    ))
}

async fn kv_get(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((ns, key)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let val = svc
        .kv_get(&caller, &ns, &key)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(FerroError::NotFound(format!("kv '{ns}/{key}'"))))?;
    Ok(Json(serde_json::json!({
        "value": String::from_utf8_lossy(&val.value),
        "metadata": val.metadata,
        "expires_at": val.expires_at,
        "updated_at": val.updated_at,
    })))
}

async fn kv_put(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((ns, key)): Path<(String, String)>,
    Json(req): Json<KvPutReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if req.ttl_secs == Some(0) {
        return Err(ApiError(FerroError::InvalidArgument(
            "kv ttl_secs must be greater than 0 (KV entries must have an expiration)".into(),
        )));
    }
    svc.kv_put(
        &caller,
        &ns,
        &key,
        Bytes::from(req.value),
        req.metadata,
        req.ttl_secs,
        req.ephemeral,
    )
    .await
    .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn kv_incr(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((ns, key)): Path<(String, String)>,
    Json(req): Json<KvIncrReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if req.ttl_secs == Some(0) {
        return Err(ApiError(FerroError::InvalidArgument(
            "kv ttl_secs must be greater than 0 (KV entries must have an expiration)".into(),
        )));
    }
    let val = svc
        .kv_incr(&caller, &ns, &key, req.delta, req.ttl_secs)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "value": val })))
}

async fn kv_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((ns, key)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.kv_delete(&caller, &ns, &key).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn queue_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.queue_list(&caller).await.map_err(ApiError)?))
}

async fn queue_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateQueueReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let max_pending_jobs = req
        .max_pending_jobs
        .filter(|v| *v > 0)
        .or(if req.ephemeral { Some(10_000) } else { None });
    let cfg = QueueConfig {
        group: req.group,
        name: req.name,
        project_id: req.project_id,
        consumer_project_id: req.consumer_project_id,
        description: req.description,
        max_concurrency: req.max_concurrency.unwrap_or(4),
        rate_limit_per_sec: req.rate_limit_per_sec,
        batch_size: req.batch_size.unwrap_or(20),
        lease_secs: req.lease_secs.unwrap_or(svc.config().queue_lease_secs),
        max_attempts: req.max_attempts.unwrap_or(5),
        retry_backoff_secs: req.retry_backoff_secs.unwrap_or(5).max(1),
        dlq_retention_secs: req.dlq_retention_secs.filter(|v| *v > 0),
        ephemeral: req.ephemeral,
        max_pending_jobs,
        created_at: Utc::now().timestamp(),
    };
    let saved = svc.queue_create(&caller, cfg).await.map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn queue_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.queue_delete(&caller, &name).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct UpdateQueueReq {
    group: Option<String>,
    description: Option<String>,
    consumer_project_id: Option<String>,
    clear_consumer_project: Option<bool>,
    max_concurrency: Option<i64>,
    rate_limit_per_sec: Option<i64>,
    clear_rate_limit: Option<bool>,
    batch_size: Option<i64>,
    lease_secs: Option<i64>,
    max_attempts: Option<i64>,
    retry_backoff_secs: Option<i64>,
    dlq_retention_secs: Option<i64>,
    clear_dlq_retention: Option<bool>,
    ephemeral: Option<bool>,
    max_pending_jobs: Option<i64>,
    clear_max_pending_jobs: Option<bool>,
}

async fn queue_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<UpdateQueueReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let rate_opt = if req.clear_rate_limit.unwrap_or(false) {
        Some(None)
    } else {
        req.rate_limit_per_sec.map(Some)
    };
    let consumer_opt = if req.clear_consumer_project.unwrap_or(false) {
        Some(None)
    } else {
        req.consumer_project_id.map(Some)
    };
    let dlq_opt = if req.clear_dlq_retention.unwrap_or(false) {
        Some(None)
    } else {
        req.dlq_retention_secs.map(Some)
    };
    let max_pending_opt = if req.clear_max_pending_jobs.unwrap_or(false) {
        Some(None)
    } else {
        req.max_pending_jobs.map(Some)
    };
    let updated = svc
        .queue_update_full(
            &caller,
            &name,
            req.group,
            req.description,
            consumer_opt,
            req.max_concurrency,
            rate_opt,
            req.batch_size,
            req.lease_secs,
            req.max_attempts,
            req.retry_backoff_secs,
            dlq_opt,
            req.ephemeral,
            max_pending_opt,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn queue_enqueue(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<EnqueueReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let payload_bytes = serde_json::to_vec(&req.payload)
        .map_err(|e| ApiError(FerroError::InvalidArgument(e.to_string())))?;
    let job_id = svc
        .queue_enqueue(
            &caller,
            &name,
            Bytes::from(payload_bytes),
            EnqueueOpts {
                delay_secs: req.delay_secs,
                priority: req.priority,
                dedup_key: req.dedup_key,
                max_attempts: req.max_attempts,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "job_id": job_id })),
    ))
}

async fn queue_stats(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.queue_stats(&caller, &name).await.map_err(ApiError)?,
    ))
}

async fn queue_inspect(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<InspectQueueQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let status = JobStatus::parse(q.status.as_deref().unwrap_or("dead")).map_err(ApiError)?;
    Ok(Json(
        svc.queue_inspect(&caller, &name, status, q.limit.unwrap_or(50))
            .await
            .map_err(ApiError)?,
    ))
}

async fn queue_dlq_replay(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<ReplayDlqReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let count = svc
        .queue_dlq_replay(&caller, &name, req.job_ids.as_deref())
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "replayed": count })))
}

async fn queue_purge(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let count = svc.queue_purge(&caller, &name).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "purged": count })))
}

async fn bucket_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_buckets(&caller).await.map_err(ApiError)?))
}

async fn bucket_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateBucketReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let b = svc
        .create_bucket_with_opts(
            &caller,
            &req.name,
            BucketOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                public_read: Some(req.public_read),
                max_object_size_mb: req.max_object_size_mb,
                quota_mb: req.quota_mb,
                allowed_content_types: req.allowed_content_types,
                cache_control: req.cache_control,
                cors_origins: req.cors_origins,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(b)))
}

async fn bucket_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<UpdateBucketReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_bucket_with_opts(
            &caller,
            &name,
            BucketOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                public_read: req.public_read,
                max_object_size_mb: req.max_object_size_mb,
                quota_mb: req.quota_mb,
                allowed_content_types: req.allowed_content_types,
                cache_control: req.cache_control,
                cors_origins: req.cors_origins,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn bucket_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_bucket(&caller, &name).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct CreateCronReq {
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    schedule: String,
    target: String,
    enabled: Option<bool>,
    timezone: Option<String>,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct UpdateCronReq {
    description: Option<String>,
    enabled: Option<bool>,
    schedule: Option<String>,
    target: Option<String>,
    timezone: Option<String>,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
}

async fn cron_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_cron_jobs(&caller).await.map_err(ApiError)?))
}

async fn cron_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateCronReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .create_cron_job_with_opts(
            &caller,
            &req.name,
            CronJobOpts {
                project_id: req.project_id,
                description: req.description,
                schedule: Some(req.schedule),
                target: Some(req.target),
                enabled: req.enabled,
                timezone: req.timezone,
                timeout_ms: req.timeout_ms,
                max_retries: req.max_retries,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn cron_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateCronReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_cron_job_with_opts(
            &caller,
            &id,
            CronJobOpts {
                project_id: None,
                description: req.description,
                schedule: req.schedule,
                target: req.target,
                enabled: req.enabled,
                timezone: req.timezone,
                timeout_ms: req.timeout_ms,
                max_retries: req.max_retries,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn cron_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_cron_job(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cron_run(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let ran = svc.trigger_cron_job(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(ran))
}

async fn bucket_presign(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<PresignReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (url, expires_at) = svc
        .presign_blob_url(&caller, &req.op, &name, &req.key, req.expires_secs)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        serde_json::json!({ "url": url, "expires_at": expires_at }),
    ))
}

async fn db_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let items = svc.list_user_dbs(&caller).await.map_err(ApiError)?;
    Ok(Json(items))
}

async fn db_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateDbReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let meta = svc
        .create_user_db_with_opts(
            &caller,
            &req.name,
            UserDbOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                init_sql: req.init_sql,
                max_rows: req.max_rows,
                max_size_mb: req.max_size_mb,
                foreign_keys: req.foreign_keys,
                read_only: req.read_only,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(meta)))
}

async fn db_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateDbReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_user_db(
            &caller,
            &id,
            UserDbOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                init_sql: None,
                max_rows: req.max_rows,
                max_size_mb: req.max_size_mb,
                foreign_keys: req.foreign_keys,
                read_only: req.read_only,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn db_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_user_db(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn db_schema(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let schema = svc.schema_user_db(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "schema": schema })))
}

async fn db_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<DbQueryReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_user_db(&caller, &id, &req.sql, req.params)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

async fn db_execute(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<DbExecuteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let stmts = req
        .statements
        .into_iter()
        .map(|s| (s.sql, s.params))
        .collect();
    let affected = svc
        .execute_user_db_batch(&caller, &id, stmts)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "rows_affected": affected })))
}

#[derive(Debug, Deserialize)]
struct CreateDbFtsReq {
    name: String,
    table: String,
    columns: Vec<String>,
    tokenizer: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DbFtsQueryReq {
    #[serde(default)]
    query: String,
}

async fn db_fts_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let items = svc.list_user_db_fts(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(items))
}

async fn db_fts_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateDbFtsReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let created = svc
        .create_user_db_fts(
            &caller,
            &id,
            &req.name,
            &req.table,
            req.columns,
            req.tokenizer.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn db_fts_drop(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.drop_user_db_fts(&caller, &id, &name)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn db_fts_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    Json(req): Json<DbFtsQueryReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_user_db_fts(&caller, &id, &name, &req.query)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
struct CreateSearchIndexReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    tokenizer: Option<String>,
    writer_budget_mb: Option<usize>,
    schema: Option<Vec<SearchFieldSchema>>,
    facet_fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct UpdateSearchIndexReq {
    group: Option<String>,
    description: Option<String>,
    writer_budget_mb: Option<usize>,
}

async fn search_list_indexes(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_search_indexes(&caller).await.map_err(ApiError)?,
    ))
}

async fn search_create_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateSearchIndexReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let created = svc
        .create_search_index_with_opts(
            &caller,
            &req.name,
            SearchIndexOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                tokenizer: req.tokenizer,
                writer_budget_mb: req.writer_budget_mb,
                schema: req.schema,
                facet_fields: req.facet_fields,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(created)))
}

async fn search_update_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateSearchIndexReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_search_index(
            &caller,
            &id,
            SearchIndexOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                tokenizer: None,
                writer_budget_mb: req.writer_budget_mb,
                schema: None,
                facet_fields: None,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn search_delete_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_search_index(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn search_commit_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .commit_search_index(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
struct SearchDocsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

async fn search_list_docs(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<SearchDocsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let docs = svc
        .list_search_docs_page(
            &caller,
            &id,
            query.limit.unwrap_or(100),
            query.offset.unwrap_or(0),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(docs))
}

async fn search_upsert_doc(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(doc): Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .upsert_search_doc(&caller, &id, doc)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn search_delete_doc(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, doc_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let deleted = svc
        .delete_search_doc(&caller, &id, &doc_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

async fn search_query_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SearchQueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_search_index(&caller, &id, req)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

async fn logs_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Query(q): Query<LogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.query_logs(&caller, q).await.map_err(ApiError)?))
}

async fn logs_stream(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Query(q): Query<AuthQuery>,
) -> Result<Sse<impl Stream<Item = std::result::Result<Event, Infallible>>>, ApiError> {
    let caller = authenticate_caller(&svc, &headers, q.token.as_deref()).await?;
    let stream = svc.subscribe_logs(&caller).into_stream().map(|entry| {
        let json = serde_json::to_string(&entry).unwrap_or_default();
        Ok(Event::default().data(json))
    });
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

#[derive(Debug, Deserialize)]
struct ReleasesQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ApplyUpdateReq {
    version: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default = "default_true")]
    restart: bool,
}

fn default_true() -> bool {
    true
}

async fn system_info(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let _caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(update::system_info()))
}

async fn system_update_check(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let res = update::check_for_update()
        .await
        .map_err(|e| ApiError(FerroError::Internal(e)))?;
    Ok(Json(res))
}

async fn system_releases(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Query(q): Query<ReleasesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let items = update::list_releases(q.limit.unwrap_or(10))
        .await
        .map_err(|e| ApiError(FerroError::Internal(e)))?;
    Ok(Json(items))
}

async fn system_update_apply(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<ApplyUpdateReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    if !caller.scope.allows_admin() {
        return Err(ApiError(FerroError::Forbidden(
            "admin scope required".into(),
        )));
    }
    let _lock = update::UpdateLock::try_acquire().ok_or_else(|| {
        ApiError(FerroError::Conflict(
            "another update is already in progress".into(),
        ))
    })?;

    let res = update::apply_update(req.version.as_deref(), req.force, req.restart)
        .await
        .map_err(|e| ApiError(FerroError::Internal(e)))?;

    if res.restarting && (res.new_version != res.previous_version || req.force) {
        tokio::spawn(async {
            tokio::time::sleep(Duration::from_millis(600)).await;
            update::request_restart();
        });
    }

    Ok(Json(res))
}

#[derive(Debug, Default, Deserialize)]
struct ResetSettingsReq {
    section: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DnsCheckReq {
    domain: String,
    #[serde(default)]
    check_wildcard: bool,
}

async fn get_platform_settings(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let bundle = svc.get_platform_settings(&caller).await.map_err(ApiError)?;
    Ok(Json(bundle))
}

async fn update_platform_settings(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(patch): Json<PlatformSettingsPatch>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let request_host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty());
    let bundle = svc
        .update_platform_settings(&caller, patch, request_host)
        .await
        .map_err(ApiError)?;
    Ok(Json(bundle))
}

async fn reset_platform_settings(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    body: Option<Json<ResetSettingsReq>>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let section = body.and_then(|Json(r)| r.section);
    let bundle = svc
        .reset_platform_settings(&caller, section.as_deref())
        .await
        .map_err(ApiError)?;
    Ok(Json(bundle))
}

async fn settings_dns_check(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<DnsCheckReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .check_domain_dns(&caller, &req.domain, req.check_wildcard)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

async fn settings_test_alert(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc.test_alert_webhook(&caller).await.map_err(ApiError)?;
    Ok(Json(res))
}

#[derive(Debug, Deserialize)]
struct CreateAiProviderReq {
    name: String,
    base_url: String,
    api_key: Option<String>,
    default_model: Option<String>,
    fallback_provider: Option<String>,
    rate_limit_rpm: Option<u32>,
    cache_ttl_secs: Option<u64>,
    monthly_token_budget: Option<u64>,
    timeout_secs: Option<u64>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateAiProviderReq {
    base_url: Option<String>,
    api_key: Option<String>,
    default_model: Option<String>,
    fallback_provider: Option<String>,
    rate_limit_rpm: Option<u32>,
    cache_ttl_secs: Option<u64>,
    monthly_token_budget: Option<u64>,
    timeout_secs: Option<u64>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct AiUsageQuery {
    provider: Option<String>,
    limit: Option<usize>,
}

async fn ai_list_providers(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_ai_providers(&caller).await.map_err(ApiError)?,
    ))
}

async fn ai_create_provider(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateAiProviderReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .create_ai_provider(
            &caller,
            &req.name,
            AiProviderOpts {
                name: None,
                base_url: Some(req.base_url),
                api_key: req.api_key,
                default_model: req.default_model,
                fallback_provider: req.fallback_provider,
                rate_limit_rpm: req.rate_limit_rpm,
                cache_ttl_secs: req.cache_ttl_secs,
                monthly_token_budget: req.monthly_token_budget,
                timeout_secs: req.timeout_secs,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn ai_update_provider(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(req): Json<UpdateAiProviderReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_ai_provider(
            &caller,
            &name,
            AiProviderOpts {
                name: None,
                base_url: req.base_url,
                api_key: req.api_key,
                default_model: req.default_model,
                fallback_provider: req.fallback_provider,
                rate_limit_rpm: req.rate_limit_rpm,
                cache_ttl_secs: req.cache_ttl_secs,
                monthly_token_budget: req.monthly_token_budget,
                timeout_secs: req.timeout_secs,
                enabled: req.enabled,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn ai_delete_provider(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_ai_provider(&caller, &name)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ai_usage_summary(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Query(q): Query<AiUsageQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let summary = svc
        .list_ai_usage(&caller, q.provider.as_deref(), q.limit.unwrap_or(50))
        .await
        .map_err(ApiError)?;
    Ok(Json(summary))
}

async fn ai_clear_cache(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let evicted = svc.clear_ai_cache(&caller).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "evicted": evicted })))
}

pub async fn ai_list_models(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let models = svc.list_ai_models(&caller).await.map_err(ApiError)?;
    Ok(Json(models))
}

async fn ai_proxy_inner(
    svc: PlatformService,
    headers: HeaderMap,
    subpath: &str,
    body: serde_json::Value,
) -> Result<axum::response::Response, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let requested = headers
        .get("x-ferropod-provider")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| {
            body.get("provider")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    match svc
        .ai_proxy_or_stream(&caller, requested.as_deref(), subpath, body)
        .await
        .map_err(ApiError)?
    {
        AiProxyResult::Json(res) => {
            let status = StatusCode::from_u16(res.status).unwrap_or(StatusCode::OK);
            let cache_hdr = if res.cached { "HIT" } else { "MISS" };
            Ok((
                status,
                [
                    ("x-ferropod-cache", cache_hdr.to_string()),
                    ("x-ferropod-provider", res.provider_name),
                ],
                Json(res.body),
            )
                .into_response())
        }
        AiProxyResult::Stream(st) => {
            let status = StatusCode::from_u16(st.status).unwrap_or(StatusCode::OK);
            Ok(axum::response::Response::builder()
                .status(status)
                .header(axum::http::header::CONTENT_TYPE, st.content_type)
                .header(axum::http::header::CACHE_CONTROL, "no-cache")
                .header("x-ferropod-cache", "BYPASS")
                .header("x-ferropod-provider", st.provider_name)
                .body(axum::body::Body::from_stream(st.body))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response()))
        }
    }
}

pub async fn ai_chat_completions(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    ai_proxy_inner(svc, headers, "chat/completions", body).await
}

pub async fn ai_embeddings(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    ai_proxy_inner(svc, headers, "embeddings", body).await
}

async fn alert_webhooks_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_alert_webhooks(&caller).await.map_err(ApiError)?,
    ))
}

async fn alert_webhooks_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<AlertWebhookOpts>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .create_alert_webhook(&caller, req)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn alert_webhooks_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<AlertWebhookOpts>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_alert_webhook(&caller, &id, req)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

async fn alert_webhooks_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_alert_webhook(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn alert_webhooks_test(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .test_alert_webhook_by_id(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

async fn realtime_list_channels(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let channels = svc
        .realtime_list_channels(&caller)
        .await
        .map_err(ApiError)?;
    Ok(Json(channels))
}

pub async fn realtime_sse(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(channel): Path<String>,
    Query(q): Query<AuthQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let caller = authenticate_caller(&svc, &headers, q.token.as_deref()).await?;
    let rx = svc
        .realtime_subscribe(&caller, &channel)
        .await
        .map_err(ApiError)?;

    let stream = futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Ok(ev) = Event::default()
                        .id(msg.id.clone())
                        .event(msg.event.clone())
                        .json_data(&msg)
                    {
                        return Some((Ok::<Event, Infallible>(ev), rx));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    return Some((
                        Ok(Event::default().event("lagged").data(skipped.to_string())),
                        rx,
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

pub async fn realtime_publish(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(channel): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (event, data) = if let Some(obj) = body.as_object() {
        if obj.contains_key("data") {
            (
                obj.get("event").and_then(|v| v.as_str()),
                obj.get("data").cloned().unwrap_or(serde_json::Value::Null),
            )
        } else {
            (obj.get("event").and_then(|v| v.as_str()), body.clone())
        }
    } else {
        (None, body)
    };
    let msg = svc
        .realtime_publish(&caller, &channel, event, data)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(msg)))
}
