use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use bytes::Bytes;
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use tokio_stream::StreamExt;

use crate::core::model::{AutoBackupConfig, FerroError, LogQuery, PlatformSettingsPatch};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};
use crate::update;

use super::AuthQuery;

#[derive(Debug, Serialize)]
pub(super) struct StatusResp {
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


pub(super) async fn list_resource_groups(
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


pub(super) async fn api_openapi() -> impl IntoResponse {
    Json(crate::web::openapi::openapi_document())
}

pub(super) async fn api_llms_txt() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; charset=utf-8",
        )],
        crate::web::openapi::llms_txt(),
    )
}

pub(super) async fn status(
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
pub(super) struct MetricsResp {
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

pub(super) async fn metrics(
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

pub(super) async fn metrics_prometheus(
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
pub(super) struct CreateBackupReq {
    label: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct BackupInfo {
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

pub(super) async fn list_backups(
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

pub(super) async fn create_backup(
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

pub(super) async fn delete_backup(
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

pub(super) async fn download_backup(
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
pub(super) struct RestoreBackupReq {
    #[serde(default)]
    restart: bool,
}

#[derive(Debug, Serialize)]
pub(super) struct RestoreBackupResp {
    backup: String,
    restored_files: Vec<String>,
    restarting: bool,
}

pub(super) async fn restore_backup(
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

pub(super) async fn get_backup_schedule(
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

pub(super) async fn update_backup_schedule(
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


pub(super) async fn logs_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Query(q): Query<LogQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.query_logs(&caller, q).await.map_err(ApiError)?))
}

pub(super) async fn logs_stream(
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
pub(super) struct ReleasesQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ApplyUpdateReq {
    version: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default = "default_true")]
    restart: bool,
}

fn default_true() -> bool {
    true
}

pub(super) async fn system_info(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let _caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(update::system_info()))
}

pub(super) async fn system_update_check(
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

pub(super) async fn system_releases(
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

pub(super) async fn system_update_apply(
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
pub(super) struct ResetSettingsReq {
    section: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DnsCheckReq {
    domain: String,
    #[serde(default)]
    check_wildcard: bool,
}

pub(super) async fn get_platform_settings(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let bundle = svc.get_platform_settings(&caller).await.map_err(ApiError)?;
    Ok(Json(bundle))
}

pub(super) async fn update_platform_settings(
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

pub(super) async fn reset_platform_settings(
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

pub(super) async fn settings_dns_check(
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

pub(super) async fn settings_test_alert(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc.test_alert_webhook(&caller).await.map_err(ApiError)?;
    Ok(Json(res))
}

