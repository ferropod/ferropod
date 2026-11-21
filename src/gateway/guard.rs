use std::time::Instant;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::core::model::{Caller, FerroError, LogEntry};
use crate::core::service::PlatformService;

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

/// HTTP error wrapper translating [`FerroError`] into status codes and JSON payloads.
pub struct ApiError(pub FerroError);

impl From<FerroError> for ApiError {
    fn from(value: FerroError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            FerroError::NotFound(_) => StatusCode::NOT_FOUND,
            FerroError::Conflict(_) => StatusCode::CONFLICT,
            FerroError::InvalidArgument(_) => StatusCode::BAD_REQUEST,
            FerroError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            FerroError::Forbidden(_) => StatusCode::FORBIDDEN,
            FerroError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            FerroError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            FerroError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json(ErrorBody {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

/// Extract bearer token from `Authorization` header (or optional `?token=` query param).
pub fn extract_bearer_token(req_headers: &axum::http::HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(auth) = req_headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(tok) = auth.strip_prefix("Bearer ").or_else(|| auth.strip_prefix("bearer ")) {
            let t = tok.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    query_token
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub async fn authenticate_caller(
    svc: &PlatformService,
    headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
) -> std::result::Result<Caller, ApiError> {
    let raw = extract_bearer_token(headers, query_token)
        .ok_or_else(|| ApiError(FerroError::Unauthorized("missing bearer token".into())))?;
    let tok = svc.authenticate_token(&raw).await.map_err(ApiError)?;
    Ok(Caller::from_token(&tok))
}

/// Request ID injection, global concurrency limiting, and non-blocking access logging middleware.
pub async fn access_log_middleware(
    State(svc): State<PlatformService>,
    mut req: Request,
    next: Next,
) -> Response {
    let permit = match svc.try_acquire_http_permit() {
        Some(p) => p,
        None => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorBody {
                    error: "server concurrency limit reached".into(),
                }),
            )
                .into_response();
        }
    };

    let req_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| format!("req_{}", Uuid::new_v4().simple()));

    if let Ok(hv) = HeaderValue::from_str(&req_id) {
        req.headers_mut().insert("x-request-id", hv);
    }

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let started = Instant::now();
    let mut res = next.run(req).await;
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = res.status().as_u16();
    if let Ok(hv) = HeaderValue::from_str(&req_id) {
        res.headers_mut().insert("x-request-id", hv);
    }

    // Skip logging internal loopback TLS checks to avoid noise
    if path != "/_internal/tls/allow" {
        let level = if status >= 500 {
            "error"
        } else if status >= 400 {
            "warn"
        } else {
            "info"
        };
        svc.emit_log(LogEntry {
            id: None,
            ts: Utc::now().timestamp_millis(),
            project_id: None,
            request_id: Some(req_id),
            source: "access".to_string(),
            level: level.to_string(),
            message: format!("{method} {path} -> {status} ({elapsed_ms}ms)"),
            meta_json: Some(
                serde_json::json!({
                    "method": method,
                    "path": path,
                    "host": host,
                    "status": status,
                    "duration_ms": elapsed_ms,
                })
                .to_string(),
            ),
        });
    }

    res
}
