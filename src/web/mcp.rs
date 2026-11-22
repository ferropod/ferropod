use std::collections::HashMap;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use serde_json::{json, Value};

use crate::core::model::{
    Bindings, Caller, EnqueueOpts, FerroError, JobStatus, KvNamespaceOpts, LogQuery, QueueConfig,
    Result as FerroResult, SearchIndexOpts, SearchQueryInput, SqlValue, TargetEnv, UserDbOpts,
};
use crate::core::service::{CreateDeploymentInput, PlatformService};
use crate::gateway::guard::{authenticate_caller, ApiError};
use crate::web::openapi;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn rpc_err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into(),
        }
    })
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "platform_overview",
            "description": "Summarize FerroPod platform status, apps, routes, queue depths (highlighting DLQ jobs), and recent error logs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "log_limit": { "type": "integer", "description": "Recent error log sample count (default 10)" }
                }
            }
        },
        {
            "name": "deploy_stack",
            "description": "Idempotently provision an app, its KV/DB/Bucket/Queue/Search bindings, optional inline static_files or Wasm/WAT worker, create a deployment, and optionally bind a hostname route.",
            "inputSchema": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "description": "App name" },
                    "target_env": { "type": "string", "enum": ["prod", "preview"], "description": "Target environment (default prod)" },
                    "assets_bucket": { "type": "string", "description": "Static assets bucket name" },
                    "assets_prefix": { "type": "string", "description": "Optional prefix inside bucket" },
                    "spa_fallback": { "type": "boolean", "description": "Enable SPA index.html fallback" },
                    "static_files": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Optional inline map of asset path -> UTF-8 content to upload into assets_bucket"
                    },
                    "worker_wat": { "type": "string", "description": "Optional inline WAT worker source" },
                    "worker_wasm_base64": { "type": "string", "description": "Optional base64-encoded .wasm/.cwasm worker artifact" },
                    "bindings": { "type": "object", "description": "Logical resource bindings { kv, db, queue, bucket, search, env }" },
                    "route_host": { "type": "string", "description": "Optional hostname to bind to target_env" }
                }
            }
        },
        {
            "name": "promote_or_rollback",
            "description": "Promote a preview or specific deployment to production, or rollback production to the previous release.",
            "inputSchema": {
                "type": "object",
                "required": ["action", "project"],
                "properties": {
                    "action": { "type": "string", "enum": ["promote", "rollback"] },
                    "project": { "type": "string", "description": "App name or ID" },
                    "deployment_id": { "type": "string", "description": "Optional explicit deployment ID" }
                }
            }
        },
        {
            "name": "invoke_worker",
            "description": "Send a synthetic HTTP request to a deployed Wasm Worker or static site.",
            "inputSchema": {
                "type": "object",
                "required": ["project"],
                "properties": {
                    "project": { "type": "string", "description": "App name or ID" },
                    "method": { "type": "string", "description": "HTTP method (default GET)" },
                    "path": { "type": "string", "description": "Request path and optional query string (default /)" },
                    "env": { "type": "string", "enum": ["prod", "preview"] },
                    "headers": { "type": "object", "additionalProperties": { "type": "string" } },
                    "body": { "type": "string", "description": "Optional UTF-8 request body" }
                }
            }
        },
        {
            "name": "inspect_schema",
            "description": "List accessible user databases when 'db' is omitted, or inspect SQL DDL schema of a specific database.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "db": { "type": "string", "description": "User database name or ID (omit to list all DBs)" }
                }
            }
        },
        {
            "name": "query_db",
            "description": "Run a read-only SQL query (mode='query') or DML/DDL statements (mode='execute') on a user Turso database.",
            "inputSchema": {
                "type": "object",
                "required": ["db", "sql"],
                "properties": {
                    "db": { "type": "string", "description": "User database name or ID" },
                    "mode": { "type": "string", "enum": ["query", "execute"], "description": "Default 'query'" },
                    "sql": { "type": "string", "description": "Parameterized SQL statement" },
                    "params": { "type": "array", "description": "Positional bind parameters" }
                }
            }
        },
        {
            "name": "diagnose_logs",
            "description": "Query recent FerroPod access, runtime, queue, cron, or audit logs for diagnostics.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project_id": { "type": "string" },
                    "request_id": { "type": "string" },
                    "source": { "type": "string" },
                    "level": { "type": "string" },
                    "q": { "type": "string" },
                    "limit": { "type": "integer", "description": "Max rows (default 50)" }
                }
            }
        },
        {
            "name": "manage_kv",
            "description": "Read, write, delete, or list keys in a FerroPod KV namespace.",
            "inputSchema": {
                "type": "object",
                "required": ["action", "namespace"],
                "properties": {
                    "action": { "type": "string", "enum": ["get", "put", "delete", "list"] },
                    "namespace": { "type": "string" },
                    "key": { "type": "string" },
                    "value": { "type": "string", "description": "UTF-8 value for put" },
                    "ttl_secs": { "type": "integer", "description": "Optional TTL seconds for put" },
                    "ephemeral": { "type": "boolean", "description": "Store in memory-only cache when true" },
                    "prefix": { "type": "string", "description": "Prefix filter for list" },
                    "limit": { "type": "integer" }
                }
            }
        },
        {
            "name": "manage_queue",
            "description": "Inspect queue depth stats, enqueue jobs, inspect pending/running/dead jobs, or replay DLQ jobs.",
            "inputSchema": {
                "type": "object",
                "required": ["action", "queue"],
                "properties": {
                    "action": { "type": "string", "enum": ["stats", "enqueue", "inspect", "replay_dlq"] },
                    "queue": { "type": "string" },
                    "payload": { "description": "JSON payload for enqueue" },
                    "status": { "type": "string", "enum": ["pending", "running", "dead"] },
                    "limit": { "type": "integer" }
                }
            }
        },
        {
            "name": "search_query",
            "description": "Query or upsert JSON documents in a FerroPod full-text / hybrid search index.",
            "inputSchema": {
                "type": "object",
                "required": ["action", "index"],
                "properties": {
                    "action": { "type": "string", "enum": ["query", "upsert"] },
                    "index": { "type": "string", "description": "Search index name or ID" },
                    "query": { "type": "string", "description": "Search keywords (for query)" },
                    "fuzzy": { "type": "boolean" },
                    "limit": { "type": "integer" },
                    "doc": { "type": "object", "description": "Document JSON object (for upsert)" }
                }
            }
        }
    ])
}

fn str_arg<'a>(args: &'a Value, key: &str) -> FerroResult<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FerroError::InvalidArgument(format!("missing required parameter '{key}'")))
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn parse_sql_params(args: &Value) -> Vec<SqlValue> {
    args.get("params")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|v| match v {
                    Value::Null => SqlValue::Null,
                    Value::Bool(b) => SqlValue::Bool(*b),
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            SqlValue::Integer(i)
                        } else {
                            SqlValue::Real(n.as_f64().unwrap_or(0.0))
                        }
                    }
                    Value::String(s) => SqlValue::Text(s.clone()),
                    other => SqlValue::Text(other.to_string()),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn map_conflict_ok<T>(res: FerroResult<T>) -> FerroResult<()> {
    match res {
        Ok(_) | Err(FerroError::Conflict(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

async fn ensure_stack_bindings(
    svc: &PlatformService,
    caller: &Caller,
    project_id: &str,
    bindings: &Bindings,
    assets_bucket: Option<&str>,
    has_static_files: bool,
) -> FerroResult<()> {
    for ns in bindings.kv.values() {
        map_conflict_ok(
            svc.create_kv_namespace_with_opts(
                caller,
                ns,
                KvNamespaceOpts {
                    project_id: Some(project_id.to_string()),
                    ..Default::default()
                },
            )
            .await,
        )?;
    }
    let existing_dbs = svc.list_user_dbs(caller).await?;
    for db_name in bindings.db.values() {
        if !existing_dbs
            .iter()
            .any(|d| d.name == *db_name || d.id == *db_name)
        {
            map_conflict_ok(
                svc.create_user_db_with_opts(
                    caller,
                    db_name,
                    UserDbOpts {
                        project_id: Some(project_id.to_string()),
                        ..Default::default()
                    },
                )
                .await,
            )?;
        }
    }
    for q_name in bindings.queue.values() {
        map_conflict_ok(
            svc.queue_create(
                caller,
                QueueConfig {
                    group: None,
                    name: q_name.clone(),
                    project_id: Some(project_id.to_string()),
                    consumer_project_id: None,
                    description: None,
                    max_concurrency: 4,
                    rate_limit_per_sec: None,
                    batch_size: 20,
                    lease_secs: 60,
                    max_attempts: 3,
                    retry_backoff_secs: 5,
                    dlq_retention_secs: None,
                    ephemeral: false,
                    max_pending_jobs: None,
                    created_at: Utc::now().timestamp(),
                },
            )
            .await,
        )?;
    }
    for bkt in bindings
        .bucket
        .values()
        .map(String::as_str)
        .chain(assets_bucket)
    {
        if svc.get_bucket_meta(bkt).await?.is_none() {
            map_conflict_ok(
                svc.create_bucket(caller, bkt, Some(project_id), has_static_files)
                    .await,
            )?;
        }
    }
    let existing_search = svc.list_search_indexes(caller).await?;
    for s_name in bindings.search.values() {
        if !existing_search
            .iter()
            .any(|s| s.name == *s_name || s.id == *s_name)
        {
            map_conflict_ok(
                svc.create_search_index_with_opts(
                    caller,
                    s_name,
                    SearchIndexOpts {
                        project_id: Some(project_id.to_string()),
                        ..Default::default()
                    },
                )
                .await,
            )?;
        }
    }
    Ok(())
}

async fn run_tool(
    svc: &PlatformService,
    caller: &Caller,
    name: &str,
    args: &Value,
) -> FerroResult<Value> {
    let result = match name {
        "platform_overview" => {
            let log_limit = args
                .get("log_limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(10)
                .clamp(1, 100) as usize;
            let (projects, routes, queues, err_logs) = tokio::try_join!(
                svc.list_projects(caller),
                svc.list_routes(caller),
                svc.queue_list(caller),
                svc.query_logs(
                    caller,
                    LogQuery {
                        level: Some("error".into()),
                        limit: Some(log_limit),
                        ..Default::default()
                    }
                )
            )?;
            let mut queue_summaries = Vec::with_capacity(queues.len());
            for q in queues {
                let st = svc.queue_stats(caller, &q.name).await?;
                queue_summaries.push(json!({
                    "name": q.name,
                    "pending": st.pending,
                    "running": st.running,
                    "dead": st.dead,
                    "ephemeral": q.ephemeral,
                }));
            }
            json!({
                "actor": caller.actor,
                "scope": caller.scope.as_str(),
                "profile": format!("{:?}", svc.config().profile),
                "apps": projects,
                "routes": routes,
                "queues": queue_summaries,
                "recent_error_count": err_logs.len(),
                "recent_errors": err_logs,
            })
        }
        "deploy_stack" => {
            let app_name = str_arg(args, "name")?;
            let proj = match svc.get_project(caller, app_name).await {
                Ok(p) => p,
                Err(FerroError::NotFound(_)) => svc.create_project(caller, app_name, None).await?,
                Err(e) => return Err(e),
            };
            let bindings: Bindings = args
                .get("bindings")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|e| FerroError::InvalidArgument(format!("invalid bindings: {e}")))?
                .unwrap_or_default();
            let assets_bucket = opt_str(args, "assets_bucket")
                .map(str::to_owned)
                .or_else(|| {
                    args.get("static_files")
                        .and_then(|v| v.as_object())
                        .filter(|m| !m.is_empty())
                        .map(|_| format!("{}-assets", proj.name))
                });
            let has_static = args
                .get("static_files")
                .and_then(|v| v.as_object())
                .is_some_and(|m| !m.is_empty());

            ensure_stack_bindings(
                svc,
                caller,
                &proj.id,
                &bindings,
                assets_bucket.as_deref(),
                has_static,
            )
            .await?;

            let mut uploaded_files = 0usize;
            if let (Some(bucket), Some(files_obj)) = (
                assets_bucket.as_deref(),
                args.get("static_files").and_then(|v| v.as_object()),
            ) {
                let auth = crate::core::model::BlobAuth::Caller(caller);
                for (path, content_val) in files_obj {
                    let content = content_val.as_str().ok_or_else(|| {
                        FerroError::InvalidArgument(format!(
                            "static file '{path}' must be UTF-8 string"
                        ))
                    })?;
                    let clean_key = path.trim_start_matches('/');
                    let full_key = match opt_str(args, "assets_prefix").map(|p| p.trim_matches('/'))
                    {
                        Some(pfx) if !pfx.is_empty() => format!("{pfx}/{clean_key}"),
                        _ => clean_key.to_string(),
                    };
                    let ct = mime_guess::from_path(&full_key)
                        .first_or_octet_stream()
                        .to_string();
                    let data = Bytes::from(content.as_bytes().to_vec());
                    let stream =
                        futures_util::stream::once(async move { Ok::<_, std::io::Error>(data) });
                    svc.put_object_stream(&auth, bucket, &full_key, Box::pin(stream), &ct, None)
                        .await?;
                    uploaded_files += 1;
                }
            }

            let artifact_path = if let Some(wat) = opt_str(args, "worker_wat") {
                let (stored, _, _) = svc
                    .upload_project_artifact(
                        caller,
                        &proj.id,
                        "worker.wat",
                        Bytes::from(wat.as_bytes().to_vec()),
                    )
                    .await?;
                Some(stored)
            } else if let Some(b64) = opt_str(args, "worker_wasm_base64") {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .map_err(|e| {
                        FerroError::InvalidArgument(format!("invalid base64 worker: {e}"))
                    })?;
                let (stored, _, _) = svc
                    .upload_project_artifact(caller, &proj.id, "worker.wasm", Bytes::from(decoded))
                    .await?;
                Some(stored)
            } else {
                None
            };

            let target_env = match opt_str(args, "target_env") {
                Some("preview") => Some(TargetEnv::Preview),
                _ => Some(TargetEnv::Prod),
            };
            let spa_fallback = args
                .get("spa_fallback")
                .and_then(|v| v.as_bool())
                .unwrap_or(has_static);
            let dep = svc
                .create_deployment(
                    caller,
                    &proj.id,
                    CreateDeploymentInput {
                        artifact_path,
                        artifact_bytes: None,
                        assets_bucket: assets_bucket.clone(),
                        assets_prefix: opt_str(args, "assets_prefix").map(str::to_owned),
                        spa_fallback,
                        bindings,
                        crons: Vec::new(),
                        target_env,
                    },
                )
                .await?;

            let route = if let Some(host) = opt_str(args, "route_host") {
                Some(
                    svc.upsert_route(
                        caller,
                        host,
                        &proj.id,
                        target_env.unwrap_or(TargetEnv::Prod),
                    )
                    .await?,
                )
            } else {
                None
            };

            json!({
                "project": proj,
                "deployment": dep,
                "uploaded_static_files": uploaded_files,
                "route": route,
            })
        }
        "promote_or_rollback" => {
            let action = str_arg(args, "action")?;
            let project = str_arg(args, "project")?;
            let dep_id = opt_str(args, "deployment_id");
            let dep = match action {
                "promote" => svc.promote_deployment(caller, project, dep_id).await?,
                "rollback" => svc.rollback_deployment(caller, project, dep_id).await?,
                other => {
                    return Err(FerroError::InvalidArgument(format!(
                        "unsupported action '{other}', expected 'promote' or 'rollback'"
                    )))
                }
            };
            json!({ "action": action, "deployment": dep })
        }
        "invoke_worker" => {
            let project = str_arg(args, "project")?;
            let method = opt_str(args, "method").unwrap_or("GET");
            let path = opt_str(args, "path").unwrap_or("/");
            let target_env = match opt_str(args, "env") {
                Some("preview") => Some(TargetEnv::Preview),
                _ => Some(TargetEnv::Prod),
            };
            let hdrs: Vec<(String, Vec<u8>)> = args
                .get("headers")
                .and_then(|v| v.as_object())
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.as_bytes().to_vec())))
                        .collect()
                })
                .unwrap_or_default();
            let body = Bytes::from(opt_str(args, "body").unwrap_or("").as_bytes().to_vec());
            let (dep_id, resp) = svc
                .invoke_project_http(caller, project, target_env, method, path, hdrs, body)
                .await?;
            use futures_util::TryStreamExt;
            let raw = resp
                .body
                .try_fold(Vec::new(), |mut acc, chunk| async move {
                    acc.extend_from_slice(&chunk);
                    Ok(acc)
                })
                .await
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("read worker response: {e}")))?;
            let resp_headers: HashMap<String, String> = resp
                .headers
                .into_iter()
                .map(|(k, v)| (k, String::from_utf8_lossy(&v).into_owned()))
                .collect();
            json!({
                "deployment_id": dep_id,
                "status": resp.status,
                "headers": resp_headers,
                "body": String::from_utf8_lossy(&raw),
            })
        }
        "inspect_schema" => {
            if let Some(db) = opt_str(args, "db") {
                let schema = svc.schema_user_db(caller, db).await?;
                json!({ "db": db, "schema": schema })
            } else {
                let dbs = svc.list_user_dbs(caller).await?;
                json!({ "databases": dbs })
            }
        }
        "query_db" => {
            let db = str_arg(args, "db")?;
            let sql = str_arg(args, "sql")?;
            let mode = opt_str(args, "mode").unwrap_or("query");
            let params = parse_sql_params(args);
            match mode {
                "query" => {
                    let qr = svc.query_user_db(caller, db, sql, params).await?;
                    serde_json::to_value(qr)
                        .map_err(|e| FerroError::Internal(anyhow::anyhow!("{e}")))?
                }
                "execute" => {
                    let affected = svc
                        .execute_user_db_batch(caller, db, vec![(sql.to_string(), params)])
                        .await?;
                    json!({ "rows_affected": affected })
                }
                other => {
                    return Err(FerroError::InvalidArgument(format!(
                        "unsupported mode '{other}', expected 'query' or 'execute'"
                    )))
                }
            }
        }
        "diagnose_logs" => {
            let q = LogQuery {
                project_id: opt_str(args, "project_id").map(str::to_owned),
                request_id: opt_str(args, "request_id").map(str::to_owned),
                source: opt_str(args, "source").map(str::to_owned),
                level: opt_str(args, "level").map(str::to_owned),
                since_ts: None,
                limit: Some(
                    args.get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(50)
                        .clamp(1, 200) as usize,
                ),
                q: opt_str(args, "q").map(str::to_owned),
            };
            let entries = svc.query_logs(caller, q).await?;
            json!({ "count": entries.len(), "entries": entries })
        }
        "manage_kv" => {
            let action = str_arg(args, "action")?;
            let ns = str_arg(args, "namespace")?;
            match action {
                "get" => {
                    let key = str_arg(args, "key")?;
                    match svc.kv_get(caller, ns, key).await? {
                        Some(v) => json!({
                            "found": true,
                            "value": String::from_utf8_lossy(&v.value),
                            "expires_at": v.expires_at,
                            "updated_at": v.updated_at,
                        }),
                        None => json!({ "found": false }),
                    }
                }
                "put" => {
                    let key = str_arg(args, "key")?;
                    let value = str_arg(args, "value")?;
                    let ttl_secs = args.get("ttl_secs").and_then(|v| v.as_u64());
                    let ephemeral = args
                        .get("ephemeral")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    svc.kv_put(
                        caller,
                        ns,
                        key,
                        Bytes::from(value.as_bytes().to_vec()),
                        None,
                        ttl_secs,
                        ephemeral,
                    )
                    .await?;
                    json!({ "ok": true, "namespace": ns, "key": key })
                }
                "delete" => {
                    let key = str_arg(args, "key")?;
                    svc.kv_delete(caller, ns, key).await?;
                    json!({ "ok": true, "namespace": ns, "key": key })
                }
                "list" => {
                    let prefix = opt_str(args, "prefix");
                    let limit = args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(100)
                        .clamp(1, 500) as usize;
                    let keys = svc.kv_list(caller, ns, prefix, limit).await?;
                    json!({ "count": keys.len(), "keys": keys })
                }
                other => {
                    return Err(FerroError::InvalidArgument(format!(
                        "unsupported kv action '{other}'"
                    )))
                }
            }
        }
        "manage_queue" => {
            let action = str_arg(args, "action")?;
            let queue = str_arg(args, "queue")?;
            match action {
                "stats" => {
                    let st = svc.queue_stats(caller, queue).await?;
                    serde_json::to_value(st)
                        .map_err(|e| FerroError::Internal(anyhow::anyhow!("{e}")))?
                }
                "enqueue" => {
                    let payload = args.get("payload").cloned().unwrap_or(json!({}));
                    let bytes = Bytes::from(serde_json::to_vec(&payload).map_err(|e| {
                        FerroError::InvalidArgument(format!("invalid payload: {e}"))
                    })?);
                    let job_id = svc
                        .queue_enqueue(caller, queue, bytes, EnqueueOpts::default())
                        .await?;
                    json!({ "job_id": job_id, "queue": queue })
                }
                "inspect" => {
                    let status = JobStatus::parse(opt_str(args, "status").unwrap_or("dead"))?;
                    let limit = args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(20)
                        .clamp(1, 100) as usize;
                    let jobs = svc.queue_inspect(caller, queue, status, limit).await?;
                    json!({ "count": jobs.len(), "jobs": jobs })
                }
                "replay_dlq" => {
                    let replayed = svc.queue_dlq_replay(caller, queue, None).await?;
                    json!({ "replayed": replayed, "queue": queue })
                }
                other => {
                    return Err(FerroError::InvalidArgument(format!(
                        "unsupported queue action '{other}'"
                    )))
                }
            }
        }
        "search_query" => {
            let action = str_arg(args, "action")?;
            let index = str_arg(args, "index")?;
            match action {
                "query" => {
                    let query = opt_str(args, "query").unwrap_or("").to_string();
                    let fuzzy = args.get("fuzzy").and_then(|v| v.as_bool()).unwrap_or(true);
                    let limit = args
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(10)
                        .clamp(1, 50) as usize;
                    let res = svc
                        .query_search_index(
                            caller,
                            index,
                            SearchQueryInput {
                                query,
                                fuzzy,
                                sort: None,
                                limit: Some(limit),
                                facets: HashMap::new(),
                            },
                        )
                        .await?;
                    serde_json::to_value(res)
                        .map_err(|e| FerroError::Internal(anyhow::anyhow!("{e}")))?
                }
                "upsert" => {
                    let doc = args.get("doc").cloned().ok_or_else(|| {
                        FerroError::InvalidArgument("missing required parameter 'doc'".into())
                    })?;
                    let saved = svc.upsert_search_doc(caller, index, doc).await?;
                    json!({ "saved": saved })
                }
                other => {
                    return Err(FerroError::InvalidArgument(format!(
                        "unsupported search action '{other}'"
                    )))
                }
            }
        }
        unknown => {
            return Err(FerroError::NotFound(format!(
                "unknown MCP tool '{unknown}'"
            )));
        }
    };

    svc.audit(
        None,
        &caller.actor,
        &format!("mcp.{name}"),
        json!({ "scope": caller.scope.as_str() }),
    );
    Ok(result)
}

/// Pure-Rust JSON-RPC 2.0 MCP handler shared between `/_mcp` HTTP and `ferropod mcp` stdio.
pub async fn handle_rpc(svc: &PlatformService, caller: &Caller, body: Value) -> Option<Value> {
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").cloned();

    // JSON-RPC notifications omit "id" (e.g. notifications/initialized)
    if id.is_none() || method.starts_with("notifications/") {
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    let resp = match method {
        "initialize" => rpc_ok(
            id,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": "ferropod",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        ),
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, json!({ "tools": tool_definitions() })),
        "resources/list" => rpc_ok(
            id,
            json!({
                "resources": [
                    {
                        "uri": "ferropod://overview",
                        "name": "Platform Overview",
                        "description": "Live cluster status, apps, routes, and queues",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "ferropod://llms.txt",
                        "name": "Agent Handbook",
                        "description": "FerroPod API, CLI, and MCP guide",
                        "mimeType": "text/plain"
                    }
                ]
            }),
        ),
        "resources/read" => {
            let uri = body
                .get("params")
                .and_then(|p| p.get("uri"))
                .and_then(|u| u.as_str())
                .unwrap_or("");
            match uri {
                "ferropod://llms.txt" => rpc_ok(
                    id,
                    json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": openapi::llms_txt()
                        }]
                    }),
                ),
                "ferropod://overview" => {
                    match run_tool(svc, caller, "platform_overview", &json!({})).await {
                        Ok(val) => rpc_ok(
                            id,
                            json!({
                                "contents": [{
                                    "uri": uri,
                                    "mimeType": "application/json",
                                    "text": serde_json::to_string_pretty(&val).unwrap_or_default()
                                }]
                            }),
                        ),
                        Err(e) => rpc_err(id, -32603, e.to_string()),
                    }
                }
                other => rpc_err(id, -32602, format!("unknown resource uri '{other}'")),
            }
        }
        "tools/call" => {
            let Some(params) = body.get("params") else {
                return Some(rpc_err(id, -32602, "missing params"));
            };
            let Some(tool_name) = params.get("name").and_then(|v| v.as_str()) else {
                return Some(rpc_err(id, -32602, "missing tool name"));
            };
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match run_tool(svc, caller, tool_name, &args).await {
                Ok(val) => {
                    let text = serde_json::to_string_pretty(&val).unwrap_or_else(|_| "{}".into());
                    rpc_ok(
                        id,
                        json!({
                            "content": [{ "type": "text", "text": text }],
                            "isError": false
                        }),
                    )
                }
                Err(e) => rpc_ok(
                    id,
                    json!({
                        "content": [{ "type": "text", "text": e.to_string() }],
                        "isError": true
                    }),
                ),
            }
        }
        other => rpc_err(id, -32601, format!("method not found: {other}")),
    };

    Some(resp)
}

pub async fn mcp_http_handler(
    State(svc): State<PlatformService>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> std::result::Result<Response, ApiError> {
    let caller = authenticate_caller(&svc, &headers, None).await?;
    match handle_rpc(&svc, &caller, body).await {
        Some(resp) => Ok((StatusCode::OK, Json(resp)).into_response()),
        None => Ok((StatusCode::ACCEPTED, Body::empty()).into_response()),
    }
}
