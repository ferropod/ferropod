use std::time::Duration;

use axum::{
    body::Body,
    http::{
        header, uri::Authority, HeaderMap, HeaderName, Method, Request, StatusCode, Uri, Version,
    },
    response::{IntoResponse, Response},
};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};

use crate::config::LocalApiConfig;

#[derive(Clone)]
pub(super) struct LocalApiProxy {
    host: String,
    prefix: String,
    upstream: Authority,
    client: Client<HttpConnector, Body>,
}

impl LocalApiProxy {
    pub(super) fn new(config: &LocalApiConfig) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(Duration::from_secs(5)));
        Self {
            host: config.host.clone(),
            prefix: config.prefix.clone(),
            upstream: config
                .upstream
                .to_string()
                .parse()
                .expect("valid socket address"),
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    pub(super) fn matches(&self, host: &str, path: &str) -> bool {
        host == self.host
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

        let uri = Uri::builder()
            .scheme("http")
            .authority(self.upstream.clone())
            .path_and_query(
                req.uri()
                    .path_and_query()
                    .expect("matched a request path")
                    .clone(),
            )
            .build()
            .expect("valid upstream and request path");
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

        match tokio::time::timeout(Duration::from_secs(30), self.client.request(req)).await {
            Ok(Ok(mut res)) => {
                strip_hop_headers(res.headers_mut());
                res.map(Body::new)
            }
            Ok(Err(err)) => {
                tracing::debug!(%err, upstream = %self.upstream, "local API request failed");
                (StatusCode::BAD_GATEWAY, "local API unavailable").into_response()
            }
            Err(_) => (StatusCode::GATEWAY_TIMEOUT, "local API request timed out").into_response(),
        }
    }
}

fn strip_hop_headers(headers: &mut HeaderMap) {
    let connection_headers: Vec<_> = headers
        .get_all(header::CONNECTION)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .filter_map(|name| HeaderName::from_bytes(name.trim_ascii()).ok())
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
