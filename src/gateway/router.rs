use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderMap, Method, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;

use crate::core::service::PlatformService;
use crate::gateway::{blob_api, guard::access_log_middleware};
use crate::web;

#[derive(Debug, Serialize)]
struct HealthzResp {
    status: &'static str,
}

/// Build the unified HTTP router covering healthz, Blob API, REST API,
/// zero-build Admin UI, and Host-routed static sites / Workers.
pub fn build_router(svc: PlatformService) -> Router {
    let blob_router = blob_api::router()
        .layer(RequestBodyLimitLayer::new(svc.config().max_blob_upload_bytes));

    Router::new()
        .route("/_healthz", get(|| async { Json(HealthzResp { status: "ok" }) }))
        .nest("/_blob", blob_router)
        .nest("/_api/v1", web::api::router())
        .route("/_admin", get(web::admin_index))
        .route("/_admin/", get(web::admin_index))
        .route("/_admin/assets/{file}", get(web::admin_asset))
        .fallback(host_route_handler)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            svc.clone(),
            access_log_middleware,
        ))
        .with_state(svc)
}

async fn host_route_handler(
    State(svc): State<PlatformService>,
    req: Request<Body>,
) -> Response {
    let method = req.method().clone();
    let headers = req.headers().clone();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let Some(route) = svc.lookup_route(host).await else {
        return (StatusCode::NOT_FOUND, "route not found").into_response();
    };

    let Some(dep) = route.active_deployment else {
        return (StatusCode::SERVICE_UNAVAILABLE, "project has no active deployment").into_response();
    };

    if let Some(bucket) = dep.assets_bucket.as_deref() {
        if method == Method::GET || method == Method::HEAD {
            if let Some(resp) =
                try_serve_static(&svc, bucket, dep.assets_prefix.as_deref(), dep.spa_fallback, req.uri().path(), &headers, method == Method::HEAD).await
            {
                return resp;
            }
        }
    }

    if dep.artifact_path.is_some() {
        return (
            StatusCode::NOT_IMPLEMENTED,
            "Wasmtime worker execution will be enabled in M2",
        )
            .into_response();
    }

    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn build_asset_key(prefix: Option<&str>, req_path: &str) -> String {
    let clean = req_path.trim_start_matches('/');
    let rel = if clean.is_empty() {
        "index.html".to_string()
    } else if clean.ends_with('/') {
        format!("{clean}index.html")
    } else {
        clean.to_string()
    };
    match prefix.map(|p| p.trim_matches('/')).filter(|p| !p.is_empty()) {
        Some(pfx) => format!("{pfx}/{rel}"),
        None => rel,
    }
}

async fn try_serve_static(
    svc: &PlatformService,
    bucket: &str,
    prefix: Option<&str>,
    spa_fallback: bool,
    req_path: &str,
    headers: &HeaderMap,
    is_head: bool,
) -> Option<Response> {
    let primary_key = build_asset_key(prefix, req_path);
    let mut found = svc
        .static_head_object(bucket, &primary_key)
        .await
        .ok()
        .flatten();
    let mut resolved_key = primary_key;

    if found.is_none() && spa_fallback && !req_path.contains('.') {
        let fallback_key = build_asset_key(prefix, "/index.html");
        found = svc
            .static_head_object(bucket, &fallback_key)
            .await
            .ok()
            .flatten();
        resolved_key = fallback_key;
    }

    let meta = found?;
    let etag = format!("\"{}\"", meta.sha256);
    if let Some(inm) = headers.get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()) {
        if inm.trim() == etag || inm.trim() == "*" {
            return Some(
                Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(header::ETAG, etag)
                    .body(Body::empty())
                    .unwrap(),
            );
        }
    }

    if is_head {
        return Some(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, meta.content_type)
                .header(header::CONTENT_LENGTH, meta.size.to_string())
                .header(header::ETAG, etag)
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::empty())
                .unwrap(),
        );
    }

    let range = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(raw) => match blob_api::parse_byte_range(raw, meta.size) {
            Some(r) => Some(r),
            None => {
                return Some(
                    Response::builder()
                        .status(StatusCode::RANGE_NOT_SATISFIABLE)
                        .header(header::CONTENT_RANGE, format!("bytes */{}", meta.size))
                        .body(Body::empty())
                        .unwrap(),
                );
            }
        },
        None => None,
    };

    let Ok(Some((meta, stream, len, span))) = svc
        .static_get_object_stream(bucket, &resolved_key, range)
        .await
    else {
        return None;
    };

    let mut builder = Response::builder()
        .status(if span.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, meta.content_type)
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::ETAG, etag)
        .header(header::ACCEPT_RANGES, "bytes");

    if let Some((start, end)) = span {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", meta.size),
        );
    }

    Some(builder.body(Body::from_stream(stream)).unwrap())
}
