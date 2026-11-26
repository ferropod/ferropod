use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use bytes::Bytes;
use serde::Deserialize;

use crate::core::model::{FerroError, KvNamespaceOpts, DEFAULT_KV_TTL_SECS};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateKvNamespaceReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    default_ttl_secs: Option<u64>,
    #[serde(default)]
    default_ephemeral: bool,
    max_value_bytes: Option<usize>,
    max_keys: Option<usize>,
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateKvNamespaceReq {
    group: Option<String>,
    description: Option<String>,
    default_ttl_secs: Option<u64>,
    default_ephemeral: Option<bool>,
    max_value_bytes: Option<usize>,
    max_keys: Option<usize>,
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KvPutReq {
    value: String,
    metadata: Option<String>,
    ttl_secs: Option<u64>,
    #[serde(default)]
    ephemeral: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct KvIncrReq {
    delta: i64,
    ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KvListQuery {
    prefix: Option<String>,
    limit: Option<usize>,
}


pub(super) async fn kv_create_namespace(
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
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(ns)))
}

pub(super) async fn kv_update_namespace(
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
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

pub(super) async fn kv_delete_namespace(
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

pub(super) async fn kv_list_namespaces(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_kv_namespaces(&caller).await.map_err(ApiError)?,
    ))
}

pub(super) async fn kv_list(
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

pub(super) async fn kv_get(
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

pub(super) async fn kv_put(
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

pub(super) async fn kv_incr(
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

pub(super) async fn kv_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((ns, key)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.kv_delete(&caller, &ns, &key).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

