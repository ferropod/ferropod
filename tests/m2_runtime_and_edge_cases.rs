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

    tokio::fs::create_dir_all(cfg.artifacts_dir()).await.unwrap();

    let core = Arc::new(TursoCoreStore::open(&cfg.core_db_path()).await.unwrap());
    let kv = Arc::new(
        TursoKvStore::open(&cfg.kv_db_path(), cfg.max_ephemeral_kv_entries)
            .await
            .unwrap(),
    );
    let queue = Arc::new(TursoQueueStore::open(&cfg.queue_db_path()).await.unwrap());
    let user_dbs = Arc::new(
        TursoUserDbManager::new(cfg.user_dbs_dir(), cfg.max_open_user_dbs).unwrap(),
    );
    let blobs = Arc::new(
        FsBlobStore::new(cfg.blobs_dir(), core.clone(), cfg.blob_gc_grace_secs).unwrap(),
    );
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
        .uri(format!("/_api/v1/projects/{}/artifacts/worker.wasm", proj.id))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::from(wasm))
        .unwrap();
    let upload_resp = app.clone().oneshot(upload_req).await.unwrap();
    assert_eq!(upload_resp.status(), StatusCode::CREATED);
    let upload_json: serde_json::Value = serde_json::from_slice(
        &upload_resp.into_body().collect().await.unwrap().to_bytes(),
    )
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
    let upstream = Router::new().route("/upstream", get(|| async { "upstream-pong" }));
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, upstream).await;
    });

    let h = setup_m2_harness(|_| {}).await;
    let app = build_router(h.svc.clone());
    let proj = h
        .svc
        .create_project(&h.admin, "outbound-app", None)
        .await
        .unwrap();
    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: Some("outbound.wasm".into()),
                artifact_bytes: Some(test_worker_wasm_bytes()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "outbound.example.test", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();

    let req = Request::builder()
        .method(Method::GET)
        .uri("/outbound")
        .header(header::HOST, "outbound.example.test")
        .header("x-upstream-authority", addr.to_string())
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(body.as_ref(), b"200:upstream-pong");

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

    // 2. Upload precompiled .cwasm bytes via HTTP -> precompiled == true
    let up_req = Request::builder()
        .method(Method::PUT)
        .uri(format!("/_api/v1/projects/{}/artifacts/pre.cwasm", proj.id))
        .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
        .body(Body::from(cwasm_bytes))
        .unwrap();
    let up_resp = app.clone().oneshot(up_req).await.unwrap();
    assert_eq!(up_resp.status(), StatusCode::CREATED);
    let up_json: serde_json::Value =
        serde_json::from_slice(&up_resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(up_json["precompiled"], true);
    let pre_path = up_json["artifact_path"].as_str().unwrap().to_string();

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
        ok_resp.into_body().collect().await.unwrap().to_bytes().as_ref(),
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
        .queue_enqueue(&h.admin, "jobs", Bytes::from_static(b"ack"), EnqueueOpts::default())
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

    let kv_ack = h.svc.kv_get(&h.admin, "q-kv", "queue_acked").await.unwrap().unwrap();
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

    let replayed = h.svc.queue_dlq_replay(&h.admin, "jobs", None).await.unwrap();
    assert_eq!(replayed, 1);
    let stats_after = h.svc.queue_stats(&h.admin, "jobs").await.unwrap();
    assert_eq!(stats_after.dead, 0);
    assert_eq!(stats_after.pending, 2);
}

#[tokio::test]
async fn cron_matcher_and_scheduled_worker_execution() {
    // Unit checks for cron_matches_timestamp
    let mon_0230 = Utc.with_ymd_and_hms(2026, 11, 16, 2, 30, 0).unwrap().timestamp();
    let sat_0230 = Utc.with_ymd_and_hms(2026, 11, 21, 2, 30, 0).unwrap().timestamp();
    let hourly = Utc.with_ymd_and_hms(2026, 11, 21, 5, 0, 0).unwrap().timestamp();
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
}
