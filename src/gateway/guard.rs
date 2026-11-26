use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::Serialize;
use uuid::Uuid;

use crate::core::model::{Caller, FerroError, LogEntry};
use crate::core::service::{GatewayMetrics, PlatformService};

const RATE_LIMITER_SHARDS: usize = 16;
const MAX_KEYS_PER_SHARD: usize = 1024;
const RATE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
struct WindowSlot {
    window_start: Instant,
    prev_count: u32,
    curr_count: u32,
}

impl WindowSlot {
    fn effective_and_remaining(&mut self, now: Instant) -> (u32, u64) {
        let elapsed = now.saturating_duration_since(self.window_start);
        if elapsed >= RATE_WINDOW * 2 {
            self.prev_count = 0;
            self.curr_count = 0;
            self.window_start = now;
        } else if elapsed >= RATE_WINDOW {
            self.prev_count = self.curr_count;
            self.curr_count = 0;
            self.window_start += RATE_WINDOW;
        }

        let elapsed_secs = now
            .saturating_duration_since(self.window_start)
            .as_secs()
            .min(60);
        let remaining_secs = 60 - elapsed_secs;
        let weighted_prev = ((u64::from(self.prev_count) * remaining_secs) / 60) as u32;
        let effective = self.curr_count.saturating_add(weighted_prev);
        (effective, remaining_secs.max(1))
    }
}

/// Sharded O(1)-memory sliding-window per-key rate limiter (60-second window).
#[derive(Debug)]
pub struct SlidingWindowRateLimiter {
    shards: [Mutex<HashMap<String, WindowSlot>>; RATE_LIMITER_SHARDS],
}

impl Default for SlidingWindowRateLimiter {
    fn default() -> Self {
        Self {
            shards: std::array::from_fn(|_| Mutex::new(HashMap::new())),
        }
    }
}

impl SlidingWindowRateLimiter {
    fn shard_index(key: &str) -> usize {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() as usize) % RATE_LIMITER_SHARDS
    }

    pub fn is_limited(&self, key: &str, max_per_min: u32) -> (bool, u64) {
        if max_per_min == 0 {
            return (false, 0);
        }
        let now = Instant::now();
        let shard = &self.shards[Self::shard_index(key)];
        let map = shard.lock().unwrap_or_else(|e| e.into_inner());
        let Some(&slot_copy) = map.get(key) else {
            return (false, 0);
        };
        let mut slot = slot_copy;
        let (effective, reset_secs) = slot.effective_and_remaining(now);
        (effective >= max_per_min, reset_secs)
    }

    pub fn check_and_record(&self, key: &str, max_per_min: u32) -> (bool, u32, u64) {
        if max_per_min == 0 {
            return (true, 0, 0);
        }
        let now = Instant::now();
        let shard = &self.shards[Self::shard_index(key)];
        let mut map = shard.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= MAX_KEYS_PER_SHARD && !map.contains_key(key) {
            let expiry = RATE_WINDOW * 2;
            map.retain(|_, slot| now.saturating_duration_since(slot.window_start) < expiry);
            if map.len() >= MAX_KEYS_PER_SHARD {
                if let Some(first_key) = map.keys().next().cloned() {
                    map.remove(&first_key);
                }
            }
        }
        let slot = map.entry(key.to_string()).or_insert(WindowSlot {
            window_start: now,
            prev_count: 0,
            curr_count: 0,
        });
        let (effective, reset_secs) = slot.effective_and_remaining(now);
        if effective >= max_per_min {
            return (false, 0, reset_secs);
        }

        slot.curr_count = slot.curr_count.saturating_add(1);
        let remaining = max_per_min.saturating_sub(effective + 1);
        (true, remaining, reset_secs)
    }
}

/// Extract client IP from proxy headers (when trusted) or TCP peer address.
pub fn client_ip_from_headers(
    headers: &HeaderMap,
    peer_ip: Option<IpAddr>,
    trust_proxy: bool,
) -> String {
    if trust_proxy {
        if let Some(xff) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
            if let Some(first) = xff
                .split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return first.to_string();
            }
        }
        if let Some(real_ip) = headers.get("x-real-ip").and_then(|v| v.to_str().ok()) {
            let t = real_ip.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    peer_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "anon".to_string())
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

/// Restrict control-plane/admin routes (`/_panel`, `/_api/v1`, `/_mcp`) to the configured `panel_domain`
/// (plus loopback hosts for local CLI/recovery access) when `restrict_panel_to_domain` is enabled.
pub async fn panel_domain_guard_middleware(
    State(svc): State<PlatformService>,
    req: Request,
    next: Next,
) -> Response {
    let Some(allowed_domain) = svc.effective_panel_restriction() else {
        return next.run(req).await;
    };
    let Some(raw_host) = crate::gateway::extract_request_host(&req) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    if crate::core::service::is_loopback_host(raw_host)
        || crate::core::service::normalize_host(raw_host) == allowed_domain
    {
        return next.run(req).await;
    }
    (StatusCode::NOT_FOUND, "not found").into_response()
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

    const AUTH_FAIL_RPM: u32 = 30;
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());
    let client_ip =
        client_ip_from_headers(req.headers(), peer_ip, svc.effective_trust_proxy_headers());
    let is_auth_precheck_path = path.starts_with("/_api/v1/")
        || path == "/_mcp"
        || path == "/mcp"
        || path.starts_with("/_ai/")
        || path.starts_with("/_realtime/");
    let is_auth_record_path = is_auth_precheck_path || path.starts_with("/_blob/");
    let auth_fail_key = format!("auth_fail:{client_ip}");

    if is_auth_precheck_path {
        let (limited, retry_after) = svc.rate_limiter().is_limited(&auth_fail_key, AUTH_FAIL_RPM);
        if limited {
            metrics.rate_limited_total.fetch_add(1, Ordering::Relaxed);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", retry_after.to_string())],
                Json(ErrorBody {
                    error: "too many failed authentication attempts, please retry later".into(),
                }),
            )
                .into_response();
        }
    }

    let started = Instant::now();
    let mut res = next.run(req).await;
    drop(permit);

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let status = res.status().as_u16();
    if status == 401 && is_auth_record_path {
        svc.rate_limiter()
            .check_and_record(&auth_fail_key, AUTH_FAIL_RPM);
    }
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

#[cfg(test)]
mod tests {
    use super::SlidingWindowRateLimiter;

    #[test]
    fn rate_limiter_enforces_limit_and_isolates_keys() {
        let rl = SlidingWindowRateLimiter::default();

        // max_per_min == 0 disables rate limiting.
        assert_eq!(rl.check_and_record("route:unlimited:1.2.3.4", 0), (true, 0, 0));

        // Allows up to max_per_min for key A and rejects the next request.
        assert_eq!(rl.check_and_record("route:a:1.1.1.1", 3), (true, 2, 60));
        assert_eq!(rl.check_and_record("route:a:1.1.1.1", 3), (true, 1, 60));
        assert_eq!(rl.check_and_record("route:a:1.1.1.1", 3), (true, 0, 60));
        let (allowed, remaining, reset_secs) = rl.check_and_record("route:a:1.1.1.1", 3);
        assert!(!allowed);
        assert_eq!(remaining, 0);
        assert!((1..=60).contains(&reset_secs));

        // Independent keys (across shards) remain unaffected.
        for i in 0..32 {
            let key = format!("route:b:10.0.0.{i}");
            let (ok, rem, _) = rl.check_and_record(&key, 2);
            assert!(ok);
            assert_eq!(rem, 1);
        }
    }
}
