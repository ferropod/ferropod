use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

pub mod api;

const ADMIN_HTML: &str = include_str!("assets/index.html");
const ADMIN_CSS: &str = include_str!("assets/styles.css");
const ADMIN_I18N: &str = include_str!("assets/i18n.js");
const ADMIN_JS: &str = include_str!("assets/app.js");

pub async fn admin_index() -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        ADMIN_HTML,
    )
        .into_response()
}

pub async fn admin_asset(Path(file): Path<String>) -> Response {
    let (content_type, body) = match file.as_str() {
        "styles.css" => ("text/css; charset=utf-8", ADMIN_CSS),
        "i18n.js" => ("application/javascript; charset=utf-8", ADMIN_I18N),
        "app.js" => ("application/javascript; charset=utf-8", ADMIN_JS),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}
