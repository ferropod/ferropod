use std::collections::HashMap;
use std::time::Instant;

use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use serde::Deserialize;

use crate::core::model::{
    Bindings, CronJobOpts, FerroError, ProjectOpts, RouteOpts, TargetEnv, TokenScope,
};
use crate::core::service::{CreateDeploymentInput, PlatformService};
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateProjectReq {
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
pub(super) struct UpdateProjectReq {
    group: Option<String>,
    description: Option<String>,
    health_check_path: Option<String>,
    max_concurrency: Option<i64>,
    timeout_ms: Option<u64>,
    wasm_max_memory_mb: Option<usize>,
    allow_private_outbound: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateDeploymentReq {
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
pub(super) struct InvokeProjectReq {
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
pub(super) struct PromoteReq {
    deployment_id: Option<String>,
    #[serde(default)]
    verify_health: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpsertRouteReq {
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
pub(super) struct UpdateRouteReq {
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
pub(super) struct SetSecretReq {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTokenReq {
    name: String,
    scope: String,
    expires_in_days: Option<i64>,
}


pub(super) fn runtime_routes(prefix: &str) -> Router<PlatformService> {
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


pub(super) async fn list_projects(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_projects(&caller).await.map_err(ApiError)?))
}

pub(super) async fn create_project(
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

pub(super) async fn update_project(
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

pub(super) async fn get_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.get_project(&caller, &id).await.map_err(ApiError)?))
}

pub(super) async fn delete_project(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_project(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn list_deployments(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_deployments(&caller, &id).await.map_err(ApiError)?,
    ))
}

pub(super) async fn create_deployment(
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
pub(super) struct DeploymentEnvReq {
    env: HashMap<String, String>,
}

pub(super) async fn create_deployment_with_env(
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

pub(super) async fn upload_artifact(
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

pub(super) async fn invoke_project(
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

pub(super) async fn promote_deployment(
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

pub(super) async fn rollback_deployment(
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

pub(super) async fn list_secrets(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_secret_keys(&caller, &id).await.map_err(ApiError)?,
    ))
}

pub(super) async fn set_secret(
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

pub(super) async fn delete_secret(
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

pub(super) async fn list_routes(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_routes(&caller).await.map_err(ApiError)?))
}

pub(super) async fn upsert_route(
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

pub(super) async fn update_route(
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

pub(super) async fn delete_route(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(host): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_route(&caller, &host).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn list_tokens(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_tokens(&caller).await.map_err(ApiError)?))
}

pub(super) async fn create_token(
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

pub(super) async fn revoke_token(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.revoke_token(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}


#[derive(Debug, Deserialize)]
pub(super) struct CreateCronReq {
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
pub(super) struct UpdateCronReq {
    description: Option<String>,
    enabled: Option<bool>,
    schedule: Option<String>,
    target: Option<String>,
    timezone: Option<String>,
    timeout_ms: Option<u64>,
    max_retries: Option<u32>,
}

pub(super) async fn cron_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(svc.list_cron_jobs(&caller).await.map_err(ApiError)?))
}

pub(super) async fn cron_create(
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

pub(super) async fn cron_update(
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

pub(super) async fn cron_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_cron_job(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn cron_run(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let ran = svc.trigger_cron_job(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(ran))
}

