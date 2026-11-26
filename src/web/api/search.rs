use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::core::model::{SearchFieldSchema, SearchIndexOpts, SearchQueryInput};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, ApiError};

#[derive(Debug, Deserialize)]
pub(super) struct CreateSearchIndexReq {
    group: Option<String>,
    name: String,
    project_id: Option<String>,
    description: Option<String>,
    tokenizer: Option<String>,
    writer_budget_mb: Option<usize>,
    max_docs: Option<usize>,
    rate_limit_rpm: Option<u32>,
    schema: Option<Vec<SearchFieldSchema>>,
    facet_fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateSearchIndexReq {
    group: Option<String>,
    description: Option<String>,
    writer_budget_mb: Option<usize>,
    max_docs: Option<usize>,
    rate_limit_rpm: Option<u32>,
}

pub(super) async fn search_list_indexes(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    Ok(Json(
        svc.list_search_indexes(&caller).await.map_err(ApiError)?,
    ))
}

pub(super) async fn search_create_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(req): Json<CreateSearchIndexReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let created = svc
        .create_search_index_with_opts(
            &caller,
            &req.name,
            SearchIndexOpts {
                group: req.group,
                project_id: req.project_id,
                description: req.description,
                tokenizer: req.tokenizer,
                writer_budget_mb: req.writer_budget_mb,
                max_docs: req.max_docs,
                rate_limit_rpm: req.rate_limit_rpm,
                schema: req.schema,
                facet_fields: req.facet_fields,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub(super) async fn search_update_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<UpdateSearchIndexReq>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .update_search_index(
            &caller,
            &id,
            SearchIndexOpts {
                group: req.group,
                project_id: None,
                description: req.description,
                tokenizer: None,
                writer_budget_mb: req.writer_budget_mb,
                max_docs: req.max_docs,
                rate_limit_rpm: req.rate_limit_rpm,
                schema: None,
                facet_fields: None,
            },
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

pub(super) async fn search_delete_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    svc.delete_search_index(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn search_commit_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let updated = svc
        .commit_search_index(&caller, &id)
        .await
        .map_err(ApiError)?;
    Ok(Json(updated))
}

#[derive(Debug, Deserialize)]
pub(super) struct SearchDocsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
}

pub(super) async fn search_list_docs(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(query): Query<SearchDocsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let docs = svc
        .list_search_docs_page(
            &caller,
            &id,
            query.limit.unwrap_or(100),
            query.offset.unwrap_or(0),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(docs))
}

pub(super) async fn search_upsert_doc(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(doc): Json<serde_json::Value>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let saved = svc
        .upsert_search_doc(&caller, &id, doc)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(saved)))
}

pub(super) async fn search_delete_doc(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path((id, doc_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let deleted = svc
        .delete_search_doc(&caller, &id, &doc_id)
        .await
        .map_err(ApiError)?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

pub(super) async fn search_query_index(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<SearchQueryInput>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    let res = svc
        .query_search_index(&caller, &id, req)
        .await
        .map_err(ApiError)?;
    Ok(Json(res))
}

