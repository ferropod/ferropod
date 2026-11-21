use std::time::Duration;

use axum::{
    body::Body,
    http::{
        header, uri::Authority, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode,
        Uri, Version,
    },
    response::{IntoResponse, Response},
};
use http_body_util::{LengthLimitError, Limited};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};

use crate::config::LocalApiConfig;
use crate::core::service::normalize_host;

#[derive(Clone)]
pub(super) struct LocalApiProxy {
    host: String,
    prefix: String,
    upstream: Authority,
    max_body_bytes: usize,
    client: Client<HttpConnector, Body>,
}

pub(super) fn has_unsafe_path_segments(path: &str) -> bool {
    if !path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return true;
    }
    let bytes = path.as_bytes();
    for window in bytes.windows(3) {
        if matches!(
            window,
            b"%2e" | b"%2E" | b"%2f" | b"%2F" | b"%5c" | b"%5C" | b"%00"
        ) {
            return true;
        }
    }
    path.split('/').any(|seg| seg == "." || seg == "..")
}

impl LocalApiProxy {
    pub(super) fn new(config: &LocalApiConfig, max_body_bytes: usize) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        Self {
            host: normalize_host(&config.host),
            prefix: config.prefix.clone(),
            upstream: config
                .upstream
                .to_string()
                .parse()
                .expect("valid socket address"),
            max_body_bytes,
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    pub(super) fn matches(&self, host: &str, path: &str) -> bool {
        normalize_host(host) == self.host
            && !has_unsafe_path_segments(path)
            && path
                .strip_prefix(&self.prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    pub(super) async fn forward(&self, mut req: Request<Body>) -> Response {
        if req.method() == Method::CONNECT || req.headers().contains_key(header::UPGRADE) {
            return (
                StatusCode::METHOD_NOT_ALLOWED,
                "HTTP tunnels are unsupported",
            )
                .into_response();
        }
        if has_unsafe_path_segments(req.uri().path()) {
            return (StatusCode::BAD_REQUEST, "invalid request path").into_response();
        }
        if let Some(content_length) = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            if content_length > self.max_body_bytes as u64 {
                return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response();
            }
        }

        let Some(path_and_query) = req.uri().path_and_query().cloned() else {
            return (StatusCode::BAD_REQUEST, "invalid request path").into_response();
        };
        let Ok(uri) = Uri::builder()
            .scheme("http")
            .authority(self.upstream.clone())
            .path_and_query(path_and_query)
            .build()
        else {
            return (StatusCode::BAD_REQUEST, "invalid request path").into_response();
        };

        if !req.headers().contains_key(header::HOST) {
            if let Some(authority) = req.uri().authority().map(|a| a.as_str().to_owned()) {
                if let Ok(hv) = HeaderValue::from_str(&authority) {
                    req.headers_mut().insert(header::HOST, hv);
                }
            }
        }

        *req.uri_mut() = uri;
        *req.version_mut() = Version::HTTP_11;
        strip_hop_headers(req.headers_mut());
        for name in [
            "forwarded",
            "x-forwarded-for",
            "x-forwarded-host",
            "x-forwarded-proto",
            "x-forwarded-port",
            "x-real-ip",
        ] {
            req.headers_mut().remove(name);
        }

        let body = std::mem::take(req.body_mut());
        *req.body_mut() = Body::new(Limited::new(body, self.max_body_bytes));

        match tokio::time::timeout(Duration::from_secs(30), self.client.request(req)).await {
            Ok(Ok(mut res)) => {
                strip_hop_headers(res.headers_mut());
                res.map(Body::new)
            }
            Ok(Err(err)) => {
                let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&err);
                while let Some(source) = cur {
                    if source.is::<LengthLimitError>() {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
                            .into_response();
                    }
                    cur = source.source();
                }
                tracing::debug!(%err, upstream = %self.upstream, "local API request failed");
                (StatusCode::BAD_GATEWAY, "local API unavailable").into_response()
            }
            Err(_) => (StatusCode::GATEWAY_TIMEOUT, "local API request timed out").into_response(),
        }
    }
}

fn is_protected_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host" | "content-length" | "content-type" | "authorization" | "cookie" | "x-request-id"
    )
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let connection_headers: Vec<_> = headers
        .get_all(header::CONNECTION)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .filter_map(|name| HeaderName::from_bytes(name.trim_ascii()).ok())
        .filter(|name| !is_protected_header(name))
        .collect();
    for name in connection_headers {
        headers.remove(name);
    }
    for name in [
        "connection",
        "keep-alive",
        "proxy-connection",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}
