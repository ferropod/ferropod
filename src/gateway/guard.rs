use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
use crate::core::service::{GatewayMetrics, PlatformService};

/// Bounded sliding-window per-key rate limiter (60-second window).
#[derive(Debug, Default)]
pub struct SlidingWindowRateLimiter {
    windows: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl SlidingWindowRateLimiter {
    pub fn check_and_record(&self, key: &str, max_per_min: u32) -> (bool, u32, u64) {
        if max_per_min == 0 {
            return (true, 0, 0);
        }
        let now = Instant::now();
        let cutoff = now - Duration::from_secs(60);
        let mut map = self.windows.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 && !map.contains_key(key) {
            map.retain(|_, q| q.back().is_some_and(|ts| *ts > cutoff));
            if map.len() > 10_000 {
                if let Some(first_key) = map.keys().next().cloned() {
                    map.remove(&first_key);
                }
            }
        }
        let q = map.entry(key.to_string()).or_default();
        while q.front().is_some_and(|ts| *ts <= cutoff) {
            q.pop_front();
        }
        let limit = max_per_min as usize;
        if q.len() >= limit {
            let reset_secs = q
                .front()
                .map(|oldest| {
                    (Duration::from_secs(60) - now.saturating_duration_since(*oldest))
                        .as_secs()
                        .max(1)
                })
                .unwrap_or(60);
            return (false, 0, reset_secs);
        }
        q.push_back(now);
        let remaining = limit.saturating_sub(q.len()) as u32;
        (true, remaining, 60)
    }
}

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

/// Hash a route Basic Auth credential pair using domain-separated SHA-256.
pub fn hash_basic_auth(user: &str, password: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"ferropod:basic-auth:v1:");
    hasher.update(user.as_bytes());
    hasher.update(b":");
    hasher.update(password.as_bytes());
    hex::encode(hasher.finalize())
}

/// Verify HTTP `Authorization: Basic ...` header in constant time against stored credential hash.
pub fn verify_basic_auth(
    headers: &axum::http::HeaderMap,
    expected_user: &str,
    expected_hash: &str,
) -> bool {
    use base64::Engine as _;
    let Some(raw_auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    else {
        return false;
    };
    let Some(b64) = raw_auth
        .strip_prefix("Basic ")
        .or_else(|| raw_auth.strip_prefix("basic "))
    else {
        return false;
    };
    let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else {
        return false;
    };
    let Ok(cred) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Some((user, pass)) = cred.split_once(':') else {
        return false;
    };
    if user != expected_user {
        return false;
    }
    let actual_hash = hash_basic_auth(user, pass);
    if actual_hash.len() != expected_hash.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in actual_hash.bytes().zip(expected_hash.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Extract bearer token from `Authorization` header (or optional `?token=` query param).
pub fn extract_bearer_token(
    req_headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
) -> Option<String> {
    if let Some(auth) = req_headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
        {
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

struct ActiveRequestGuard(Arc<GatewayMetrics>);

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.0.active_http_requests.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Request ID injection, global concurrency limiting, and non-blocking access logging middleware.
pub async fn access_log_middleware(
    State(svc): State<PlatformService>,
    mut req: Request,
    next: Next,
) -> Response {
    let metrics = svc.metrics().clone();
    let permit = match svc.try_acquire_http_permit() {
        Some(p) => p,
        None => {
            metrics.rate_limited_total.fetch_add(1, Ordering::Relaxed);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorBody {
                    error: "server concurrency limit reached".into(),
                }),
            )
                .into_response();
        }
    };

    metrics.active_http_requests.fetch_add(1, Ordering::Relaxed);
    let _active_guard = ActiveRequestGuard(metrics.clone());

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
    let host = crate::gateway::extract_request_host(&req)
        .unwrap_or("")
        .to_string();

    let started = Instant::now();
    let mut res = next.run(req).await;
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = res.status().as_u16();
    metrics.record_http(status, elapsed_ms);
    if let Ok(hv) = HeaderValue::from_str(&req_id) {
        res.headers_mut().insert("x-request-id", hv);
    }

    // Skip logging internal loopback TLS checks and healthy /_healthz probes to avoid log churn
    let skip_log = path == "/_internal/tls/allow" || (path == "/_healthz" && status < 400);
    if !skip_log {
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
