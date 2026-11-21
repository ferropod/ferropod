use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use crate::core::service::PlatformService;

#[derive(Debug, Deserialize)]
pub struct TlsAllowQuery {
    pub domain: String,
}

/// Loopback endpoint called by Caddy `on_demand_tls.ask`.
/// Returns 200 OK only if the requested domain exists in the active route table.
pub async fn allow_domain(
    State(svc): State<PlatformService>,
    Query(q): Query<TlsAllowQuery>,
) -> impl IntoResponse {
    if svc.is_domain_allowed(&q.domain).await {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    }
}
