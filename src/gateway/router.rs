use std::net::SocketAddr;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use serde::Serialize;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use crate::core::model::{RequestCtx, TargetEnv, WorkerHttpRequest};
use crate::core::service::{PlatformService, DEFAULT_ROUTE_IP_RPM};
use crate::gateway::{
    blob_api::{self, etag_matches},
    guard::{
        access_log_middleware, client_ip_from_headers, panel_domain_guard_middleware,
        verify_basic_auth,
    },
    local_api::{
        forward_dynamic_upstream, has_unsafe_path_segments, strip_hop_headers, LocalApiProxy,
    },
};
use crate::web;

#[derive(Debug, Serialize)]
struct HealthzResp {
    status: &'static str,
}

/// Build the unified HTTP router covering healthz, Blob API, REST API,
/// zero-build Admin UI, and Host-routed static sites / Workers.
pub fn build_router(svc: PlatformService) -> Router {
    let blob_router = blob_api::router().layer(RequestBodyLimitLayer::new(
        svc.config().max_blob_upload_bytes,
    ));
    let local_api = svc
        .config()
        .local_api
        .as_ref()
        .map(|cfg| LocalApiProxy::new(cfg, svc.config().max_blob_upload_bytes));

    let mcp_routes = Router::new()
        .route("/_mcp", post(web::mcp::mcp_http_handler))
        .route("/mcp", post(web::mcp::mcp_http_handler))
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    let panel_routes = Router::new()
        .nest("/_api/v1", web::api::router())
        .merge(mcp_routes)
        .route("/_panel", get(web::admin_index))
        .route("/_panel/", get(web::admin_index))
        .route("/_panel/assets/{file}", get(web::admin_asset))
        .layer(middleware::from_fn_with_state(
            svc.clone(),
            panel_domain_guard_middleware,
        ));

    let ai_realtime_routes = Router::new()
        .route("/_ai/v1/models", get(web::api::ai_list_models))
        .route(
            "/_ai/v1/chat/completions",
            post(web::api::ai_chat_completions),
        )
        .route("/_ai/v1/embeddings", post(web::api::ai_embeddings))
        .route("/_realtime/{channel}/sse", get(web::api::realtime_sse))
        .route(
            "/_realtime/{channel}/publish",
            post(web::api::realtime_publish),
        )
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024));

    Router::new()
        .route(
            "/_healthz",
            get(|| async { Json(HealthzResp { status: "ok" }) }),
        )
        .nest("/_blob", blob_router)
        .merge(ai_realtime_routes)
        .merge(panel_routes)
        .fallback(host_route_handler)
        .layer(Extension(local_api))
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(
            svc.clone(),
            access_log_middleware,
        ))
        .with_state(svc)
}

fn attach_rate_limit_headers(resp: &mut Response, info: Option<(u32, u32)>) {
    if let Some((limit, remaining)) = info {
        if let Ok(v) = HeaderValue::from_str(&limit.to_string()) {
            resp.headers_mut().insert("x-ratelimit-limit", v);
        }
        if let Ok(v) = HeaderValue::from_str(&remaining.to_string()) {
            resp.headers_mut().insert("x-ratelimit-remaining", v);
        }
    }
}

async fn host_route_handler(
    State(svc): State<PlatformService>,
    Extension(local_api): Extension<Option<LocalApiProxy>>,
    req: Request<Body>,
) -> Response {
    let req_path = req.uri().path();
    if has_unsafe_path_segments(req_path) {
        return (StatusCode::BAD_REQUEST, "invalid request path").into_response();
    }
    if req_path == "/_internal" || req_path.starts_with("/_internal/") {
        return (StatusCode::FORBIDDEN, "internal path forbidden").into_response();
    }

    let host = crate::gateway::extract_request_host(&req).unwrap_or("");

    let Some(route) = svc.lookup_route(host).await else {
        return (StatusCode::NOT_FOUND, "route not found").into_response();
    };

    // 1. Optional Preview / Site HTTP Basic Authentication gate
    if let (Some(expected_user), Some(expected_hash)) = (
        route.basic_auth_user.as_deref(),
        route.basic_auth_pass_hash.as_deref(),
    ) {
        if !verify_basic_auth(req.headers(), expected_user, expected_hash) {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Basic realm=\"ferropod\"")],
                "authentication required",
            )
                .into_response();
        }
    }

    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip());

    // 2. Per-route sliding-window rate limiting (fallback to DEFAULT_ROUTE_IP_RPM)
    let effective_route_rpm = route
        .rate_limit_rpm
        .filter(|r| *r > 0)
        .unwrap_or(DEFAULT_ROUTE_IP_RPM);
    let rate_info = if effective_route_rpm > 0 {
        let ip =
            client_ip_from_headers(req.headers(), peer_ip, svc.effective_trust_proxy_headers());
        let rl_key = format!("route:{}:{ip}", route.host);
        let (allowed, remaining, reset_secs) = svc
            .rate_limiter()
            .check_and_record(&rl_key, effective_route_rpm);
        if !allowed {
            svc.metrics()
                .rate_limited_total
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [
                    ("retry-after", reset_secs.to_string()),
                    ("x-ratelimit-limit", effective_route_rpm.to_string()),
                    ("x-ratelimit-remaining", "0".to_string()),
                ],
                "route rate limit exceeded",
            )
                .into_response();
        }
        Some((effective_route_rpm, remaining))
    } else {
        None
    };

    let Some(active_dep) = route.active_deployment else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "project has no active deployment",
        )
            .into_response();
    };

    // 3. Dynamic per-route reverse proxy (prefix + upstream) or boot-time --local-api proxy
    if let (Some(prefix), Some(upstream)) = (
        route.proxy_prefix.as_deref(),
        route.proxy_upstream.as_deref(),
    ) {
        if req_path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        {
            let Some(_app_permit) = svc.try_acquire_app_permit(&route.project_id) else {
                svc.metrics()
                    .rate_limited_total
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return (
                    StatusCode::TOO_MANY_REQUESTS,
                    "app concurrency limit reached",
                )
                    .into_response();
            };
            let mut resp =
                forward_dynamic_upstream(upstream, svc.config().max_blob_upload_bytes, req).await;
            attach_rate_limit_headers(&mut resp, rate_info);
            return resp;
        }
    }

    if let Some(proxy) = local_api.filter(|proxy| proxy.matches(&route.host, req_path)) {
        let mut resp = proxy.forward(req).await;
        attach_rate_limit_headers(&mut resp, rate_info);
        return resp;
    }

    // 4. Optional deterministic canary weight split (smooth interleaved pacing)
    let (dep, is_canary) = match route.canary_deployment.filter(|_| route.canary_weight > 0) {
        Some(canary_dep) => {
            let slot = svc.next_canary_ticket() % 100;
            let w = u64::from(route.canary_weight.min(100));
            let hit = ((slot + 1) * w / 100) > (slot * w / 100);
            if hit {
                (canary_dep, true)
            } else {
                (active_dep, false)
            }
        }
        None => (active_dep, false),
    };
    let served_dep_id = dep.id.clone();

    let method = req.method().clone();
    let headers = req.headers().clone();
    let req_path_owned = req.uri().path().to_string();
    let has_worker = dep.artifact_path.is_some();

    let finalize_resp = |mut resp: Response| -> Response {
        if let Ok(v) = HeaderValue::from_str(&served_dep_id) {
            resp.headers_mut().insert("x-ferropod-deployment", v);
        }
        resp.headers_mut().insert(
            "x-ferropod-canary",
            HeaderValue::from_static(if is_canary { "true" } else { "false" }),
        );
        attach_rate_limit_headers(&mut resp, rate_info);
        resp
    };

    if let Some(bucket) = dep.assets_bucket.as_deref() {
        if method == Method::GET || method == Method::HEAD {
            if let Some(resp) = try_serve_static(
                &svc,
                bucket,
                dep.assets_prefix.as_deref(),
                if has_worker { false } else { dep.spa_fallback },
                &req_path_owned,
                &headers,
                method == Method::HEAD,
            )
            .await
            {
                return finalize_resp(resp);
            }
        }
    }

    if has_worker {
        let max_worker_req_bytes = svc.config().max_blob_upload_bytes.min(32 * 1024 * 1024);
        if let Some(content_length) = headers
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            if content_length > max_worker_req_bytes as u64 {
                return finalize_resp(
                    (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
                );
            }
        }

        let uri_str = req
            .uri()
            .path_and_query()
            .map(|pq| pq.as_str().to_string())
            .unwrap_or_else(|| "/".to_string());
        let header_pairs = headers
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
            .collect();
        let collected_body = match tokio::time::timeout(
            Duration::from_secs(30),
            Limited::new(req.into_body(), max_worker_req_bytes).collect(),
        )
        .await
        {
            Ok(Ok(collected)) => collected.to_bytes(),
            Ok(Err(err)) => {
                if err.is::<LengthLimitError>() {
                    return finalize_resp(
                        (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
                    );
                }
                return finalize_resp(
                    (StatusCode::BAD_REQUEST, "failed to read request body").into_response(),
                );
            }
            Err(_) => {
                return finalize_resp(
                    (StatusCode::REQUEST_TIMEOUT, "request body read timed out").into_response(),
                );
            }
        };
        let body_stream =
            futures_util::stream::once(async move { Ok::<_, std::io::Error>(collected_body) });
        let worker_req = WorkerHttpRequest {
            method: method.as_str().to_string(),
            uri: uri_str,
            headers: header_pairs,
            body: Box::pin(body_stream),
        };
        let request_id = headers
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("req_{}", Uuid::new_v4().simple()));
        let ctx = RequestCtx::new(route.project_id.clone(), request_id, dep.clone());
        let resp = match svc.invoke_worker_http(ctx, worker_req).await {
            Ok(resp) => {
                let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::OK);
                let mut resp_headers = HeaderMap::new();
                for (k, v) in resp.headers {
                    if let (Ok(name), Ok(val)) = (
                        HeaderName::from_bytes(k.as_bytes()),
                        HeaderValue::from_bytes(&v),
                    ) {
                        resp_headers.append(name, val);
                    }
                }
                let is_json_content = resp_headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|ct| {
                        ct.trim()
                            .to_ascii_lowercase()
                            .starts_with("application/json")
                    });

                if status == StatusCode::NOT_FOUND
                    && dep.spa_fallback
                    && (method == Method::GET || method == Method::HEAD)
                    && !req_path_owned.contains('.')
                    && !is_json_content
                {
                    if let Some(bucket) = dep.assets_bucket.as_deref() {
                        if let Some(spa_resp) = try_serve_static(
                            &svc,
                            bucket,
                            dep.assets_prefix.as_deref(),
                            false,
                            "/index.html",
                            &headers,
                            method == Method::HEAD,
                        )
                        .await
                        {
                            return finalize_resp(spa_resp);
                        }
                    }
                }

                strip_hop_headers(&mut resp_headers);
                resp_headers.remove(header::CONTENT_LENGTH);
                let mut builder = Response::builder().status(status);
                if let Some(h) = builder.headers_mut() {
                    *h = resp_headers;
                }
                builder
                    .body(Body::from_stream(resp.body))
                    .unwrap_or_else(|_| {
                        (StatusCode::BAD_GATEWAY, "invalid worker response").into_response()
                    })
            }
            Err(err) => {
                let msg = err.to_string();
                let code = if msg.contains("timed out") || msg.contains("interrupt") {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::BAD_GATEWAY
                };
                (code, msg).into_response()
            }
        };

        let out = finalize_resp(resp);
        if !is_canary && route.target_env == TargetEnv::Prod {
            let is_5xx = out.status().as_u16() >= 500;
            let eff = svc.effective_settings();
            if svc.health_tracker().record_outcome_with_policy(
                &served_dep_id,
                is_5xx,
                eff.auto_rollback_enabled,
                eff.auto_rollback_window_secs,
                eff.auto_rollback_min_requests,
                eff.auto_rollback_error_rate_pct,
            ) {
                let _ = svc
                    .auto_rollback_deployment(&route.project_id, &served_dep_id)
                    .await;
            }
        }
        return out;
    }

    finalize_resp((StatusCode::NOT_FOUND, "not found").into_response())
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
    match prefix
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
    {
        Some(pfx) => format!("{pfx}/{rel}"),
        None => rel,
    }
}

fn attach_cache_control(
    mut builder: axum::http::response::Builder,
    cache_control: Option<&str>,
) -> axum::http::response::Builder {
    if let Some(cc) = cache_control {
        if let Ok(hv) = HeaderValue::from_str(cc) {
            builder = builder.header(header::CACHE_CONTROL, hv);
        }
    }
    builder
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

    if found.is_none() && spa_fallback && !req_path.contains('.') {
        let fallback_key = build_asset_key(prefix, "/index.html");
        found = svc
            .static_head_object(bucket, &fallback_key)
            .await
            .ok()
            .flatten();
    }

    let meta = found?;
    let cache_control = svc.static_bucket_cache_control(bucket).await;
    let etag = format!("\"{}\"", meta.sha256);
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if etag_matches(inm, &etag) {
            let builder = attach_cache_control(
                Response::builder()
                    .status(StatusCode::NOT_MODIFIED)
                    .header(header::ETAG, etag),
                cache_control.as_deref(),
            );
            return Some(builder.body(Body::empty()).unwrap());
        }
    }

    if is_head {
        let builder = attach_cache_control(
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, meta.content_type)
                .header(header::CONTENT_LENGTH, meta.size.to_string())
                .header(header::ETAG, etag)
                .header(header::ACCEPT_RANGES, "bytes"),
            cache_control.as_deref(),
        );
        return Some(builder.body(Body::empty()).unwrap());
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

    let Ok((meta, stream, len, span)) = svc.static_open_stream_for_meta(meta, range).await else {
        return None;
    };

    let mut builder = attach_cache_control(
        Response::builder()
            .status(if span.is_some() {
                StatusCode::PARTIAL_CONTENT
            } else {
                StatusCode::OK
            })
            .header(header::CONTENT_TYPE, meta.content_type)
            .header(header::CONTENT_LENGTH, len.to_string())
            .header(header::ETAG, etag)
            .header(header::ACCEPT_RANGES, "bytes"),
        cache_control.as_deref(),
    );

    if let Some((start, end)) = span {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", meta.size),
        );
    }

    Some(builder.body(Body::from_stream(stream)).unwrap())
}
