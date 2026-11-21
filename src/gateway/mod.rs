use axum::http::{header, Request};

pub mod blob_api;
pub mod guard;
mod local_api;
pub mod router;
pub mod tls;

/// Extract the request target host from the HTTP `Host` header, falling back to HTTP/2 `:authority`.
pub(super) fn extract_request_host<B>(req: &Request<B>) -> Option<&str> {
    req.headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty())
        .or_else(|| req.uri().authority().map(|a| a.as_str()))
}
