use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use futures_util::stream::Stream;
use serde::Deserialize;

use crate::core::model::{AiProviderOpts, AlertWebhookOpts};
use crate::core::service::{AiProxyResult, PlatformService};
use crate::gateway::guard::{authenticate_caller, ApiError};

use super::AuthQuery;

#[derive(Debug, Deserialize)]
pub(super) struct CreateAiProviderReq {
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
pub(super) struct UpdateAiProviderReq {
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
pub(super) struct AiUsageQuery {
    provider: Option<String>,
    limit: Option<usize>,
}

pub(super) async fn ai_list_providers(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_ai_providers(&caller).await.map_err(ApiError)?,
    ))
}

pub(super) async fn ai_create_provider(
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

pub(super) async fn ai_update_provider(
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

pub(super) async fn ai_delete_provider(
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

pub(super) async fn ai_usage_summary(
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

pub(super) async fn ai_clear_cache(
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

pub(super) async fn alert_webhooks_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_alert_webhooks(&caller).await.map_err(ApiError)?,
    ))
}

pub(super) async fn alert_webhooks_create(
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

pub(super) async fn alert_webhooks_update(
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

pub(super) async fn alert_webhooks_delete(
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

pub(super) async fn alert_webhooks_test(
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

pub(super) async fn realtime_list_channels(
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
    let (rx, permit) = svc
        .realtime_subscribe(&caller, &channel)
        .await
        .map_err(ApiError)?;

    let stream = futures_util::stream::unfold((rx, permit), |(mut rx, permit)| async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if let Ok(ev) = Event::default()
                        .id(msg.id.clone())
                        .event(msg.event.clone())
                        .json_data(&msg)
                    {
                        return Some((Ok::<Event, Infallible>(ev), (rx, permit)));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    return Some((
                        Ok(Event::default().event("lagged").data(skipped.to_string())),
                        (rx, permit),
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
