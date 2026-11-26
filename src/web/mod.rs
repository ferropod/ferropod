use std::collections::HashMap;
use std::sync::OnceLock;

use axum::{
    extract::Path,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

pub mod api;
pub mod mcp;
pub mod openapi;

const ADMIN_HTML: &str = include_str!("assets/index.html");
const ADMIN_CSS: &str = include_str!("assets/styles.css");
const ADMIN_I18N: &str = include_str!("assets/i18n.js");
const ADMIN_I18N_ZH_TW: &str = include_str!("assets/i18n_zh_tw.json");
const ADMIN_I18N_JA: &str = include_str!("assets/i18n_ja.json");
const ADMIN_I18N_KO: &str = include_str!("assets/i18n_ko.json");
const ADMIN_I18N_ES: &str = include_str!("assets/i18n_es.json");
const ADMIN_SEARCH_COMPONENTS: &str = include_str!("assets/search_components.js");
const ADMIN_PERF_COMPONENT: &str = include_str!("assets/perf_component.js");
const ADMIN_OPS_COMPONENT: &str = include_str!("assets/ops_component.js");
const ADMIN_RUNTIME_COMPONENT: &str = include_str!("assets/runtime_component.js");
const ADMIN_UPDATE_COMPONENT: &str = include_str!("assets/update_component.js");
const ADMIN_SETTINGS_COMPONENT: &str = include_str!("assets/settings_component.js");
const ADMIN_AI_COMPONENT: &str = include_str!("assets/ai_component.js");
const ADMIN_VIEW_KV: &str = include_str!("assets/view_kv.js");
const ADMIN_VIEW_QUEUES: &str = include_str!("assets/view_queues.js");
const ADMIN_VIEW_BLOBS: &str = include_str!("assets/view_blobs.js");
const ADMIN_VIEW_DBS: &str = include_str!("assets/view_dbs.js");
const ADMIN_VIEW_SEARCH: &str = include_str!("assets/view_search.js");
const ADMIN_JS: &str = include_str!("assets/app.js");

fn asset_etags() -> &'static HashMap<&'static str, String> {
    static ETAGS: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    ETAGS.get_or_init(|| {
        let items: &[(&'static str, &'static str)] = &[
            ("index.html", ADMIN_HTML),
            ("styles.css", ADMIN_CSS),
            ("i18n.js", ADMIN_I18N),
            ("i18n_zh_tw.json", ADMIN_I18N_ZH_TW),
            ("i18n_ja.json", ADMIN_I18N_JA),
            ("i18n_ko.json", ADMIN_I18N_KO),
            ("i18n_es.json", ADMIN_I18N_ES),
            ("search_components.js", ADMIN_SEARCH_COMPONENTS),
            ("perf_component.js", ADMIN_PERF_COMPONENT),
            ("ops_component.js", ADMIN_OPS_COMPONENT),
            ("runtime_component.js", ADMIN_RUNTIME_COMPONENT),
            ("update_component.js", ADMIN_UPDATE_COMPONENT),
            ("settings_component.js", ADMIN_SETTINGS_COMPONENT),
            ("ai_component.js", ADMIN_AI_COMPONENT),
            ("view_kv.js", ADMIN_VIEW_KV),
            ("view_queues.js", ADMIN_VIEW_QUEUES),
            ("view_blobs.js", ADMIN_VIEW_BLOBS),
            ("view_dbs.js", ADMIN_VIEW_DBS),
            ("view_search.js", ADMIN_VIEW_SEARCH),
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
        "i18n_zh_tw.json" => (
            "i18n_zh_tw.json",
            "application/json; charset=utf-8",
            ADMIN_I18N_ZH_TW,
        ),
        "i18n_ja.json" => (
            "i18n_ja.json",
            "application/json; charset=utf-8",
            ADMIN_I18N_JA,
        ),
        "i18n_ko.json" => (
            "i18n_ko.json",
            "application/json; charset=utf-8",
            ADMIN_I18N_KO,
        ),
        "i18n_es.json" => (
            "i18n_es.json",
            "application/json; charset=utf-8",
            ADMIN_I18N_ES,
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
        "settings_component.js" => (
            "settings_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_SETTINGS_COMPONENT,
        ),
        "ai_component.js" => (
            "ai_component.js",
            "application/javascript; charset=utf-8",
            ADMIN_AI_COMPONENT,
        ),
        "view_kv.js" => (
            "view_kv.js",
            "application/javascript; charset=utf-8",
            ADMIN_VIEW_KV,
        ),
        "view_queues.js" => (
            "view_queues.js",
            "application/javascript; charset=utf-8",
            ADMIN_VIEW_QUEUES,
        ),
        "view_blobs.js" => (
            "view_blobs.js",
            "application/javascript; charset=utf-8",
            ADMIN_VIEW_BLOBS,
        ),
        "view_dbs.js" => (
            "view_dbs.js",
            "application/javascript; charset=utf-8",
            ADMIN_VIEW_DBS,
        ),
        "view_search.js" => (
            "view_search.js",
            "application/javascript; charset=utf-8",
            ADMIN_VIEW_SEARCH,
        ),
        "app.js" => ("app.js", "application/javascript; charset=utf-8", ADMIN_JS),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    serve_embedded(&headers, key, content_type, body)
}
