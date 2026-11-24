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

pub(crate) fn has_unsafe_path_segments(path: &str) -> bool {
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

pub(super) fn strip_hop_headers(headers: &mut HeaderMap) {
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

fn dynamic_proxy_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("valid reqwest dynamic proxy client")
    })
}

/// Forward an HTTP request matching a dynamic per-route reverse proxy (`http://` or `https://`).
pub(super) async fn forward_dynamic_upstream(
    upstream_base: &str,
    max_body_bytes: usize,
    req: Request<Body>,
) -> Response {
    use axum::body::HttpBody as _;

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
        if content_length > max_body_bytes as u64 {
            return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response();
        }
    }

    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let target_url = format!("{}{}", upstream_base.trim_end_matches('/'), path_and_query);
    let Ok(parsed_method) = reqwest::Method::from_bytes(req.method().as_str().as_bytes()) else {
        return (StatusCode::BAD_REQUEST, "unsupported HTTP method").into_response();
    };

    let (parts, body) = req.into_parts();
    let mut req_headers = parts.headers;
    strip_hop_headers(&mut req_headers);
    for name in [
        "host",
        "content-length",
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-forwarded-port",
        "x-real-ip",
    ] {
        req_headers.remove(name);
    }

    let mut builder = dynamic_proxy_client().request(parsed_method, target_url);
    if body.size_hint().exact() != Some(0) {
        let limited_stream = Body::new(Limited::new(body, max_body_bytes)).into_data_stream();
        builder = builder.body(reqwest::Body::wrap_stream(limited_stream));
    }
    for (name, val) in &req_headers {
        if let Ok(v) = reqwest::header::HeaderValue::from_bytes(val.as_bytes()) {
            builder = builder.header(name.as_str(), v);
        }
    }

    match builder.send().await {
        Ok(up_resp) => {
            let status =
                StatusCode::from_u16(up_resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
            let mut out_headers = HeaderMap::new();
            for (k, v) in up_resp.headers() {
                if let (Ok(hn), Ok(hv)) = (
                    HeaderName::from_bytes(k.as_str().as_bytes()),
                    HeaderValue::from_bytes(v.as_bytes()),
                ) {
                    out_headers.append(hn, hv);
                }
            }
            strip_hop_headers(&mut out_headers);
            out_headers.remove(header::CONTENT_LENGTH);
            let mut resp = Response::builder().status(status);
            if let Some(h) = resp.headers_mut() {
                *h = out_headers;
            }
            resp.body(Body::from_stream(up_resp.bytes_stream()))
                .unwrap_or_else(|_| {
                    (StatusCode::BAD_GATEWAY, "invalid upstream response").into_response()
                })
        }
        Err(err) => {
            let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(&err);
            while let Some(source) = cur {
                if source.is::<LengthLimitError>() {
                    return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large")
                        .into_response();
                }
                cur = source.source();
            }
            if err.is_timeout() {
                (StatusCode::GATEWAY_TIMEOUT, "upstream proxy timed out").into_response()
            } else {
                (StatusCode::BAD_GATEWAY, "upstream proxy unavailable").into_response()
            }
        }
    }
}
