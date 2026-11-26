use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::core::model::BucketOpts;
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateBucketReq {
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
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateBucketReq {
    group: Option<String>,
    description: Option<String>,
    public_read: Option<bool>,
    max_object_size_mb: Option<usize>,
    quota_mb: Option<usize>,
    allowed_content_types: Option<String>,
    cache_control: Option<String>,
    cors_origins: Option<String>,
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PresignReq {
    key: String,
    op: String,
    #[serde(default = "default_presign_ttl")]
    expires_secs: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct BrowseBucketQuery {
    prefix: Option<String>,
    search: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeleteBucketPrefixQuery {
    prefix: String,
}

fn default_presign_ttl() -> u64 {
    300
}


pub(super) async fn bucket_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_buckets(&caller).await.map_err(ApiError)?))
}

pub(super) async fn bucket_create(
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
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(b)))
}

pub(super) async fn bucket_update(
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
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

pub(super) async fn bucket_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_bucket(&caller, &name).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}


pub(super) async fn bucket_presign(
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

pub(super) async fn bucket_browse(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<BrowseBucketQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .browse_bucket(
            &caller,
            &name,
            q.prefix.as_deref(),
            q.search.as_deref(),
            20_000,
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

pub(super) async fn bucket_delete_prefix(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(q): Query<DeleteBucketPrefixQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let (prefix, deleted) = svc
        .delete_objects_by_prefix(&caller, &name, &q.prefix)
        .await
        .map_err(ApiError)?;
    Ok(Json(
        serde_json::json!({ "prefix": prefix, "deleted": deleted }),
    ))
}

