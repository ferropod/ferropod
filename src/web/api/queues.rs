use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use chrono::Utc;
use serde::Deserialize;

use crate::core::model::{EnqueueOpts, FerroError, JobStatus, QueueConfig};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateQueueReq {
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
pub(super) struct EnqueueReq {
    payload: serde_json::Value,
    delay_secs: Option<u64>,
    priority: Option<i32>,
    dedup_key: Option<String>,
    max_attempts: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct InspectQueueQuery {
    status: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReplayDlqReq {
    job_ids: Option<Vec<i64>>,
}


pub(super) async fn queue_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.queue_list(&caller).await.map_err(ApiError)?))
}

pub(super) async fn queue_create(
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

pub(super) async fn queue_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.queue_delete(&caller, &name).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateQueueReq {
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

pub(super) async fn queue_update(
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

pub(super) async fn queue_enqueue(
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

pub(super) async fn queue_stats(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.queue_stats(&caller, &name).await.map_err(ApiError)?,
    ))
}

pub(super) async fn queue_inspect(
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

pub(super) async fn queue_dlq_replay(
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

pub(super) async fn queue_purge(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let count = svc.queue_purge(&caller, &name).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "purged": count })))
}

