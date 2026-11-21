use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
    Json, Router,
};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};

use crate::core::model::{BlobAuth, ByteRange, ByteStream, Caller, FerroError};
use crate::core::service::PlatformService;
use crate::gateway::guard::{authenticate_caller, extract_bearer_token, ApiError};

const BLOB_CSP_HEADER: &str =
    "sandbox; default-src 'none'; img-src 'self' data:; media-src 'self'; style-src 'unsafe-inline'";

pub(crate) fn etag_matches(if_none_match: &str, current_etag: &str) -> bool {
    let target = current_etag.trim().trim_start_matches("W/");
    if_none_match.split(',').any(|candidate| {
        let tok = candidate.trim();
        tok == "*" || tok.trim_start_matches("W/") == target
    })
}

#[derive(Debug, Deserialize)]
pub struct BlobQuery {
    pub op: Option<String>,
    pub exp: Option<i64>,
    pub sig: Option<String>,
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub prefix: Option<String>,
    pub limit: Option<usize>,
    pub token: Option<String>,
}

#[derive(Debug, Serialize)]
struct DeleteResp {
    deleted: bool,
}

enum OwnedBlobAuth {
    Caller(Caller),
    Presigned { op: String, exp: i64, sig: String },
    Anonymous,
}

impl OwnedBlobAuth {
    fn as_ref(&self) -> BlobAuth<'_> {
        match self {
            Self::Caller(c) => BlobAuth::Caller(c),
            Self::Presigned { op, exp, sig } => BlobAuth::Presigned { op, exp: *exp, sig },
            Self::Anonymous => BlobAuth::Anonymous,
        }
    }
}

async fn resolve_blob_auth(
    svc: &PlatformService,
    headers: &HeaderMap,
    q: &BlobQuery,
) -> std::result::Result<OwnedBlobAuth, ApiError> {
    if let (Some(op), Some(exp), Some(sig)) = (&q.op, q.exp, &q.sig) {
        return Ok(OwnedBlobAuth::Presigned {
            op: op.clone(),
            exp,
            sig: sig.clone(),
        });
    }
    if extract_bearer_token(headers, q.token.as_deref()).is_some() {
        let caller = authenticate_caller(svc, headers, q.token.as_deref()).await?;
        return Ok(OwnedBlobAuth::Caller(caller));
    }
    Ok(OwnedBlobAuth::Anonymous)
}

pub fn router() -> Router<PlatformService> {
    Router::new().route("/{bucket}", get(list_objects)).route(
        "/{bucket}/{*key}",
        put(put_object)
            .get(get_object)
            .head(head_object)
            .delete(delete_object),
    )
}

async fn put_object(
    State(svc): State<PlatformService>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
    body: Body,
) -> std::result::Result<impl IntoResponse, ApiError> {
    let auth = resolve_blob_auth(&svc, &headers, &q).await?;

    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| {
            mime_guess::from_path(&key)
                .first_or_octet_stream()
                .essence_str()
                .to_string()
        });

    let stream: ByteStream = Box::pin(
        body.into_data_stream()
            .map_err(|e| std::io::Error::other(e.to_string())),
    );
    let meta = svc
        .put_object_stream(&auth.as_ref(), &bucket, &key, stream, &content_type, None)
        .await
        .map_err(ApiError)?;
    Ok((StatusCode::CREATED, Json(meta)))
}

async fn get_object(
    State(svc): State<PlatformService>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    let auth = resolve_blob_auth(&svc, &headers, &q).await?;
    let auth_ref = auth.as_ref();

    let head_meta = svc
        .head_object(&auth_ref, &bucket, &key)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(FerroError::NotFound(format!("object '{bucket}/{key}'"))))?;

    let etag = format!("\"{}\"", head_meta.sha256);
    if let Some(inm) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if etag_matches(inm, &etag) {
            return Ok(Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, etag)
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .header(header::CONTENT_SECURITY_POLICY, BLOB_CSP_HEADER)
                .body(Body::empty())
                .unwrap());
        }
    }

    let range = match headers.get(header::RANGE).and_then(|v| v.to_str().ok()) {
        Some(raw) => match parse_byte_range(raw, head_meta.size) {
            Some(r) => Some(r),
            None => {
                return Ok(Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{}", head_meta.size))
                    .body(Body::empty())
                    .unwrap());
            }
        },
        None => None,
    };

    let (meta, stream, len, span) = svc
        .get_object_stream(&auth_ref, &bucket, &key, range)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(FerroError::NotFound(format!("object '{bucket}/{key}'"))))?;

    let mut builder = Response::builder()
        .status(if span.is_some() {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, meta.content_type)
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::ETAG, etag)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, BLOB_CSP_HEADER);

    if let Some((start, end)) = span {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{}", meta.size),
        );
    }

    if let Ok(Some(b_meta)) = svc.get_bucket_meta(&bucket).await {
        if let Some(cc) = b_meta.cache_control.as_deref() {
            builder = builder.header(header::CACHE_CONTROL, cc);
        }
        if let Some(cors) = b_meta.cors_origins.as_deref() {
            builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, cors);
        }
    }

    Ok(builder.body(Body::from_stream(stream)).unwrap())
}

async fn head_object(
    State(svc): State<PlatformService>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
) -> std::result::Result<Response, ApiError> {
    let auth = resolve_blob_auth(&svc, &headers, &q).await?;
    let meta = svc
        .head_object(&auth.as_ref(), &bucket, &key)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(FerroError::NotFound(format!("object '{bucket}/{key}'"))))?;

    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, meta.content_type)
        .header(header::CONTENT_LENGTH, meta.size.to_string())
        .header(header::ETAG, format!("\"{}\"", meta.sha256))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_SECURITY_POLICY, BLOB_CSP_HEADER);

    if let Ok(Some(b_meta)) = svc.get_bucket_meta(&bucket).await {
        if let Some(cc) = b_meta.cache_control.as_deref() {
            builder = builder.header(header::CACHE_CONTROL, cc);
        }
        if let Some(cors) = b_meta.cors_origins.as_deref() {
            builder = builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, cors);
        }
    }

    Ok(builder.body(Body::empty()).unwrap())
}

async fn delete_object(
    State(svc): State<PlatformService>,
    Path((bucket, key)): Path<(String, String)>,
    Query(q): Query<BlobQuery>,
    headers: HeaderMap,
) -> std::result::Result<impl IntoResponse, ApiError> {
    let auth = resolve_blob_auth(&svc, &headers, &q).await?;
    let deleted = svc
        .delete_object(&auth.as_ref(), &bucket, &key)
        .await
        .map_err(ApiError)?;
    if !deleted {
        return Err(ApiError(FerroError::NotFound(format!(
            "object '{bucket}/{key}'"
        ))));
    }
    Ok(Json(DeleteResp { deleted }))
}

async fn list_objects(
    State(svc): State<PlatformService>,
    Path(bucket): Path<String>,
    Query(q): Query<ListQuery>,
    headers: HeaderMap,
) -> std::result::Result<impl IntoResponse, ApiError> {
    let auth = if extract_bearer_token(&headers, q.token.as_deref()).is_some() {
        OwnedBlobAuth::Caller(authenticate_caller(&svc, &headers, q.token.as_deref()).await?)
    } else {
        OwnedBlobAuth::Anonymous
    };
    let items = svc
        .list_objects(
            &auth.as_ref(),
            &bucket,
            q.prefix.as_deref(),
            q.limit.unwrap_or(200),
        )
        .await
        .map_err(ApiError)?;
    Ok(Json(items))
}

pub fn parse_byte_range(header_val: &str, total_size: u64) -> Option<ByteRange> {
    let spec = header_val.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || total_size == 0 {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    if start_s.is_empty() {
        let suffix = end_s.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        let start = total_size.saturating_sub(suffix);
        return Some(ByteRange {
            start,
            end: Some(total_size - 1),
        });
    }
    let start = start_s.parse::<u64>().ok()?;
    if start >= total_size {
        return None;
    }
    let end = if end_s.is_empty() {
        None
    } else {
        let e = end_s.parse::<u64>().ok()?;
        if e < start {
            return None;
        }
        Some(e.min(total_size - 1))
    };
    Some(ByteRange { start, end })
}
