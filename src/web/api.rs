use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{delete, get, patch, post},
    Json, Router,
};
use bytes::Bytes;
use chrono::Utc;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::core::model::{
    Bindings, EnqueueOpts, FerroError, JobStatus, LogQuery, QueueConfig, SqlValue, TargetEnv,
    TokenScope,
};
use crate::core::service::{CreateDeploymentInput, PlatformService};
use crate::gateway::guard::{authenticate_caller, ApiError};
use crate::update;

#[derive(Debug, Deserialize)]
struct AuthQuery {
    token: Option<String>,
}

#[derive(Debug, Serialize)]
struct StatusResp {
    status: &'static str,
    version: &'static str,
    profile: String,
    actor: String,
    scope: String,
    projects: usize,
    routes: usize,
    queues: usize,
    buckets: usize,
    dbs: usize,
    kv_namespaces: usize,
}

#[derive(Debug, Deserialize)]
struct CreateProjectReq {
    name: String,
    health_check_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateDeploymentReq {
    artifact_path: Option<String>,
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
struct PromoteReq {
    deployment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpsertRouteReq {
    host: String,
    project: String,
    #[serde(default = "default_prod")]
    target_env: TargetEnv,
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
    name: String,
    project_id: Option<String>,
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
    name: String,
    project_id: Option<String>,
    consumer_project_id: Option<String>,
    max_concurrency: Option<i64>,
    rate_limit_per_sec: Option<i64>,
    batch_size: Option<i64>,
    lease_secs: Option<i64>,
    max_attempts: Option<i64>,
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
    name: String,
    project_id: Option<String>,
    #[serde(default)]
    public_read: bool,
}

#[derive(Debug, Deserialize)]
struct UpdateBucketReq {
    public_read: bool,
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
    name: String,
    project_id: Option<String>,
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
        .route("/status", get(status))
        .route("/metrics", get(metrics))
        .route("/system/info", get(system_info))
        .route("/system/update/check", get(system_update_check))
        .route("/system/releases", get(system_releases))
        .route("/system/update/apply", post(system_update_apply))
        .route("/backups", get(list_backups).post(create_backup))
        .route("/backups/{name}", delete(delete_backup))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{id}", get(get_project).delete(delete_project))
        .route(
            "/projects/{id}/deployments",
            get(list_deployments).post(create_deployment),
        )
        .route("/projects/{id}/promote", post(promote_deployment))
        .route("/projects/{id}/rollback", post(rollback_deployment))
        .route("/projects/{id}/secrets", get(list_secrets).post(set_secret))
        .route("/projects/{id}/secrets/{key}", delete(delete_secret))
        .route("/routes", get(list_routes).post(upsert_route))
        .route("/routes/{host}", delete(delete_route))
        .route("/tokens", get(list_tokens).post(create_token))
        .route("/tokens/{id}", delete(revoke_token))
        .route(
            "/kv",
            get(kv_list_namespaces).post(kv_create_namespace),
        )
        .route("/kv/{ns}", get(kv_list).delete(kv_delete_namespace))
        .route(
            "/kv/{ns}/{key}",
            get(kv_get).put(kv_put).delete(kv_delete),
        )
        .route("/kv/{ns}/{key}/incr", post(kv_incr))
        .route("/queues", get(queue_list).post(queue_create))
        .route("/queues/{name}", delete(queue_delete))
        .route("/queues/{name}/enqueue", post(queue_enqueue))
        .route("/queues/{name}/stats", get(queue_stats))
        .route("/queues/{name}/inspect", get(queue_inspect))
        .route("/queues/{name}/dlq/replay", post(queue_dlq_replay))
        .route("/queues/{name}/purge", post(queue_purge))
        .route("/buckets", get(bucket_list).post(bucket_create))
        .route("/buckets/{name}", patch(bucket_update))
        .route("/buckets/{name}/presign", post(bucket_presign))
        .route("/dbs", get(db_list).post(db_create))
        .route("/dbs/{id}", delete(db_delete))
        .route("/dbs/{id}/schema", get(db_schema))
        .route("/dbs/{id}/query", post(db_query))
        .route("/dbs/{id}/execute", post(db_execute))
        .route("/logs", get(logs_query))
        .route("/logs/stream", get(logs_stream))
}

async fn status(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (projects, routes, queues, buckets, dbs, kv_namespaces) = tokio::try_join!(
        svc.list_projects(&caller),
        svc.list_routes(&caller),
        svc.queue_list(&caller),
        svc.list_buckets(&caller),
        svc.list_user_dbs(&caller),
        svc.list_kv_namespaces(&caller),
    )
    .map_err(ApiError)?;
    Ok(Json(StatusResp {
        status: "ok",
        version: update::CURRENT_VERSION,
        profile: format!("{:?}", svc.config().profile),
        actor: caller.actor.clone(),
        scope: caller.scope.as_str(),
        projects: projects.len(),
        routes: routes.len(),
        queues: queues.len(),
        buckets: buckets.len(),
        dbs: dbs.len(),
        kv_namespaces: kv_namespaces.len(),
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
            if let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) {
                total = kb.saturating_mul(1024);
            }
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            if let Some(kb) = rest.split_whitespace().next().and_then(|s| s.parse::<u64>().ok()) {
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
    let l1 = parts.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let l5 = parts.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
    let l15 = parts.next().and_then(|s| s.parse::<f64>().ok()).unwrap_or(0.0);
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
    let mem_budget_bytes = if profile.eq_ignore_ascii_case("lowmem") {
        512 * 1024 * 1024
    } else if mem_total_bytes > 0 {
        mem_total_bytes
    } else {
        512 * 1024 * 1024
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
    }))
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
    items.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
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
    let name = match req.label.as_deref().and_then(sanitize_backup_segment) {
        Some(lbl) => format!("snap_{ts}_{lbl}"),
        None => format!("snap_{ts}"),
    };
    let backups_root = svc.config().data_dir.join("backups");
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
    let target = svc.config().data_dir.join("backups").join(&name);
    if !target.exists() {
        return Err(ApiError(FerroError::NotFound(format!(
            "backup '{name}' not found"
        ))));
    }
    tokio::fs::remove_dir_all(&target)
        .await
        .map_err(|e| ApiError(FerroError::Internal(anyhow::anyhow!("delete backup: {e}"))))?;
    Ok(StatusCode::NO_CONTENT)
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
        .create_project(&caller, &req.name, req.health_check_path.as_deref())
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(proj)))
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
    let dep = svc
        .create_deployment(
            &caller,
            &id,
            CreateDeploymentInput {
                artifact_path: req.artifact_path,
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

async fn promote_deployment(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<PromoteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let dep = svc
        .promote_deployment(&caller, &id, req.deployment_id.as_deref())
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
        .upsert_route(&caller, &req.host, &req.project, req.target_env)
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
    let expires_at = req
        .expires_in_days
        .map(|d| Utc::now().timestamp() + d * 86_400);
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
    let ns = svc
        .create_kv_namespace(&caller, &req.name, req.project_id.as_deref())
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(ns)))
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
    let cfg = QueueConfig {
        name: req.name,
        project_id: req.project_id,
        consumer_project_id: req.consumer_project_id,
        max_concurrency: req.max_concurrency.unwrap_or(4),
        rate_limit_per_sec: req.rate_limit_per_sec,
        batch_size: req.batch_size.unwrap_or(20),
        lease_secs: req.lease_secs.unwrap_or(svc.config().queue_lease_secs),
        max_attempts: req.max_attempts.unwrap_or(5),
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
        .create_bucket(&caller, &req.name, req.project_id.as_deref(), req.public_read)
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
    svc.set_bucket_public(&caller, &name, req.public_read)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
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
    Ok(Json(serde_json::json!({ "url": url, "expires_at": expires_at })))
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
    let id = svc
        .create_user_db(&caller, &req.name, req.project_id.as_deref())
        .await
        .map_err(ApiError)?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "id": id, "name": req.name })),
    ))
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
