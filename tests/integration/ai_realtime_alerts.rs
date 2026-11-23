use std::collections::HashMap;
use std::io::Read as _;
use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use flate2::read::GzDecoder;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tower::ServiceExt;

use ferropod::config::{Config, Profile};
use ferropod::core::crypto::Crypto;
use ferropod::core::model::{Caller, ProjectOpts, SearchIndexOpts, TokenScope};
use ferropod::core::service::{PlatformDeps, PlatformService};
use ferropod::deploy_cli::{deploy_manifest, DeployOptions, Manifest};
use ferropod::gateway::router::build_router;
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore,
    log_store::DailyLogStore, queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};
use ferropod::web::mcp;

async fn open_service(data_dir: PathBuf) -> PlatformService {
    let mut cfg = Config::for_profile(
        Profile::LowMem,
        "127.0.0.1:0".parse().unwrap(),
        data_dir,
        "integration-test-master-key-32bytes".to_string(),
    );
    cfg.blob_gc_grace_secs = 0;

    let core = Arc::new(TursoCoreStore::open(&cfg.core_db_path()).await.unwrap());
    let kv = Arc::new(
        TursoKvStore::open(&cfg.kv_db_path(), cfg.max_ephemeral_kv_entries)
            .await
            .unwrap(),
    );
    let queue = Arc::new(TursoQueueStore::open(&cfg.queue_db_path()).await.unwrap());
    let user_dbs =
        Arc::new(TursoUserDbManager::new(cfg.user_dbs_dir(), cfg.max_open_user_dbs).unwrap());
    let blobs =
        Arc::new(FsBlobStore::new(cfg.blobs_dir(), core.clone(), cfg.blob_gc_grace_secs).unwrap());
    let logs = Arc::new(
        DailyLogStore::open(cfg.logs_dir(), cfg.log_channel_capacity)
            .await
            .unwrap(),
    );

    PlatformService::new(
        cfg,
        PlatformDeps {
            projects: core.clone(),
            secrets: core.clone(),
            tokens: core.clone(),
            registry: core.clone(),
            kv,
            queue,
            objects: blobs,
            db: user_dbs,
            logs,
        },
    )
    .await
    .unwrap()
}

struct Harness {
    _tmp: TempDir,
    svc: PlatformService,
    admin: Caller,
    admin_token: String,
    app: Router,
}

async fn setup() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let svc = open_service(tmp.path().to_path_buf()).await;
    let admin = Caller::system("test");
    let (_meta, admin_token) = svc
        .create_token(&admin, "test-admin", TokenScope::Admin, None)
        .await
        .unwrap();
    let app = build_router(svc.clone());
    Harness {
        _tmp: tmp,
        svc,
        admin,
        admin_token,
        app,
    }
}

async fn json_req(
    app: &Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    if let Some(tok) = token {
        b = b.header(header::AUTHORIZATION, format!("Bearer {tok}"));
    }
    let req = if let Some(val) = body {
        b = b.header(header::CONTENT_TYPE, "application/json");
        b.body(Body::from(val.to_string())).unwrap()
    } else {
        b.body(Body::empty()).unwrap()
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let parsed = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, parsed)
}

#[tokio::test]
async fn ai_gateway_rbac_streaming_models_cache_and_usage_isolation() {
    let h = setup().await;

    // Mock OpenAI upstream server supporting both JSON and SSE streaming
    let mock_ai = Router::new().route(
        "/v1/chat/completions",
        post(|Json(req): Json<Value>| async move {
            let stream = req
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if stream {
                let sse = concat!(
                    "data: {\"id\":\"cmpl-1\",\"model\":\"gpt-mock\",\"choices\":[{\"delta\":{\"content\":\"Hello \"}}]}\n\n",
                    "data: {\"id\":\"cmpl-1\",\"model\":\"gpt-mock\",\"choices\":[{\"delta\":{\"content\":\"world\"}}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
                    "data: [DONE]\n\n"
                );
                axum::response::Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(sse))
                    .unwrap()
            } else {
                axum::response::Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": "cmpl-json",
                            "model": "gpt-mock",
                            "choices": [{"message": {"role": "assistant", "content": "cached-ok"}}],
                            "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
                        })
                        .to_string(),
                    ))
                    .unwrap()
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let ai_addr = listener.local_addr().unwrap();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, mock_ai).await.unwrap();
    });

    // 1. Create provider with custom timeout_secs and cache_ttl_secs
    let (st, created) = json_req(
        &h.app,
        Method::POST,
        "/_api/v1/ai/providers",
        Some(&h.admin_token),
        Some(json!({
            "name": "mock-openai",
            "base_url": format!("http://{ai_addr}/v1"),
            "api_key": "sk-test",
            "default_model": "gpt-mock",
            "cache_ttl_secs": 60,
            "timeout_secs": 45
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(created["timeout_secs"], 45);

    // Create readonly + two project tokens
    let (_rm, ro_token) = h
        .svc
        .create_token(&h.admin, "ro", TokenScope::Readonly, None)
        .await
        .unwrap();
    let p1 = h
        .svc
        .create_project_with_opts(&h.admin, "ai-app-1", ProjectOpts::default())
        .await
        .unwrap();
    let p2 = h
        .svc
        .create_project_with_opts(&h.admin, "ai-app-2", ProjectOpts::default())
        .await
        .unwrap();
    let (_m1, p1_token) = h
        .svc
        .create_token(&h.admin, "p1-tok", TokenScope::Project(p1.id.clone()), None)
        .await
        .unwrap();
    let (_m2, p2_token) = h
        .svc
        .create_token(&h.admin, "p2-tok", TokenScope::Project(p2.id.clone()), None)
        .await
        .unwrap();

    // 2. Readonly token is forbidden from invoking proxy or models
    let (st, _) = json_req(&h.app, Method::GET, "/_ai/v1/models", Some(&ro_token), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = json_req(
        &h.app,
        Method::POST,
        "/_ai/v1/chat/completions",
        Some(&ro_token),
        Some(json!({ "messages": [{ "role": "user", "content": "hi" }] })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 3. GET /_ai/v1/models succeeds for project token
    let (st, models) = json_req(&h.app, Method::GET, "/_ai/v1/models", Some(&p1_token), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(models["object"], "list");
    assert_eq!(models["data"][0]["id"], "gpt-mock");

    // 4. Non-streaming call + cache hit + clear cache
    let (st, body1) = json_req(
        &h.app,
        Method::POST,
        "/_ai/v1/chat/completions",
        Some(&p1_token),
        Some(json!({ "messages": [{ "role": "user", "content": "hi" }] })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body1["choices"][0]["message"]["content"], "cached-ok");

    let req2 = Request::builder()
        .method(Method::POST)
        .uri("/_ai/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {p1_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({ "messages": [{ "role": "user", "content": "hi" }] }).to_string(),
        ))
        .unwrap();
    let resp2 = h.app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.headers().get("x-ferropod-cache").unwrap(), "HIT");

    let (st, cleared) = json_req(
        &h.app,
        Method::POST,
        "/_api/v1/ai/cache/clear",
        Some(&h.admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(cleared["evicted"], 1);

    // 5. SSE Streaming passthrough as p2_token
    let req_stream = Request::builder()
        .method(Method::POST)
        .uri("/_ai/v1/chat/completions")
        .header(header::AUTHORIZATION, format!("Bearer {p2_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "messages": [{ "role": "user", "content": "stream" }],
                "stream": true
            })
            .to_string(),
        ))
        .unwrap();
    let resp_stream = h.app.clone().oneshot(req_stream).await.unwrap();
    assert_eq!(resp_stream.status(), StatusCode::OK);
    assert!(resp_stream
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/event-stream"));
    let stream_text = String::from_utf8(
        resp_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(stream_text.contains("Hello "));
    assert!(stream_text.contains("[DONE]"));

    // Allow spawned usage recorder to flush
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;

    // 6. Verify per-project usage isolation and real SQL aggregation (including month_tokens)
    let (st, u1) = json_req(
        &h.app,
        Method::GET,
        "/_api/v1/ai/usage",
        Some(&p1_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(u1["total_requests"], 2);
    assert_eq!(u1["cached_requests"], 1);
    assert_eq!(u1["total_tokens"], 14);
    assert_eq!(u1["month_tokens"], 14);

    let (st, u2) = json_req(
        &h.app,
        Method::GET,
        "/_api/v1/ai/usage",
        Some(&p2_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(u2["total_requests"], 1);
    assert_eq!(u2["total_tokens"], 7);
    assert_eq!(u2["month_tokens"], 7);

    server_handle.abort();
}

#[tokio::test]
async fn alert_webhooks_crud_hmac_and_fanout() {
    let h = setup().await;
    let (tx, mut rx) = mpsc::channel::<(String, Value)>(8);
    let tx_clone = tx.clone();

    let receiver_app = Router::new().route(
        "/hook",
        post(move |headers: HeaderMap, body: Bytes| {
            let tx = tx_clone.clone();
            async move {
                let sig = headers
                    .get("x-ferropod-signature")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                let expected = format!("sha256={}", Crypto::hmac_sha256_hex(b"whsec_123", &body));
                assert_eq!(sig, expected, "HMAC signature mismatch");
                let parsed: Value = serde_json::from_slice(&body).unwrap();
                let _ = tx.send((sig, parsed)).await;
                StatusCode::OK
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        axum::serve(listener, receiver_app).await.unwrap();
    });

    // Create webhook
    let (st, created) = json_req(
        &h.app,
        Method::POST,
        "/_api/v1/alerts/webhooks",
        Some(&h.admin_token),
        Some(json!({
            "name": "primary-hook",
            "url": format!("http://{hook_addr}/hook"),
            "secret": "whsec_123",
            "events": ["auto_rollback", "test"]
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["has_secret"], true);

    // Explicit test endpoint
    let (st, test_res) = json_req(
        &h.app,
        Method::POST,
        &format!("/_api/v1/alerts/webhooks/{id}/test"),
        Some(&h.admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(test_res["ok"], true);
    let (_, ev1) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev1["event"], "test");

    // Platform fire_alert fanout
    h.svc.fire_alert(
        "auto_rollback",
        "Deployment rolled back".into(),
        json!({ "project": "demo" }),
    );
    let (_, ev2) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ev2["event"], "auto_rollback");

    // Delete webhook
    let (st, _) = json_req(
        &h.app,
        Method::DELETE,
        &format!("/_api/v1/alerts/webhooks/{id}"),
        Some(&h.admin_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    srv.abort();
}

#[tokio::test]
async fn realtime_pubsub_sse_and_scope_isolation() {
    let h = setup().await;
    let p1 = h
        .svc
        .create_project_with_opts(&h.admin, "rt-app", ProjectOpts::default())
        .await
        .unwrap();
    let (_m1, p1_token) = h
        .svc
        .create_token(&h.admin, "rt-p1", TokenScope::Project(p1.id.clone()), None)
        .await
        .unwrap();
    let (_mr, ro_token) = h
        .svc
        .create_token(&h.admin, "rt-ro", TokenScope::Readonly, None)
        .await
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = h.app.clone();
    let srv = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let allowed_ch = format!("{}:updates", p1.id);
    let client = reqwest::Client::new();
    let mut sse_resp = client
        .get(format!("http://{addr}/_realtime/{allowed_ch}/sse"))
        .bearer_auth(&p1_token)
        .send()
        .await
        .unwrap();
    assert_eq!(sse_resp.status(), StatusCode::OK);

    // Readonly cannot publish, project token cannot publish to other project channel
    let (st, _) = json_req(
        &h.app,
        Method::POST,
        &format!("/_realtime/{allowed_ch}/publish"),
        Some(&ro_token),
        Some(json!({ "event": "ping", "data": { "n": 1 } })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &h.app,
        Method::POST,
        "/_realtime/otherproj:updates/publish",
        Some(&p1_token),
        Some(json!({ "event": "ping", "data": { "n": 1 } })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Publish valid event
    let (st, pub_msg) = json_req(
        &h.app,
        Method::POST,
        &format!("/_realtime/{allowed_ch}/publish"),
        Some(&p1_token),
        Some(json!({ "event": "sync", "data": { "ok": true } })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    assert_eq!(pub_msg["event"], "sync");

    let chunk = tokio::time::timeout(std::time::Duration::from_secs(2), sse_resp.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&chunk);
    assert!(text.contains("event: sync"), "unexpected sse body: {text}");
    assert!(text.contains("\"ok\":true"), "unexpected sse body: {text}");

    let (st, channels) = json_req(
        &h.app,
        Method::GET,
        "/_api/v1/realtime/channels",
        Some(&p1_token),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(channels[0]["channel"], allowed_ch);

    srv.abort();
}

#[tokio::test]
async fn backups_download_schedule_and_restore() {
    let h = setup().await;

    let (st, sched) = json_req(
        &h.app,
        Method::PATCH,
        "/_api/v1/backups/schedule",
        Some(&h.admin_token),
        Some(json!({ "enabled": true, "interval_hours": 12, "max_backups": 3 })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(sched["enabled"], true);
    assert_eq!(sched["interval_hours"], 12);

    let (st, bk) = json_req(
        &h.app,
        Method::POST,
        "/_api/v1/backups",
        Some(&h.admin_token),
        Some(json!({ "label": "dltest" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let bk_name = bk["name"].as_str().unwrap().to_string();

    let dl_req = Request::builder()
        .method(Method::GET)
        .uri(format!("/_api/v1/backups/{bk_name}/download"))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::empty())
        .unwrap();
    let dl_resp = h.app.clone().oneshot(dl_req).await.unwrap();
    assert_eq!(dl_resp.status(), StatusCode::OK);
    let gz_bytes = dl_resp.into_body().collect().await.unwrap().to_bytes();

    let mut archive = tar::Archive::new(GzDecoder::new(&gz_bytes[..]));
    let mut found_core = false;
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        if path.ends_with("core.db") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            assert!(!buf.is_empty());
            found_core = true;
        }
    }
    assert!(found_core, "core.db missing from downloaded tar.gz");

    let (st, restored) = json_req(
        &h.app,
        Method::POST,
        &format!("/_api/v1/backups/{bk_name}/restore"),
        Some(&h.admin_token),
        Some(json!({ "restart": false })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(restored["restarting"], false);
    assert!(restored["restored_files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "core.db"));
}

#[tokio::test]
async fn mcp_manage_ai_and_hybrid_search_and_manifest_extensions() {
    let h = setup().await;

    // MCP tools/list includes 11 tools including manage_ai
    let list_resp = mcp::handle_rpc(
        &h.svc,
        &h.admin,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
    )
    .await
    .unwrap();
    let tools = list_resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 11);
    assert!(tools.iter().any(|t| t["name"] == "manage_ai"));

    // Hybrid search via MCP search_query with vector + alpha
    h.svc
        .create_search_index_with_opts(&h.admin, "vec-idx", SearchIndexOpts::default())
        .await
        .unwrap();
    let _ = mcp::handle_rpc(
        &h.svc,
        &h.admin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "search_query",
                "arguments": {
                    "action": "upsert",
                    "index": "vec-idx",
                    "doc": { "id": "doc1", "title": "rust wasm", "embedding": [1.0, 0.0] }
                }
            }
        }),
    )
    .await
    .unwrap();
    let search_res = mcp::handle_rpc(
        &h.svc,
        &h.admin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "search_query",
                "arguments": {
                    "action": "query",
                    "index": "vec-idx",
                    "query": "",
                    "vector": [1.0, 0.0],
                    "alpha": 1.0,
                    "limit": 5
                }
            }
        }),
    )
    .await
    .unwrap();
    assert_eq!(search_res["result"]["isError"], false);
    assert!(search_res["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("doc1"));

    // Manifest with secrets, extended routes, and ai_providers
    let tmp_dir = tempfile::tempdir().unwrap();
    let toml_path = tmp_dir.path().join("ferropod.toml");
    let toml_body = r#"
[app]
name = "full-manifest-app"

[secrets]
JWT_SECRET = "local-dev-secret"

[[routes]]
host = "app.example.com"
env = "prod"
proxy_prefix = "/api"
proxy_upstream = "127.0.0.1:4000"
rate_limit_rpm = 120
basic_auth_user = "preview"
canary_weight = 10

[[ai_providers]]
name = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4o-mini"
timeout_secs = 90
"#;
    std::fs::write(&toml_path, toml_body).unwrap();
    let parsed: Manifest = toml::from_str(toml_body).unwrap();
    assert_eq!(
        parsed.secrets.get("JWT_SECRET").unwrap(),
        "local-dev-secret"
    );
    assert_eq!(parsed.routes[0].rate_limit_rpm, Some(120));
    assert_eq!(parsed.ai_providers[0].timeout_secs, Some(90));

    let dry = deploy_manifest(DeployOptions {
        config_path: toml_path,
        server_url: "http://127.0.0.1:8080".into(),
        token: h.admin_token.clone(),
        target_env: ferropod::core::model::TargetEnv::Prod,
        dry_run: true,
    })
    .await
    .unwrap();
    assert!(dry
        .planned_actions
        .iter()
        .any(|a| a.contains("set secret 'JWT_SECRET'")));
    assert!(dry
        .planned_actions
        .iter()
        .any(|a| a.contains("sync AI provider 'openai'")));
    let _ = HashMap::<String, String>::new();
}
