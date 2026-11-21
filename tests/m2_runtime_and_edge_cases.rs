//! M2 Wasmtime runtime, WIT v0.2 host bindings, outbound HTTP, precompiled `.cwasm`,
//! epoch interruption, queue consumer dispatch, cron scheduling, and gateway protection tests.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tower::ServiceExt;

use ferropod::config::{Config, Profile};
use ferropod::core::model::{
    Bindings, Caller, EnqueueOpts, JobStatus, LogQuery, QueueConfig, TargetEnv, TokenScope,
};
use ferropod::core::service::{
    cron_matches_timestamp, CreateDeploymentInput, PlatformDeps, PlatformService,
};
use ferropod::core::traits::LogStore;
use ferropod::gateway::router::build_router;
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore,
    log_store::DailyLogStore, queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};

static TEST_WORKER_WASM: OnceLock<Vec<u8>> = OnceLock::new();

fn test_worker_wasm_bytes() -> Bytes {
    let bytes = TEST_WORKER_WASM.get_or_init(|| {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let worker_manifest = manifest_dir.join("tests/fixtures/test_worker/Cargo.toml");
        let target_dir = manifest_dir.join("target/test-worker");
        let status = Command::new("cargo")
            .args([
                "build",
                "--manifest-path",
                worker_manifest.to_str().unwrap(),
                "--target",
                "wasm32-wasip2",
                "--release",
                "--target-dir",
                target_dir.to_str().unwrap(),
            ])
            .status()
            .expect("failed to invoke cargo build for test_worker");
        assert!(status.success(), "building test_worker failed");
        let wasm_path = target_dir.join("wasm32-wasip2/release/test_worker.wasm");
        std::fs::read(&wasm_path).expect("failed to read compiled test_worker.wasm")
    });
    Bytes::from(bytes.clone())
}

struct M2Harness {
    tmp: TempDir,
    svc: PlatformService,
    logs: Arc<DailyLogStore>,
    admin: Caller,
    admin_token: String,
}

async fn setup_m2_harness(mutate_cfg: impl FnOnce(&mut Config)) -> M2Harness {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_profile(
        Profile::LowMem,
        "127.0.0.1:0".parse().unwrap(),
        tmp.path().to_path_buf(),
        "m2-integration-test-master-key-32b".to_string(),
    );
    cfg.blob_gc_grace_secs = 0;
    mutate_cfg(&mut cfg);

    tokio::fs::create_dir_all(cfg.artifacts_dir())
        .await
        .unwrap();

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

    let svc = PlatformService::new(
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
            logs: logs.clone(),
        },
    )
    .await
    .unwrap();

    let admin = Caller::system("m2-test");
    let (_meta, admin_token) = svc
        .create_token(&admin, "m2-admin", TokenScope::Admin, None)
        .await
        .unwrap();

    M2Harness {
        tmp,
        svc,
        logs,
        admin,
        admin_token,
    }
}

async fn api_json(
    app: &Router,
    method: Method,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, Bytes) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {token}"));
    let req = if let Some(val) = body {
        b = b.header(header::CONTENT_TYPE, "application/json");
        b.body(Body::from(val.to_string())).unwrap()
    } else {
        b.body(Body::empty()).unwrap()
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes)
}

#[tokio::test]
async fn http_worker_and_all_wit_v2_host_bindings() {
    let h = setup_m2_harness(|_| {}).await;
    let app = build_router(h.svc.clone());
    let wasm = test_worker_wasm_bytes();

    let proj = h
        .svc
        .create_project(&h.admin, "wit-app", None)
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "app-kv", Some(&proj.id))
        .await
        .unwrap();
    let db = h
        .svc
        .create_user_db(&h.admin, "app-db", Some(&proj.id))
        .await
        .unwrap();
    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "app-q".into(),
                project_id: Some(proj.id.clone()),
                consumer_project_id: None,
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
                created_at: 0,
            },
        )
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "app-bucket", Some(&proj.id), false)
        .await
        .unwrap();
    h.svc
        .set_secret(&h.admin, &proj.id, "API_SECRET", "top-secret-42")
        .await
        .unwrap();

    // Upload raw wasm artifact via REST API -> AOT compiled to .cwasm
    let upload_req = Request::builder()
        .method(Method::PUT)
        .uri(format!(
            "/_api/v1/projects/{}/artifacts/worker.wasm",
            proj.id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::from(wasm))
        .unwrap();
    let upload_resp = app.clone().oneshot(upload_req).await.unwrap();
    assert_eq!(upload_resp.status(), StatusCode::CREATED);
    let upload_json: serde_json::Value =
        serde_json::from_slice(&upload_resp.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    assert_eq!(upload_json["precompiled"], false);
    let artifact_path = upload_json["artifact_path"].as_str().unwrap().to_string();
    assert!(artifact_path.ends_with(".cwasm"));
    assert!(h.tmp.path().join("artifacts").join(&artifact_path).exists());

    // Create deployment with bindings & route
    let mut bindings = Bindings::default();
    bindings.kv.insert("KV".into(), "app-kv".into());
    bindings.db.insert("DB".into(), db.clone());
    bindings.queue.insert("QUEUE".into(), "app-q".into());
    bindings.bucket.insert("BUCKET".into(), "app-bucket".into());
    bindings.env.insert("APP_MODE".into(), "staging".into());

    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: Some(artifact_path),
                artifact_bytes: None,
                assets_bucket: None,
                assets_prefix: None,
                spa_fallback: false,
                bindings,
                crons: vec![],
                target_env: Some(TargetEnv::Prod),
            },
        )
        .await
        .unwrap();

    h.svc
        .upsert_route(&h.admin, "worker.example.test", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();

    // 1. Invoke via Host-routed HTTP gateway -> exercises all 6 WIT v0.2 host interfaces
    let gw_req = Request::builder()
        .method(Method::GET)
        .uri("/bindings")
        .header(header::HOST, "worker.example.test")
        .body(Body::empty())
        .unwrap();
    let gw_resp = app.clone().oneshot(gw_req).await.unwrap();
    assert_eq!(gw_resp.status(), StatusCode::OK);
    let body = gw_resp.into_body().collect().await.unwrap().to_bytes();
    let res: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(res["kv_val"], "v1");
    assert_eq!(res["kv_meta"], "m1");
    assert_eq!(res["absent"], false);
    assert_eq!(res["counter"], 5);
    assert_eq!(res["kv_pages"], serde_json::json!([2, 1]));
    assert_eq!(res["db_cols"], 3);
    assert_eq!(res["db_name"], "alpha");
    assert!(res["job_id"].as_i64().unwrap() > 0);
    assert_eq!(res["bucket_val"], "one");
    assert_eq!(res["bucket_pages"], serde_json::json!([2, 1]));
    assert_eq!(res["presigned_ok"], true);
    assert_eq!(res["env"], "staging");
    assert_eq!(res["secret"], "top-secret-42");

    // 2. Invoke via REST API POST /_api/v1/projects/{id}/invoke
    let (st, inv_bytes) = api_json(
        &app,
        Method::POST,
        &format!("/_api/v1/projects/{}/invoke", proj.id),
        &h.admin_token,
        Some(serde_json::json!({ "method": "POST", "path": "/hello?x=1" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let inv_json: serde_json::Value = serde_json::from_slice(&inv_bytes).unwrap();
    assert_eq!(inv_json["status"], 200);
    assert_eq!(inv_json["body"], "ok:Method::Post:/hello?x=1");

    // 3. Verify structured WIT log::emit + stdout + stderr were captured in LogStore
    h.logs.flush().await.unwrap();
    let entries = h
        .svc
        .query_logs(
            &h.admin,
            LogQuery {
                project_id: Some(proj.id.clone()),
                source: Some("runtime".into()),
                limit: Some(50),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let messages: Vec<&str> = entries.iter().map(|e| e.message.as_str()).collect();
    assert!(messages.iter().any(|m| m.contains("wit log emitted")));
    assert!(messages.iter().any(|m| m.contains("worker stdout line")));
    assert!(messages.iter().any(|m| m.contains("worker stderr line")));
    assert!(entries.iter().all(|e| e.request_id.is_some()));

    // 4. Gateway rejects external /_internal/* paths with 403 Forbidden
    for internal_path in ["/_internal", "/_internal/queue/app-q", "/_internal/cron"] {
        let req = Request::builder()
            .method(Method::POST)
            .uri(internal_path)
            .header(header::HOST, "worker.example.test")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}

#[tokio::test]
async fn worker_outbound_http_via_wasi_outgoing_handler() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let upstream_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let upstream_hits_srv = upstream_hits.clone();
    let upstream = Router::new().route(
        "/upstream",
        get(move || {
            let hits = upstream_hits_srv.clone();
            async move {
                hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                "upstream-pong"
            }
        }),
    );
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });

    // 1. Default config blocks private/loopback outbound requests (SSRF protection)
    let h_blocked = setup_m2_harness(|_| {}).await;
    let app_blocked = build_router(h_blocked.svc.clone());
    let proj_b = h_blocked
        .svc
        .create_project(&h_blocked.admin, "outbound-blocked", None)
        .await
        .unwrap();
    h_blocked
        .svc
        .create_deployment(
            &h_blocked.admin,
            &proj_b.id,
            CreateDeploymentInput {
                artifact_path: Some("outbound.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h_blocked
        .svc
        .upsert_route(
            &h_blocked.admin,
            "blocked.example.test",
            &proj_b.id,
            TargetEnv::Prod,
        )
        .await
        .unwrap();

    let blocked_resp = app_blocked
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/outbound")
                .header(header::HOST, "blocked.example.test")
                .header("x-upstream-authority", addr.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blocked_resp.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(upstream_hits.load(std::sync::atomic::Ordering::SeqCst), 0);

    // 2. Opt-in allows loopback HTTP except blocked_upstream (--local-api-upstream)
    let h_allowed = setup_m2_harness(|cfg| {
        cfg.wasm_allow_private_outbound = true;
    })
    .await;
    let app_allowed = build_router(h_allowed.svc.clone());
    let proj_a = h_allowed
        .svc
        .create_project(&h_allowed.admin, "outbound-app", None)
        .await
        .unwrap();
    h_allowed
        .svc
        .create_deployment(
            &h_allowed.admin,
            &proj_a.id,
            CreateDeploymentInput {
                artifact_path: Some("outbound.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h_allowed
        .svc
        .upsert_route(
            &h_allowed.admin,
            "outbound.example.test",
            &proj_a.id,
            TargetEnv::Prod,
        )
        .await
        .unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/outbound")
        .header(header::HOST, "outbound.example.test")
        .header("x-upstream-authority", addr.to_string())
        .body(Body::empty())
        .unwrap();
    let resp = app_allowed.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"200:upstream-pong");
    assert_eq!(upstream_hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 3. Even with wasm_allow_private_outbound=true, local_api.upstream remains blocked
    let h_local = setup_m2_harness(|cfg| {
        cfg.wasm_allow_private_outbound = true;
        cfg.local_api = Some(ferropod::config::LocalApiConfig {
            host: "site.example.test".into(),
            prefix: "/api".into(),
            upstream: addr,
        });
    })
    .await;
    let app_local = build_router(h_local.svc.clone());
    let proj_l = h_local
        .svc
        .create_project(&h_local.admin, "outbound-local-api", None)
        .await
        .unwrap();
    h_local
        .svc
        .create_deployment(
            &h_local.admin,
            &proj_l.id,
            CreateDeploymentInput {
                artifact_path: Some("outbound.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h_local
        .svc
        .upsert_route(
            &h_local.admin,
            "localblocked.example.test",
            &proj_l.id,
            TargetEnv::Prod,
        )
        .await
        .unwrap();
    let local_blocked = app_local
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/outbound")
                .header(header::HOST, "localblocked.example.test")
                .header("x-upstream-authority", addr.to_string())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(local_blocked.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(upstream_hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    server.abort();
}

#[tokio::test]
async fn precompiled_cwasm_upload_limits_and_epoch_timeout() {
    let h = setup_m2_harness(|cfg| {
        cfg.wasm_epoch_timeout_ms = 300;
        cfg.wasm_max_memory_bytes = 8 * 1024 * 1024;
    })
    .await;
    let app = build_router(h.svc.clone());
    let proj = h
        .svc
        .create_project(&h.admin, "cwasm-app", None)
        .await
        .unwrap();

    // 1. Upload raw wasm -> generates .cwasm on disk (precompiled == false)
    let (stored_path, precompiled, _) = h
        .svc
        .upload_project_artifact(&h.admin, &proj.id, "raw.wasm", test_worker_wasm_bytes())
        .await
        .unwrap();
    assert!(!precompiled);
    let cwasm_bytes =
        Bytes::from(std::fs::read(h.tmp.path().join("artifacts").join(&stored_path)).unwrap());

    // Uploading precompiled native code is rejected.
    let up_req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/_api/v1/projects/{}/artifacts/pre.cwasm", proj.id))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::from(cwasm_bytes))
        .unwrap();
    let up_resp = app.clone().oneshot(up_req).await.unwrap();
    assert_eq!(up_resp.status(), StatusCode::BAD_REQUEST);
    let pre_path = stored_path;

    // Invalid artifact is rejected cleanly with 400 Bad Request
    let bad_req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/_api/v1/projects/{}/artifacts/bad.wasm", proj.id))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::from(Bytes::from_static(b"not-a-wasm-component")))
        .unwrap();
    let bad_resp = app.clone().oneshot(bad_req).await.unwrap();
    assert_eq!(bad_resp.status(), StatusCode::BAD_REQUEST);

    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: Some(pre_path),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "pre.example.test", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();

    // Precompiled artifact runs normally
    let ok_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(header::HOST, "pre.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);
    assert_eq!(
        ok_resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        b"ok:Method::Get:/ping"
    );

    // Infinite loop traps via epoch_interruption -> 504 Gateway Timeout
    let loop_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/loop")
                .header(header::HOST, "pre.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(loop_resp.status(), StatusCode::GATEWAY_TIMEOUT);

    // Exceeding 8 MiB memory limit traps via StoreLimits -> 502 Bad Gateway
    let oom_resp = app
        .oneshot(
            Request::builder()
                .uri("/oom")
                .header(header::HOST, "pre.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oom_resp.status(), StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn queue_consumer_ack_retry_dead_and_dlq_replay() {
    let h = setup_m2_harness(|_| {}).await;
    let proj = h
        .svc
        .create_project(&h.admin, "q-consumer", None)
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "q-kv", Some(&proj.id))
        .await
        .unwrap();

    let mut kv_map = HashMap::new();
    kv_map.insert("KV".into(), "q-kv".into());
    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: Some("q.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                bindings: Bindings {
                    kv: kv_map,
                    ..Default::default()
                },
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "jobs".into(),
                project_id: Some(proj.id.clone()),
                consumer_project_id: Some(proj.id.clone()),
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
                created_at: 0,
            },
        )
        .await
        .unwrap();

    h.svc
        .queue_enqueue(
            &h.admin,
            "jobs",
            Bytes::from_static(b"ack"),
            EnqueueOpts::default(),
        )
        .await
        .unwrap();
    h.svc
        .queue_enqueue(
            &h.admin,
            "jobs",
            Bytes::from_static(b"retry"),
            EnqueueOpts::default(),
        )
        .await
        .unwrap();
    h.svc
        .queue_enqueue(
            &h.admin,
            "jobs",
            Bytes::from_static(b"dead"),
            EnqueueOpts::default(),
        )
        .await
        .unwrap();

    let count = h.svc.run_queue_dispatch_once().await.unwrap();
    assert_eq!(count, 3);

    let kv_ack = h
        .svc
        .kv_get(&h.admin, "q-kv", "queue_acked")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(kv_ack.value, b"yes");

    let stats = h.svc.queue_stats(&h.admin, "jobs").await.unwrap();
    assert_eq!(stats.pending, 1);
    assert_eq!(stats.running, 0);
    assert_eq!(stats.dead, 1);

    let dead_jobs = h
        .svc
        .queue_inspect(&h.admin, "jobs", JobStatus::Dead, 10)
        .await
        .unwrap();
    assert_eq!(dead_jobs.len(), 1);
    assert_eq!(dead_jobs[0].last_error.as_deref(), Some("dead requested"));

    let replayed = h
        .svc
        .queue_dlq_replay(&h.admin, "jobs", None)
        .await
        .unwrap();
    assert_eq!(replayed, 1);
    let stats_after = h.svc.queue_stats(&h.admin, "jobs").await.unwrap();
    assert_eq!(stats_after.dead, 0);
    assert_eq!(stats_after.pending, 2);
}

#[tokio::test]
async fn cron_matcher_and_scheduled_worker_execution() {
    // Unit checks for cron_matches_timestamp
    let mon_0230 = Utc
        .with_ymd_and_hms(2026, 11, 16, 2, 30, 0)
        .unwrap()
        .timestamp();
    let sat_0230 = Utc
        .with_ymd_and_hms(2026, 11, 21, 2, 30, 0)
        .unwrap()
        .timestamp();
    let hourly = Utc
        .with_ymd_and_hms(2026, 11, 21, 5, 0, 0)
        .unwrap()
        .timestamp();
    assert!(cron_matches_timestamp("* * * * *", mon_0230));
    assert!(cron_matches_timestamp("*/5 * * * *", mon_0230));
    assert!(!cron_matches_timestamp("*/7 * * * *", mon_0230));
    assert!(cron_matches_timestamp("30 2 * * 1-5", mon_0230));
    assert!(!cron_matches_timestamp("30 2 * * 1-5", sat_0230));
    assert!(cron_matches_timestamp("@hourly", hourly));
    assert!(!cron_matches_timestamp("@hourly", mon_0230));
    assert!(cron_matches_timestamp("@every 10s", 1_700_000_000));
    assert!(!cron_matches_timestamp("@every 10s", 1_700_000_003));

    // Integration check for scheduled Worker execution
    let h = setup_m2_harness(|_| {}).await;
    let proj = h
        .svc
        .create_project(&h.admin, "cron-worker", None)
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "cron-kv", Some(&proj.id))
        .await
        .unwrap();

    let mut kv_map = HashMap::new();
    kv_map.insert("KV".into(), "cron-kv".into());
    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: Some("cron.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                bindings: Bindings {
                    kv: kv_map,
                    ..Default::default()
                },
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let job = h
        .svc
        .create_cron_job(
            &h.admin,
            "nightly-job",
            Some(&proj.id),
            "@every 60s",
            "/nightly-sync",
        )
        .await
        .unwrap();

    let ran = h.svc.run_due_cron_jobs_once().await.unwrap();
    assert_eq!(ran, 1);

    let last_name = h
        .svc
        .kv_get(&h.admin, "cron-kv", "cron_last_name")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last_name.value, b"nightly-job");
    let last_target = h
        .svc
        .kv_get(&h.admin, "cron-kv", "cron_last_target")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last_target.value, b"/nightly-sync");

    // Second immediate tick does not re-fire inside the 60s window
    assert_eq!(h.svc.run_due_cron_jobs_once().await.unwrap(), 0);

    let triggered = h.svc.trigger_cron_job(&h.admin, &job.id).await.unwrap();
    assert_eq!(triggered.last_status.as_deref(), Some("ok"));

    let no_runtime = h
        .svc
        .create_cron_job(&h.admin, "no-runtime", None, "@every 60s", "/task")
        .await
        .unwrap();
    let result = h
        .svc
        .trigger_cron_job(&h.admin, &no_runtime.id)
        .await
        .unwrap();
    assert_eq!(
        result.last_status.as_deref(),
        Some("error: cron job has no app")
    );

    let empty = h
        .svc
        .create_project(&h.admin, "empty-cron", None)
        .await
        .unwrap();
    let no_deployment = h
        .svc
        .create_cron_job(
            &h.admin,
            "no-deployment",
            Some(&empty.id),
            "@every 60s",
            "/task",
        )
        .await
        .unwrap();
    let result = h
        .svc
        .trigger_cron_job(&h.admin, &no_deployment.id)
        .await
        .unwrap();
    assert_eq!(
        result.last_status.as_deref(),
        Some("error: app has no active deployment")
    );
}

#[tokio::test]
async fn cross_project_artifact_isolation_cache_invalidation_and_gc() {
    let h = setup_m2_harness(|_| {}).await;
    let app = build_router(h.svc.clone());
    let proj_a = h
        .svc
        .create_project(&h.admin, "iso-app-a", None)
        .await
        .unwrap();
    let proj_b = h
        .svc
        .create_project(&h.admin, "iso-app-b", None)
        .await
        .unwrap();

    let (path_a, _, _) = h
        .svc
        .upload_project_artifact(
            &h.admin,
            &proj_a.id,
            "worker.wasm",
            test_worker_wasm_bytes(),
        )
        .await
        .unwrap();
    assert!(h.tmp.path().join("artifacts").join(&path_a).exists());

    // App B cannot deploy App A's artifact (service + HTTP API)
    let svc_err = h
        .svc
        .create_deployment(
            &h.admin,
            &proj_b.id,
            CreateDeploymentInput {
                artifact_path: Some(path_a.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        svc_err,
        ferropod::core::model::FerroError::Forbidden(_)
    ));

    let rest_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/_api/v1/runtimes/{}/deployments", proj_b.id))
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({ "artifact_path": path_a }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rest_resp.status(), StatusCode::FORBIDDEN);

    // Nonexistent artifact path fails at create_deployment time
    let missing_err = h
        .svc
        .create_deployment(
            &h.admin,
            &proj_b.id,
            CreateDeploymentInput {
                artifact_path: Some("does-not-exist.wasm".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing_err,
        ferropod::core::model::FerroError::NotFound(_)
    ));

    // Delete App A removes its on-disk artifacts directory
    h.svc.delete_project(&h.admin, &proj_a.id).await.unwrap();
    assert!(!h.tmp.path().join("artifacts").join(&proj_a.id).exists());

    // Referenced deployment artifact survives GC while orphan is collected when grace == 0
    let (live_path, _, _) = h
        .svc
        .upload_project_artifact(&h.admin, &proj_b.id, "live.wasm", test_worker_wasm_bytes())
        .await
        .unwrap();
    h.svc
        .create_deployment(
            &h.admin,
            &proj_b.id,
            CreateDeploymentInput {
                artifact_path: Some(live_path.clone()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "gc.example.test", &proj_b.id, TargetEnv::Prod)
        .await
        .unwrap();

    let (orphan_path, _, _) = h
        .svc
        .upload_project_artifact(
            &h.admin,
            &proj_b.id,
            "orphan.wasm",
            test_worker_wasm_bytes(),
        )
        .await
        .unwrap();
    let orphan_disk = h.tmp.path().join("artifacts").join(&orphan_path);
    let live_disk = h.tmp.path().join("artifacts").join(&live_path);
    assert!(orphan_disk.exists());
    assert!(live_disk.exists());

    // Grace period preserves freshly uploaded unreferenced artifact
    assert_eq!(h.svc.run_artifact_gc_with_grace(300).await.unwrap(), 0);
    assert!(orphan_disk.exists());

    // Grace 0 collects orphan while preserving live artifact
    assert_eq!(h.svc.run_artifact_gc().await.unwrap(), 1);
    assert!(!orphan_disk.exists());
    assert!(live_disk.exists());

    let ping_resp = app
        .oneshot(
            Request::builder()
                .uri("/ping")
                .header(header::HOST, "gc.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ping_resp.status(), StatusCode::OK);
}
