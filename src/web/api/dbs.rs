use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::core::model::{SqlValue, UserDbOpts};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateDbReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    init_sql: Option<String>,
    max_rows: Option<usize>,
    max_size_mb: Option<usize>,
    foreign_keys: Option<bool>,
    read_only: Option<bool>,
    query_timeout_ms: Option<u64>,
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateDbReq {
    group: Option<String>,
    description: Option<String>,
    max_rows: Option<usize>,
    max_size_mb: Option<usize>,
    foreign_keys: Option<bool>,
    read_only: Option<bool>,
    query_timeout_ms: Option<u64>,
    rate_limit_rpm: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DbQueryReq {
    sql: String,
    #[serde(default)]
    params: Vec<SqlValue>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DbExecuteReq {
    statements: Vec<DbStatementItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DbStatementItem {
    sql: String,
    #[serde(default)]
    params: Vec<SqlValue>,
}


pub(super) async fn db_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let items = svc.list_user_dbs(&caller).await.map_err(ApiError)?;
    Ok(Json(items))
}

pub(super) async fn db_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateDbReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let meta = svc
        .create_user_db_with_opts(
            &caller,
            &req.name,
            UserDbOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                init_sql: req.init_sql,
                max_rows: req.max_rows,
                max_size_mb: req.max_size_mb,
                foreign_keys: req.foreign_keys,
                read_only: req.read_only,
                query_timeout_ms: req.query_timeout_ms,
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(meta)))
}

pub(super) async fn db_update(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateDbReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_user_db(
            &caller,
            &id,
            UserDbOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                init_sql: None,
                max_rows: req.max_rows,
                max_size_mb: req.max_size_mb,
                foreign_keys: req.foreign_keys,
                read_only: req.read_only,
                query_timeout_ms: req.query_timeout_ms,
                rate_limit_rpm: req.rate_limit_rpm,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

pub(super) async fn db_delete(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_user_db(&caller, &id).await.map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn db_schema(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let schema = svc.schema_user_db(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "schema": schema })))
}

pub(super) async fn db_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<DbQueryReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_user_db(&caller, &id, &req.sql, req.params)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

pub(super) async fn db_execute(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<DbExecuteReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let stmts = req
        .statements
        .into_iter()
        .map(|s| (s.sql, s.params))
        .collect();
    let affected = svc
        .execute_user_db_batch(&caller, &id, stmts)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "rows_affected": affected })))
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateDbFtsReq {
    name: String,
    table: String,
    columns: Vec<String>,
    tokenizer: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DbFtsQueryReq {
    #[serde(default)]
    query: String,
}

pub(super) async fn db_fts_list(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let items = svc.list_user_db_fts(&caller, &id).await.map_err(ApiError)?;
    Ok(Json(items))
}

pub(super) async fn db_fts_create(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<CreateDbFtsReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let created = svc
        .create_user_db_fts(
            &caller,
            &id,
            &req.name,
            &req.table,
            req.columns,
            req.tokenizer.as_deref(),
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub(super) async fn db_fts_drop(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.drop_user_db_fts(&caller, &id, &name)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn db_fts_query(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, name)): Path<(String, String)>,
    Json(req): Json<DbFtsQueryReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_user_db_fts(&caller, &id, &name, &req.query)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

