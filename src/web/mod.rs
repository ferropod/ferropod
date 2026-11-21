use std::collections::HashMap;
use std::sync::OnceLock;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

pub mod api;

const ADMIN_HTML: &str = include_str!("assets/index.html");
const ADMIN_CSS: &str = include_str!("assets/styles.css");
const ADMIN_I18N: &str = include_str!("assets/i18n.js");
const ADMIN_SEARCH_COMPONENTS: &str = include_str!("assets/search_components.js");
const ADMIN_PERF_COMPONENT: &str = include_str!("assets/perf_component.js");
const ADMIN_OPS_COMPONENT: &str = include_str!("assets/ops_component.js");
const ADMIN_RUNTIME_COMPONENT: &str = include_str!("assets/runtime_component.js");
const ADMIN_UPDATE_COMPONENT: &str = include_str!("assets/update_component.js");
const ADMIN_JS: &str = include_str!("assets/app.js");

fn asset_etags() -> &'static HashMap<&'static str, String> {
    static ETAGS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    ETAGS.get_or_init(|| {
        let items: &[(&'static str, &'static str)] = &[
            ("index.html", ADMIN_HTML),
            ("styles.css", ADMIN_CSS),
            ("i18n.js", ADMIN_I18N),
            ("search_components.js", ADMIN_SEARCH_COMPONENTS),
            ("perf_component.js", ADMIN_PERF_COMPONENT),
            ("ops_component.js", ADMIN_OPS_COMPONENT),
            ("runtime_component.js", ADMIN_RUNTIME_COMPONENT),
            ("update_component.js", ADMIN_UPDATE_COMPONENT),
            ("app.js", ADMIN_JS),
        ];
        items
            .iter()
            .map(|(name, body)| {
                let mut h = Sha256::new();
                h.update(body.as_bytes());
                let digest = hex::encode(h.finalize());
                (*name, format!("\"{}\"", &digest[..16]))
            })
            .collect()
    })
}

fn serve_embedded(
    headers: &HeaderMap,
    key: &'static str,
    content_type: &'static str,
    body: &'static str,
) -> Response {
    let etag = asset_etags()
        .get(key)
        .map(String::as_str)
        .unwrap_or("\"0\"");

    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if inm == etag || inm.split(',').any(|part| part.trim() == etag) {
            let mut resp = StatusCode::NOT_MODIFIED.into_response();
            if let Ok(val) = HeaderValue::from_str(etag) {
                resp.headers_mut().insert(header::ETAG, val);
            }
            resp.headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            return resp;
        }
    }

    let mut resp = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
            (header::VARY, "Accept-Encoding"),
        ],
        body,
    )
        .into_response();
    if let Ok(val) = HeaderValue::from_str(etag) {
        resp.headers_mut().insert(header::ETAG, val);
    }
    resp
}

pub async fn admin_index(headers: HeaderMap) -> Response {
    serve_embedded(
        &headers,
        "index.html",
        "text/html; charset=utf-8",
        ADMIN_HTML,
    )
}

pub async fn admin_asset(Path(file): Path<String>, headers: HeaderMap) -> Response {
    let (key, content_type, body) = match file.as_str() {
        "styles.css" => ("styles.css", "text/css; charset=utf-8", ADMIN_CSS),
        "i18n.js" => (
            "i18n.js",
            "application/javascript; charset=utf-8",
            ADMIN_I18N,
        ),
        "search_components.js" => (
            "search_components.js",
            "application/javascript; charset=utf-8",
            ADMIN_SEARCH_COMPONENTS,
        ),
        "perf_component.js" => (
            "perf_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_PERF_COMPONENT,
        ),
        "ops_component.js" => (
            "ops_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_OPS_COMPONENT,
        ),
        "runtime_component.js" => (
            "runtime_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_RUNTIME_COMPONENT,
        ),
        "update_component.js" => (
            "update_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_UPDATE_COMPONENT,
        ),
        "app.js" => ("app.js", "application/javascript; charset=utf-8", ADMIN_JS),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    serve_embedded(&headers, key, content_type, body)
}
