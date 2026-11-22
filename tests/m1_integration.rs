use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request as AxumRequest;
use axum::http::{header, Method, Request, StatusCode};
use axum::routing::any;
use axum::{Json, Router};
use bytes::Bytes;
use chrono::Utc;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tower::ServiceExt;

use ferropod::config::{Config, LocalApiConfig, Profile};
use ferropod::core::model::{
    Bindings, BlobAuth, Caller, EnqueueOpts, JobStatus, LogEntry, LogQuery, QueueConfig, SqlValue,
    TargetEnv, TokenScope,
};
use ferropod::core::service::{CreateDeploymentInput, PlatformDeps, PlatformService};
use ferropod::core::traits::{DbManager, KvStore, LogStore, ObjectStore, QueueStore};
use ferropod::gateway::router::build_router;
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore,
    log_store::DailyLogStore, queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};

struct TestHarness {
    _tmp: TempDir,
    svc: PlatformService,
    kv: Arc<TursoKvStore>,
    queue: Arc<TursoQueueStore>,
    user_dbs: Arc<TursoUserDbManager>,
    blobs: Arc<FsBlobStore>,
    logs: Arc<DailyLogStore>,
    admin: Caller,
    admin_token: String,
}

async fn setup_harness() -> TestHarness {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_profile(
        Profile::LowMem,
        "127.0.0.1:0".parse().unwrap(),
        tmp.path().to_path_buf(),
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

    let svc = PlatformService::new(
        cfg,
        PlatformDeps {
            projects: core.clone(),
            secrets: core.clone(),
            tokens: core.clone(),
            registry: core.clone(),
            kv: kv.clone(),
            queue: queue.clone(),
            objects: blobs.clone(),
            db: user_dbs.clone(),
            logs: logs.clone(),
        },
    )
    .await
    .unwrap();

    let admin = Caller::system("test");
    let (_meta, admin_token) = svc
        .create_token(&admin, "test-admin", TokenScope::Admin, None)
        .await
        .unwrap();

    TestHarness {
        _tmp: tmp,
        svc,
        kv,
        queue,
        user_dbs,
        blobs,
        logs,
        admin,
        admin_token,
    }
}

async fn json_req(
    app: &axum::Router,
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, Bytes) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(tok) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {tok}"));
    }
    let req = if let Some(val) = body {
        builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(val.to_string()))
            .unwrap()
    } else {
        builder.body(Body::empty()).unwrap()
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes)
}

#[tokio::test]
async fn kv_persistent_ttl_atomic_and_ephemeral() {
    let h = setup_harness().await;

    // Basic put/get
    h.svc
        .kv_put(
            &h.admin,
            "ns1",
            "k1",
            Bytes::from("hello"),
            Some("meta1".into()),
            Some(60),
            false,
        )
        .await
        .unwrap();
    let got = h.svc.kv_get(&h.admin, "ns1", "k1").await.unwrap().unwrap();
    assert_eq!(got.value, b"hello");
    assert_eq!(got.metadata.as_deref(), Some("meta1"));

    // Logical TTL expiry (ttl = 0 -> expires_at == now, <= now so immediately expired)
    h.svc
        .kv_put(
            &h.admin,
            "ns1",
            "expired_key",
            Bytes::from("old"),
            None,
            Some(0),
            false,
        )
        .await
        .unwrap();
    assert!(h
        .svc
        .kv_get(&h.admin, "ns1", "expired_key")
        .await
        .unwrap()
        .is_none());

    // Batch cleanup deletes expired persistent key
    let cleaned = h.kv.cleanup_expired_batch(500).await.unwrap();
    assert!(cleaned >= 1);

    // put_if_absent atomicity
    assert!(h
        .svc
        .kv_put_if_absent(
            &h.admin,
            "ns1",
            "lock:1",
            Bytes::from("owner-a"),
            None,
            Some(60)
        )
        .await
        .unwrap());
    assert!(!h
        .svc
        .kv_put_if_absent(
            &h.admin,
            "ns1",
            "lock:1",
            Bytes::from("owner-b"),
            None,
            Some(60)
        )
        .await
        .unwrap());
    // Expired lock can be acquired again
    assert!(h
        .svc
        .kv_put_if_absent(
            &h.admin,
            "ns1",
            "expired_key",
            Bytes::from("reborn"),
            None,
            Some(60)
        )
        .await
        .unwrap());

    // incr
    assert_eq!(
        h.svc
            .kv_incr(&h.admin, "ns1", "hits", 1, Some(60))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        h.svc
            .kv_incr(&h.admin, "ns1", "hits", 4, None)
            .await
            .unwrap(),
        5
    );

    // Ephemeral KV does not hit persistent DB but is readable & listable
    h.svc
        .kv_put(
            &h.admin,
            "ns1",
            "eph:rate",
            Bytes::from("9"),
            None,
            Some(30),
            true,
        )
        .await
        .unwrap();
    let eph = h
        .svc
        .kv_get(&h.admin, "ns1", "eph:rate")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(eph.value, b"9");

    let listed = h.svc.kv_list(&h.admin, "ns1", None, 50).await.unwrap();
    let keys: Vec<_> = listed.into_iter().map(|k| k.key).collect();
    assert!(keys.contains(&"k1".to_string()));
    assert!(keys.contains(&"eph:rate".to_string()));
}

#[tokio::test]
async fn queue_microbatch_claim_ack_nack_dlq_and_lease_reap() {
    let h = setup_harness().await;

    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "jobs".into(),
                project_id: None,
                consumer_project_id: None,
                description: None,
                max_concurrency: 4,
                rate_limit_per_sec: None,
                batch_size: 20,
                lease_secs: 30,
                max_attempts: 2,
                retry_backoff_secs: 5,
                dlq_retention_secs: None,
                ephemeral: false,
                max_pending_jobs: None,
                created_at: Utc::now().timestamp(),
            },
        )
        .await
        .unwrap();

    // Concurrent micro-batch enqueue
    let mut futs = Vec::new();
    for i in 0..10 {
        let q = h.queue.clone();
        futs.push(tokio::spawn(async move {
            q.enqueue(
                "jobs",
                Bytes::from(format!("{{\"i\":{i}}}")),
                EnqueueOpts {
                    max_attempts: Some(2),
                    priority: Some(i),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
        }));
    }
    let mut ids = Vec::new();
    for f in futs {
        ids.push(f.await.unwrap());
    }
    assert_eq!(ids.len(), 10);

    // Dedup key returns existing id
    let d1 = h
        .queue
        .enqueue(
            "jobs",
            Bytes::from("dedup"),
            EnqueueOpts {
                dedup_key: Some("once".into()),
                max_attempts: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let d2 = h
        .queue
        .enqueue(
            "jobs",
            Bytes::from("dedup-dup"),
            EnqueueOpts {
                dedup_key: Some("once".into()),
                max_attempts: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(d1, d2);

    let stats = h.queue.stats("jobs").await.unwrap();
    assert_eq!(stats.pending, 11);

    // Claim batch (ordered by priority DESC), ACK first 9 immediately (deleted)
    let claimed = h.queue.claim_batch("jobs", 11, 60).await.unwrap();
    assert_eq!(claimed.len(), 11);
    let ack_ids: Vec<i64> = claimed[..9].iter().map(|j| j.id).collect();
    h.queue.ack_batch(&ack_ids).await.unwrap();

    // NACK 10th job: attempt 1 < max_attempts(2) -> pending; then claim & NACK again -> dead
    let fail_id = claimed[9].id;
    let st1 = h.queue.nack(fail_id, 0, "boom-1").await.unwrap().unwrap();
    assert_eq!(st1, JobStatus::Pending);
    let reclaimed = h.queue.claim_batch("jobs", 5, 60).await.unwrap();
    assert!(reclaimed.iter().any(|j| j.id == fail_id && j.attempts == 2));
    let st2 = h.queue.nack(fail_id, 0, "boom-2").await.unwrap().unwrap();
    assert_eq!(st2, JobStatus::Dead);

    // Replay DLQ -> becomes pending again
    let replayed = h.queue.dlq_replay("jobs", Some(&[fail_id])).await.unwrap();
    assert_eq!(replayed, 1);

    // Claim with expired lease (-1s) and test reaper
    let _ = h.queue.claim_batch("jobs", 5, -1).await.unwrap();
    let reaped = h.queue.reap_expired().await.unwrap();
    assert!(reaped >= 1);
}

#[tokio::test]
async fn user_db_ddl_dml_schema_and_snapshot() {
    let h = setup_harness().await;
    let db_id = h
        .svc
        .create_user_db(&h.admin, "app_main", None)
        .await
        .unwrap();

    let affected = h
        .svc
        .execute_user_db_batch(
            &h.admin,
            "app_main",
            vec![
                (
                    "CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL)".into(),
                    vec![],
                ),
                (
                    "INSERT INTO posts (title) VALUES (?1)".into(),
                    vec![SqlValue::Text("Hello Turso".into())],
                ),
            ],
        )
        .await
        .unwrap();
    assert_eq!(affected, 1);

    let res = h
        .svc
        .query_user_db(
            &h.admin,
            &db_id,
            "SELECT id, title FROM posts WHERE title = ?1",
            vec![SqlValue::Text("Hello Turso".into())],
        )
        .await
        .unwrap();
    assert_eq!(res.columns, vec!["id", "title"]);
    assert_eq!(res.rows.len(), 1);
    assert_eq!(res.rows[0][1], serde_json::json!("Hello Turso"));

    let schema = h.svc.schema_user_db(&h.admin, "app_main").await.unwrap();
    assert!(schema.contains("CREATE TABLE posts"));

    let snap_dir = h._tmp.path().join("snap");
    let count = h.svc.snapshot_user_dbs(&h.admin, &snap_dir).await.unwrap();
    assert_eq!(count, 1);
    assert!(snap_dir.join(format!("app_main__{db_id}.db")).exists());
    let _ = h.user_dbs.checkpoint_all().await;
}

#[tokio::test]
async fn blob_api_static_site_secrets_tls_and_logs() {
    let h = setup_harness().await;

    // Create bucket (private first, then test presigned upload + public static site)
    h.svc
        .create_bucket(&h.admin, "site-assets", None, false)
        .await
        .unwrap();

    let app = build_router(h.svc.clone());

    // 1. Presigned PUT
    let (presign_put, _) = h
        .svc
        .presign_blob_url(&h.admin, "put", "site-assets", "index.html", 120)
        .await
        .unwrap();
    let html_body = "<html><body><h1>Hello Ferropod</h1></body></html>";
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(&presign_put)
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from(html_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // 2. Bearer GET + Range + ETag 304
    let resp_get = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/_blob/site-assets/index.html")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_get.status(), StatusCode::OK);
    let etag = resp_get
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body_bytes = resp_get.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body_bytes[..], html_body.as_bytes());

    let resp_304 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/_blob/site-assets/index.html")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_304.status(), StatusCode::NOT_MODIFIED);

    let resp_range = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/_blob/site-assets/index.html")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::RANGE, "bytes=0-5")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_range.status(), StatusCode::PARTIAL_CONTENT);
    let range_bytes = resp_range.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&range_bytes[..], b"<html>");

    // 3. Project + Preview -> Promote -> Static site routing + TLS allow check
    let proj = h
        .svc
        .create_project(&h.admin, "demo-web", None)
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "demo:cache", Some(&proj.id))
        .await
        .unwrap();
    let mut kv_map = HashMap::new();
    kv_map.insert("CACHE".into(), "demo:cache".into());
    let dep_preview = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: None,
                artifact_bytes: None,
                assets_bucket: Some("site-assets".into()),
                assets_prefix: None,
                spa_fallback: true,
                bindings: Bindings {
                    kv: kv_map,
                    ..Default::default()
                },
                crons: vec![],
                target_env: Some(TargetEnv::Preview),
            },
        )
        .await
        .unwrap();

    h.svc
        .upsert_route(&h.admin, "demo.local.dev", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();
    h.svc
        .upsert_route(
            &h.admin,
            "preview.demo.local.dev",
            &proj.id,
            TargetEnv::Preview,
        )
        .await
        .unwrap();

    // Native domain allowlist checks (used by built-in TLS/ACME)
    assert!(h.svc.is_domain_allowed("demo.local.dev").await);
    assert!(!h.svc.is_domain_allowed("evil.example.com").await);
    assert_eq!(
        h.svc.allowed_domains().await,
        vec![
            "demo.local.dev".to_string(),
            "preview.demo.local.dev".to_string()
        ]
    );

    // Before promote, prod has no active deployment (503), preview works + SPA fallback works
    let prod_before = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::HOST, "demo.local.dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prod_before.status(), StatusCode::SERVICE_UNAVAILABLE);

    let preview_spa = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard/settings")
                .header(header::HOST, "preview.demo.local.dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(preview_spa.status(), StatusCode::OK);

    // Promote preview -> prod
    h.svc
        .promote_deployment(&h.admin, &proj.id, Some(&dep_preview.id))
        .await
        .unwrap();
    let prod_after = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::HOST, "demo.local.dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prod_after.status(), StatusCode::OK);

    // Secrets roundtrip + key-only listing
    h.svc
        .set_secret(&h.admin, &proj.id, "OPENAI_KEY", "sk-secret-xyz")
        .await
        .unwrap();
    assert_eq!(
        h.svc.list_secret_keys(&h.admin, &proj.id).await.unwrap(),
        vec!["OPENAI_KEY".to_string()]
    );
    assert_eq!(
        h.svc
            .get_secret(&h.admin, &proj.id, "OPENAI_KEY")
            .await
            .unwrap()
            .as_deref(),
        Some("sk-secret-xyz")
    );

    // Blob DELETE + GC removes disk payload
    let meta = h
        .svc
        .head_object(&BlobAuth::Caller(&h.admin), "site-assets", "index.html")
        .await
        .unwrap()
        .unwrap();
    let disk_blob = h._tmp.path().join("blobs").join(&meta.blob_path);
    assert!(disk_blob.exists());
    let del_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri("/_blob/site-assets/index.html")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(del_resp.status(), StatusCode::OK);
    let gc_count = h.blobs.run_gc_batch(100).await.unwrap();
    assert_eq!(gc_count, 1);
    assert!(!disk_blob.exists());

    // Daily log pipeline write + query
    h.svc.emit_log(LogEntry {
        id: None,
        ts: Utc::now().timestamp_millis(),
        project_id: Some(proj.id.clone()),
        request_id: Some("req_test_123".into()),
        source: "runtime".into(),
        level: "info".into(),
        message: "worker ok".into(),
        meta_json: None,
    });
    let logs = h
        .logs
        .query(LogQuery {
            request_id: Some("req_test_123".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].message, "worker ok");
}

#[tokio::test]
async fn multi_project_rbac_and_isolation() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());

    let proj_a = h
        .svc
        .create_project(&h.admin, "proj-a", None)
        .await
        .unwrap();
    let proj_b = h
        .svc
        .create_project(&h.admin, "proj-b", None)
        .await
        .unwrap();

    let (_, tok_a) = h
        .svc
        .create_token(
            &h.admin,
            "token-a",
            TokenScope::Project(proj_a.id.clone()),
            None,
        )
        .await
        .unwrap();
    let (_, tok_b) = h
        .svc
        .create_token(
            &h.admin,
            "token-b",
            TokenScope::Project(proj_b.id.clone()),
            None,
        )
        .await
        .unwrap();
    let (_, tok_ro) = h
        .svc
        .create_token(&h.admin, "token-ro", TokenScope::Readonly, None)
        .await
        .unwrap();

    // Provision per-project resources
    h.svc
        .create_kv_namespace(&h.admin, "ns_a", Some(&proj_a.id))
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "ns_b", Some(&proj_b.id))
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "ns_shared_none", None)
        .await
        .unwrap();

    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "q_a".into(),
                project_id: Some(proj_a.id.clone()),
                consumer_project_id: None,
                description: None,
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
                retry_backoff_secs: 5,
                dlq_retention_secs: None,
                ephemeral: false,
                max_pending_jobs: None,
                created_at: Utc::now().timestamp(),
            },
        )
        .await
        .unwrap();
    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "q_b".into(),
                project_id: Some(proj_b.id.clone()),
                consumer_project_id: Some(proj_a.id.clone()),
                description: None,
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
                retry_backoff_secs: 5,
                dlq_retention_secs: None,
                ephemeral: false,
                max_pending_jobs: None,
                created_at: Utc::now().timestamp(),
            },
        )
        .await
        .unwrap();

    h.svc
        .create_bucket(&h.admin, "bkt-a", Some(&proj_a.id), false)
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "bkt-b", Some(&proj_b.id), false)
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "bkt-pub", Some(&proj_b.id), true)
        .await
        .unwrap();

    let db_a = h
        .svc
        .create_user_db(&h.admin, "db_a", Some(&proj_a.id))
        .await
        .unwrap();
    let db_b = h
        .svc
        .create_user_db(&h.admin, "db_b", Some(&proj_b.id))
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &db_a,
            vec![
                (
                    "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT NOT NULL)"
                        .into(),
                    vec![],
                ),
                ("INSERT INTO items (v) VALUES ('alpha')".into(), vec![]),
            ],
        )
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &db_b,
            vec![(
                "CREATE TABLE secrets_b (id INTEGER PRIMARY KEY, val TEXT)".into(),
                vec![],
            )],
        )
        .await
        .unwrap();

    h.svc
        .set_secret(&h.admin, &proj_a.id, "SECRET_A", "val-a")
        .await
        .unwrap();
    h.svc
        .set_secret(&h.admin, &proj_b.id, "SECRET_B", "val-b")
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "a.example.dev", &proj_a.id, TargetEnv::Prod)
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "b.example.dev", &proj_b.id, TargetEnv::Prod)
        .await
        .unwrap();

    // 1. Projects & Deployments isolation
    let (st, body) = json_req(&app, Method::GET, "/_api/v1/projects", Some(&tok_a), None).await;
    assert_eq!(st, StatusCode::OK);
    let list: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], proj_a.id);

    let (st, _) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/projects/{}", proj_b.id),
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Cross-project resource bindings in create_deployment are rejected
    for bad_dep in [
        serde_json::json!({ "assets_bucket": "bkt-b" }),
        serde_json::json!({ "bindings": { "kv": { "K": "ns_b" } } }),
        serde_json::json!({ "bindings": { "db": { "D": db_b } } }),
        serde_json::json!({ "bindings": { "bucket": { "B": "bkt-b" } } }),
    ] {
        let (st, _) = json_req(
            &app,
            Method::POST,
            &format!("/_api/v1/projects/{}/deployments", proj_a.id),
            Some(&tok_a),
            Some(bad_dep),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }

    // 2. Routes isolation (cannot hijack or delete another project's route)
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        Some(&tok_a),
        Some(serde_json::json!({ "host": "b.example.dev", "project": proj_a.id, "target_env": "prod" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::DELETE,
        "/_api/v1/routes/b.example.dev",
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        Some(&tok_a),
        Some(serde_json::json!({ "host": "a.example.dev", "project": proj_a.id, "target_env": "preview" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // 3. Secrets isolation (cross-project + readonly cannot read/write)
    let (st, _) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/projects/{}/secrets", proj_b.id),
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/projects/{}/secrets", proj_a.id),
        Some(&tok_ro),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    assert!(h
        .svc
        .get_secret(&Caller::readonly("ro"), &proj_a.id, "SECRET_A")
        .await
        .is_err());

    // 4. KV isolation
    let (st, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/ns_a/foo",
        Some(&tok_a),
        Some(serde_json::json!({ "value": "bar" })),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    for bad_ns in ["ns_b", "ns_shared_none", "nonexistent_ns"] {
        let (st, _) = json_req(
            &app,
            Method::GET,
            &format!("/_api/v1/kv/{bad_ns}/foo"),
            Some(&tok_a),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        let (st, _) = json_req(
            &app,
            Method::PUT,
            &format!("/_api/v1/kv/{bad_ns}/foo"),
            Some(&tok_a),
            Some(serde_json::json!({ "value": "x" })),
        )
        .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
    }

    let (st, _) = json_req(
        &app,
        Method::GET,
        "/_api/v1/kv/ns_a/foo",
        Some(&tok_ro),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/ns_a/foo",
        Some(&tok_ro),
        Some(serde_json::json!({ "value": "x" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 5. Queue isolation (producer vs consumer vs foreign)
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/q_a/enqueue",
        Some(&tok_a),
        Some(serde_json::json!({ "payload": { "ok": true } })),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/q_a/enqueue",
        Some(&tok_b),
        Some(serde_json::json!({ "payload": { "bad": true } })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::GET,
        "/_api/v1/queues/q_a/stats",
        Some(&tok_b),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // consumer_project_id allows A to read stats of q_b, but NOT enqueue into q_b
    let (st, _) = json_req(
        &app,
        Method::GET,
        "/_api/v1/queues/q_b/stats",
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/q_b/enqueue",
        Some(&tok_a),
        Some(serde_json::json!({ "payload": {} })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 6. Blob & Presign isolation
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets/bkt-b/presign",
        Some(&tok_a),
        Some(serde_json::json!({ "key": "x.txt", "op": "put" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets/bkt-b/presign",
        Some(&tok_a),
        Some(serde_json::json!({ "key": "x.txt", "op": "get" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets/bkt-a/presign",
        Some(&tok_ro),
        Some(serde_json::json!({ "key": "x.txt", "op": "put" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets/bkt-a/presign",
        Some(&tok_ro),
        Some(serde_json::json!({ "key": "x.txt", "op": "get" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Put object into bkt-a using tok_a and into bkt-pub using tok_b
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/bkt-a/secret.txt")
                .header(header::AUTHORIZATION, format!("Bearer {tok_a}"))
                .body(Body::from("top-secret-a"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/bkt-pub/hello.txt")
                .header(header::AUTHORIZATION, format!("Bearer {tok_b}"))
                .body(Body::from("public-hello"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    // tok_b cannot read or list private bkt-a
    let (st, _) = json_req(
        &app,
        Method::GET,
        "/_blob/bkt-a/secret.txt",
        Some(&tok_b),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = json_req(&app, Method::GET, "/_blob/bkt-a", Some(&tok_b), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Anonymous gets 401 on private bucket, 200 on public bucket
    let (st, _) = json_req(&app, Method::GET, "/_blob/bkt-a/secret.txt", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _) = json_req(&app, Method::GET, "/_blob/bkt-pub/hello.txt", None, None).await;
    assert_eq!(st, StatusCode::OK);

    // 7. User DB isolation & Read-Only SQL guard
    let (st, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_b}/query"),
        Some(&tok_a),
        Some(serde_json::json!({ "sql": "SELECT 1" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/dbs/{db_b}/schema"),
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_a}/execute"),
        Some(&tok_ro),
        Some(serde_json::json!({ "statements": [{ "sql": "DELETE FROM items" }] })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Even admin cannot mutate through /query
    for bad_sql in [
        "DELETE FROM items",
        "INSERT INTO items (v) VALUES ('evil') RETURNING id",
        "/* c */ UPDATE items SET v = 'evil'",
        "SELECT * FROM items; DROP TABLE items",
        "WITH d AS (DELETE FROM items RETURNING *) SELECT * FROM d",
        "PRAGMA writable_schema = ON",
    ] {
        let (st, _) = json_req(
            &app,
            Method::POST,
            &format!("/_api/v1/dbs/{db_a}/query"),
            Some(&h.admin_token),
            Some(serde_json::json!({ "sql": bad_sql })),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::BAD_REQUEST,
            "SQL should be rejected: {bad_sql}"
        );
    }

    // Legitimate SELECT + REPLACE() scalar function succeeds and data is untouched
    let (st, body) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_a}/query"),
        Some(&tok_a),
        Some(serde_json::json!({ "sql": "SELECT REPLACE(v, 'a', 'A') FROM items" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let qres: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(qres["rows"][0][0], "AlphA");

    // 8. Logs query & live subscription isolation
    let mut sub_a = h.svc.subscribe_logs(&Caller::project("tok_a", &proj_a.id));
    h.svc.emit_log(LogEntry {
        id: None,
        ts: Utc::now().timestamp_millis(),
        project_id: Some(proj_b.id.clone()),
        request_id: Some("r_b".into()),
        source: "runtime".into(),
        level: "info".into(),
        message: "secret b log".into(),
        meta_json: None,
    });
    h.svc.emit_log(LogEntry {
        id: None,
        ts: Utc::now().timestamp_millis(),
        project_id: None,
        request_id: None,
        source: "audit".into(),
        level: "info".into(),
        message: "global audit".into(),
        meta_json: None,
    });
    h.svc.emit_log(LogEntry {
        id: None,
        ts: Utc::now().timestamp_millis(),
        project_id: Some(proj_a.id.clone()),
        request_id: Some("r_a".into()),
        source: "runtime".into(),
        level: "info".into(),
        message: "visible a log".into(),
        meta_json: None,
    });

    let next_for_a = sub_a.recv().await.unwrap();
    assert_eq!(next_for_a.project_id.as_deref(), Some(proj_a.id.as_str()));
    assert_eq!(next_for_a.message, "visible a log");

    let (st, _) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/logs?project_id={}", proj_b.id),
        Some(&tok_a),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, body) = json_req(&app, Method::GET, "/_api/v1/logs", Some(&tok_a), None).await;
    assert_eq!(st, StatusCode::OK);
    let entries: Vec<LogEntry> = serde_json::from_slice(&body).unwrap();
    assert!(entries
        .iter()
        .all(|e| e.project_id.as_deref() == Some(proj_a.id.as_str())));
    assert!(entries.iter().any(|e| e.message == "visible a log"));
}

#[tokio::test]
async fn atomicity_kv_edge_cases_and_route_concurrency() {
    let h = setup_harness().await;

    // 1. Deployment status transitions (Preview and Prod demote prior deployments atomically)
    let proj = h
        .svc
        .create_project(&h.admin, "atomic-app", None)
        .await
        .unwrap();
    let p1 = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: None,
                artifact_bytes: None,
                assets_bucket: None,
                assets_prefix: None,
                spa_fallback: false,
                bindings: Bindings::default(),
                crons: vec![],
                target_env: Some(TargetEnv::Preview),
            },
        )
        .await
        .unwrap();
    let p2 = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: None,
                artifact_bytes: None,
                assets_bucket: None,
                assets_prefix: None,
                spa_fallback: false,
                bindings: Bindings::default(),
                crons: vec![],
                target_env: Some(TargetEnv::Preview),
            },
        )
        .await
        .unwrap();
    let deps = h.svc.list_deployments(&h.admin, &proj.id).await.unwrap();
    let p1_after = deps.iter().find(|d| d.id == p1.id).unwrap();
    let p2_after = deps.iter().find(|d| d.id == p2.id).unwrap();
    assert_eq!(p1_after.status, "ready");
    assert_eq!(p2_after.status, "preview");

    // 2. Concurrent route upserts do not clobber each other in hot route table
    let mut route_futs = Vec::new();
    for i in 0..10 {
        let svc = h.svc.clone();
        let pid = proj.id.clone();
        route_futs.push(tokio::spawn(async move {
            let caller = Caller::system("concurrent");
            svc.upsert_route(
                &caller,
                &format!("host-{i}.local"),
                &pid,
                TargetEnv::Preview,
            )
            .await
            .unwrap();
        }));
    }
    for f in route_futs {
        f.await.unwrap();
    }
    for i in 0..10 {
        assert!(
            h.svc
                .lookup_route(&format!("host-{i}.local"))
                .await
                .is_some(),
            "missing host-{i}.local after concurrent refresh_routes"
        );
    }

    // 3. KV incr on expired key resets expired TTL & clears shadowing ephemeral value
    h.svc
        .kv_put(
            &h.admin,
            "ns_edge",
            "cnt",
            Bytes::from("99"),
            None,
            Some(0),
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        h.svc
            .kv_incr(&h.admin, "ns_edge", "cnt", 1, None)
            .await
            .unwrap(),
        1
    );
    let val = h
        .svc
        .kv_get(&h.admin, "ns_edge", "cnt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(val.value, b"1");

    // Ephemeral interaction with put_if_absent and incr
    h.svc
        .kv_put(
            &h.admin,
            "ns_edge",
            "eph_key",
            Bytes::from("10"),
            None,
            Some(60),
            true,
        )
        .await
        .unwrap();
    assert!(!h
        .svc
        .kv_put_if_absent(
            &h.admin,
            "ns_edge",
            "eph_key",
            Bytes::from("new"),
            None,
            Some(60),
        )
        .await
        .unwrap());
    assert_eq!(
        h.svc
            .kv_incr(&h.admin, "ns_edge", "eph_key", 2, None)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        h.svc
            .kv_get(&h.admin, "ns_edge", "eph_key")
            .await
            .unwrap()
            .unwrap()
            .value,
        b"2"
    );

    // 4. Static site Range + 416 handling
    h.svc
        .create_bucket(&h.admin, "static-range", Some(&proj.id), false)
        .await
        .unwrap();
    h.svc
        .put_object_stream(
            &BlobAuth::Caller(&h.admin),
            "static-range",
            "app.js",
            Box::pin(futures_util::stream::once(async {
                Ok(Bytes::from_static(b"console.log('hi');"))
            })),
            "application/javascript",
            None,
        )
        .await
        .unwrap();
    let d_prod = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                artifact_path: None,
                artifact_bytes: None,
                assets_bucket: Some("static-range".into()),
                assets_prefix: None,
                spa_fallback: false,
                bindings: Bindings::default(),
                crons: vec![],
                target_env: Some(TargetEnv::Prod),
            },
        )
        .await
        .unwrap();
    assert_eq!(d_prod.status, "active");
    h.svc
        .upsert_route(&h.admin, "static.local.dev", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();

    let app = build_router(h.svc.clone());
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .header(header::HOST, "static.local.dev")
                .header(header::RANGE, "bytes=0-6")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    let partial = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&partial[..], b"console");

    let resp_416 = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/app.js")
                .header(header::HOST, "static.local.dev")
                .header(header::RANGE, "bytes=999-1000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_416.status(), StatusCode::RANGE_NOT_SATISFIABLE);
}

#[tokio::test]
async fn admin_ui_assets_and_enriched_status() {
    let h = setup_harness().await;
    let proj = h
        .svc
        .create_project(&h.admin, "ui-app", None)
        .await
        .unwrap();
    let (_, proj_token) = h
        .svc
        .create_token(
            &h.admin,
            "ui-proj",
            TokenScope::Project(proj.id.clone()),
            None,
        )
        .await
        .unwrap();
    h.svc
        .create_kv_namespace(&h.admin, "ui_ns", Some(&proj.id))
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "ui-bkt", Some(&proj.id), false)
        .await
        .unwrap();

    let app = build_router(h.svc.clone());

    for path in ["/_admin", "/_admin/", "/_admin/assets/styles.css"] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    for path in ["/_panel", "/_panel/"] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "expected 200 for {path}");
        assert!(resp
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html"));
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap(),
            "no-cache"
        );
        let body = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("/_panel/assets/styles.css"));
        assert!(body.contains("/_panel/assets/i18n.js"));
        assert!(body.contains("/_panel/assets/app.js"));
        assert!(body.contains("user-scalable=no"));
        assert!(body.contains("maximum-scale=1.0"));
    }

    for (asset, expected_ct) in [
        ("/_panel/assets/styles.css", "text/css"),
        ("/_panel/assets/i18n.js", "application/javascript"),
        ("/_panel/assets/app.js", "application/javascript"),
        (
            "/_panel/assets/runtime_component.js",
            "application/javascript",
        ),
        ("/_panel/assets/ops_component.js", "application/javascript"),
    ] {
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(asset).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "expected 200 for {asset}");
        assert!(resp
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(expected_ct));
        assert_eq!(
            resp.headers()
                .get(header::CACHE_CONTROL)
                .unwrap()
                .to_str()
                .unwrap(),
            "no-cache"
        );
        if asset == "/_panel/assets/styles.css" {
            let css = String::from_utf8(
                resp.into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec(),
            )
            .unwrap();
            assert!(css.contains("repeat(4, minmax(0, 1fr))"));
            assert!(css.contains("touch-action: manipulation"));
            assert!(css.contains("overflow-wrap: anywhere"));
        } else if matches!(
            asset,
            "/_panel/assets/runtime_component.js" | "/_panel/assets/ops_component.js"
        ) {
            let js = String::from_utf8(
                resp.into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec(),
            )
            .unwrap();
            assert!(js.contains("stack-mobile"));
            assert!(js.contains("data-label"));
        }
    }

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_panel/assets/missing.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let (st_admin, body_admin) = json_req(
        &app,
        Method::GET,
        "/_api/v1/status",
        Some(&h.admin_token),
        None,
    )
    .await;
    assert_eq!(st_admin, StatusCode::OK);
    let admin_json: serde_json::Value = serde_json::from_slice(&body_admin).unwrap();
    assert_eq!(admin_json["status"], "ok");
    assert_eq!(admin_json["actor"], "test-admin");
    assert_eq!(admin_json["scope"], "admin");
    assert_eq!(admin_json["projects"], 1);
    assert_eq!(admin_json["buckets"], 1);
    assert_eq!(admin_json["kv_namespaces"], 1);
    assert_eq!(admin_json["queues"], 0);
    assert_eq!(admin_json["dbs"], 0);

    let (st_proj, body_proj) = json_req(
        &app,
        Method::GET,
        "/_api/v1/status",
        Some(&proj_token),
        None,
    )
    .await;
    assert_eq!(st_proj, StatusCode::OK);
    let proj_json: serde_json::Value = serde_json::from_slice(&body_proj).unwrap();
    assert_eq!(proj_json["status"], "ok");
    assert_eq!(proj_json["actor"], "ui-proj");
    assert_eq!(proj_json["scope"], format!("project:{}", proj.id));
    assert_eq!(proj_json["projects"], 1);
    assert_eq!(proj_json["buckets"], 1);
    assert_eq!(proj_json["kv_namespaces"], 1);
}

#[tokio::test]
async fn test_native_tls_resolution_and_http_redirect() {
    use ferropod::config::TlsConfig;
    use ferropod::gateway::tls::{build_https_redirect_router, resolve_tls, ResolvedTls};

    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::for_profile(
        Profile::LowMem,
        "0.0.0.0:80".parse().unwrap(),
        tmp.path().to_path_buf(),
        "test-master-key-32-bytes-minimum!!".into(),
    );

    // 1. Default: plain HTTP only
    assert_eq!(resolve_tls(&cfg, vec![]).unwrap(), ResolvedTls::Disabled);

    // 2. ACME merges explicit --acme-domain and project route domains, defaults to 0.0.0.0:443
    cfg.tls = TlsConfig {
        acme_domains: vec!["Panel.Example.com.".into()],
        acme_email: Some("ops@example.com".into()),
        ..Default::default()
    };
    let resolved = resolve_tls(
        &cfg,
        vec!["app.example.com".into(), "panel.example.com".into()],
    )
    .unwrap();
    assert_eq!(
        resolved,
        ResolvedTls::Acme {
            bind: "0.0.0.0:443".parse().unwrap(),
            domains: vec!["app.example.com".into(), "panel.example.com".into()],
            email: Some("ops@example.com".into()),
            staging: false,
            cache_dir: cfg.certs_dir(),
        }
    );

    // 3. Manual TLS with custom port
    cfg.tls = TlsConfig {
        https_bind: Some("0.0.0.0:8443".parse().unwrap()),
        cert_path: Some(tmp.path().join("cert.pem")),
        key_path: Some(tmp.path().join("key.pem")),
        ..Default::default()
    };
    assert_eq!(
        resolve_tls(&cfg, vec![]).unwrap(),
        ResolvedTls::Manual {
            bind: "0.0.0.0:8443".parse().unwrap(),
            cert_path: tmp.path().join("cert.pem"),
            key_path: tmp.path().join("key.pem"),
        }
    );

    // 4. Invalid combinations are rejected
    cfg.tls.acme_domains = vec!["conflict.example.com".into()];
    assert!(resolve_tls(&cfg, vec![]).is_err());

    cfg.tls = TlsConfig {
        cert_path: Some(tmp.path().join("cert.pem")),
        ..Default::default()
    };
    assert!(resolve_tls(&cfg, vec![]).is_err());

    // 5. HTTP -> HTTPS redirect router (port 443 and custom port 8443)
    let redirect_443 = build_https_redirect_router(443);
    let health = redirect_443
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    let r1 = redirect_443
        .oneshot(
            Request::builder()
                .uri("/_admin?tab=routes")
                .header(header::HOST, "panel.example.com:80")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        r1.headers().get(header::LOCATION).unwrap(),
        "https://panel.example.com/_admin?tab=routes"
    );

    let redirect_8443 = build_https_redirect_router(8443);
    let r2 = redirect_8443
        .oneshot(
            Request::builder()
                .uri("/hello")
                .header(header::HOST, "app.example.com:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        r2.headers().get(header::LOCATION).unwrap(),
        "https://app.example.com:8443/hello"
    );
}

#[tokio::test]
async fn test_manual_https_end_to_end_with_http_redirect() {
    use ferropod::config::TlsConfig;
    use ferropod::gateway::tls::{ensure_crypto_provider, serve_http_and_https};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};
    use tokio_rustls::TlsConnector;

    ensure_crypto_provider();
    let h = setup_harness().await;

    // Generate a self-signed certificate for localhost
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.cert.pem();
    let key_pem = cert.signing_key.serialize_pem();

    let cert_path = h._tmp.path().join("localhost.crt");
    let key_path = h._tmp.path().join("localhost.key");
    tokio::fs::write(&cert_path, &cert_pem).await.unwrap();
    tokio::fs::write(&key_path, &key_pem).await.unwrap();

    // Reserve two free loopback ports
    let http_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let https_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let mut cfg = h.svc.config().clone();
    cfg.bind_addr = format!("127.0.0.1:{http_port}").parse().unwrap();
    cfg.tls = TlsConfig {
        https_bind: Some(format!("127.0.0.1:{https_port}").parse().unwrap()),
        cert_path: Some(cert_path),
        key_path: Some(key_path),
        http_redirect_https: true,
        ..Default::default()
    };

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let svc = h.svc.clone();
    let app = build_router(svc.clone());
    let server_task = tokio::spawn(async move {
        serve_http_and_https(&cfg, &svc, app, async move {
            let _ = rx.await;
        })
        .await
    });

    // Wait briefly for both listeners to bind
    let mut http_stream = None;
    for _ in 0..30 {
        if let Ok(s) = tokio::net::TcpStream::connect(("127.0.0.1", http_port)).await {
            http_stream = Some(s);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let mut http_stream = http_stream.expect("HTTP listener failed to start");

    // 1. Verify HTTP redirects to HTTPS with custom port
    http_stream
        .write_all(
            format!(
                "GET /_panel HTTP/1.1\r\nHost: localhost:{http_port}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut http_resp = String::new();
    http_stream.read_to_string(&mut http_resp).await.unwrap();
    assert!(
        http_resp.starts_with("HTTP/1.1 308"),
        "unexpected HTTP response: {http_resp}"
    );
    assert!(
        http_resp.contains(&format!("location: https://localhost:{https_port}/_panel")),
        "missing redirect location: {http_resp}"
    );

    // 2. Verify HTTPS serves /_healthz over TLS
    let mut roots = RootCertStore::empty();
    roots.add(cert.cert.der().clone()).unwrap();
    let client_cfg = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(client_cfg));

    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", https_port))
        .await
        .unwrap();
    let mut tls_stream = connector
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .unwrap();
    tls_stream
        .write_all(b"GET /_healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut https_resp = String::new();
    tls_stream.read_to_string(&mut https_resp).await.unwrap();
    assert!(
        https_resp.starts_with("HTTP/1.1 200"),
        "unexpected HTTPS response: {https_resp}"
    );
    assert!(
        https_resp.contains(r#"{"status":"ok"}"#),
        "unexpected body: {https_resp}"
    );

    let _ = tx.send(());
    server_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn resources_can_share_a_group_without_sharing_access() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let admin = Some(h.admin_token.as_str());

    let (status, runtime) = json_req(
        &app,
        Method::POST,
        "/_api/v1/runtimes",
        admin,
        Some(serde_json::json!({"name": "grouped-worker", "group": " 分析 "})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&runtime)
    );
    let runtime: serde_json::Value = serde_json::from_slice(&runtime).unwrap();
    let id = runtime["id"].as_str().unwrap();
    assert_eq!(runtime["group"], "分析");
    let (_, legacy) = json_req(
        &app,
        Method::GET,
        &format!("/_api/v1/projects/{id}"),
        admin,
        None,
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&legacy).unwrap(),
        runtime
    );

    for (kind, names) in [
        ("runtimes", ["second-worker", "third-worker"]),
        ("dbs", ["analytics-db", "events-db"]),
        ("kv", ["analytics-kv", "events-kv"]),
        ("queues", ["analytics-queue", "events-queue"]),
        ("buckets", ["analytics-assets", "events-assets"]),
    ] {
        let path = format!("/_api/v1/{kind}");
        for name in names {
            let (status, body) = json_req(
                &app,
                Method::POST,
                &path,
                admin,
                Some(serde_json::json!({"name": name, "group": "分析"})),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::CREATED,
                "{path}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        let (status, body) = json_req(&app, Method::GET, &path, admin, None).await;
        assert_eq!(status, StatusCode::OK);
        let resources: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        for name in names {
            assert!(resources
                .iter()
                .any(|r| r["name"] == name && r["group"] == "分析"));
        }
    }

    let (_, admin_groups) =
        json_req(&app, Method::GET, "/_api/v1/resource-groups", admin, None).await;
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&admin_groups).unwrap(),
        ["分析"]
    );

    let other = h
        .svc
        .create_project(&h.admin, "ungrouped-worker", None)
        .await
        .unwrap();
    let (_, scoped_token) = h
        .svc
        .create_token(&h.admin, "group-scope", TokenScope::Project(other.id), None)
        .await
        .unwrap();
    let (status, scoped_groups) = json_req(
        &app,
        Method::GET,
        "/_api/v1/resource-groups",
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(serde_json::from_slice::<Vec<String>>(&scoped_groups)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn changing_runtime_env_creates_a_ready_version_and_keeps_secrets_separate() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());
    let runtime = h
        .svc
        .create_project(&h.admin, "settings-worker", None)
        .await
        .unwrap();
    let unrelated = h
        .svc
        .create_project(&h.admin, "unrelated-worker", None)
        .await
        .unwrap();
    let path = format!("/_api/v1/runtimes/{}", runtime.id);

    h.svc
        .create_kv_namespace(&h.admin, "state", Some(&runtime.id))
        .await
        .unwrap();
    let (status, body) = json_req(
        &app,
        Method::POST,
        &format!("{path}/deployments"),
        token,
        Some(serde_json::json!({
            "bindings": { "env": { "MODE": "prod" }, "kv": { "STATE": "state" } },
            "crons": ["0 * * * *"],
            "target_env": "prod"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let original: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let original_id = original["id"].as_str().unwrap();

    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!("{path}/secrets"),
        token,
        Some(serde_json::json!({"key": "MODE", "value": "encrypted-value"})),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let env = serde_json::json!({"env": {"MODE": "preview", "MULTILINE": "one\ntwo"}});
    let (status, body) = json_req(
        &app,
        Method::POST,
        &format!("{path}/deployments/{original_id}/env"),
        token,
        Some(env.clone()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let revised: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_ne!(revised["id"], original["id"]);
    assert_eq!(revised["status"], "ready");
    assert_eq!(revised["bindings"]["env"], env["env"]);
    assert_eq!(revised["bindings"]["kv"], original["bindings"]["kv"]);
    assert_eq!(revised["artifact_path"], original["artifact_path"]);
    assert_eq!(revised["crons"], original["crons"]);

    let current = h.svc.get_project(&h.admin, &runtime.id).await.unwrap();
    assert_eq!(current.active_deployment_id.as_deref(), Some(original_id));
    assert_eq!(
        h.svc
            .list_deployments(&h.admin, &runtime.id)
            .await
            .unwrap()
            .into_iter()
            .find(|d| d.id == original_id)
            .unwrap()
            .bindings
            .env["MODE"],
        "prod"
    );
    let (status, keys) = json_req(&app, Method::GET, &format!("{path}/secrets"), token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Vec<String>>(&keys).unwrap(),
        ["MODE"]
    );

    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!(
            "/_api/v1/runtimes/{}/deployments/{original_id}/env",
            unrelated.id
        ),
        token,
        Some(env),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scoped_queue_creation_cannot_replace_another_project_queue() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let a = h
        .svc
        .create_project(&h.admin, "queue-owner-a", None)
        .await
        .unwrap();
    let b = h
        .svc
        .create_project(&h.admin, "queue-owner-b", None)
        .await
        .unwrap();
    let (_, token_a) = h
        .svc
        .create_token(&h.admin, "queue-a", TokenScope::Project(a.id.clone()), None)
        .await
        .unwrap();
    let (_, token_b) = h
        .svc
        .create_token(&h.admin, "queue-b", TokenScope::Project(b.id.clone()), None)
        .await
        .unwrap();
    let payload = serde_json::json!({"name": "shared-name"});
    let (status, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues",
        Some(&token_a),
        Some(payload.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues",
        Some(&token_b),
        Some(payload),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let queues = h.svc.queue_list(&h.admin).await.unwrap();
    assert_eq!(
        queues
            .iter()
            .find(|q| q.name == "shared-name")
            .unwrap()
            .project_id
            .as_deref(),
        Some(a.id.as_str())
    );
}

#[tokio::test]
async fn search_considers_all_documents_and_keeps_best_matches() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());
    let (status, bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/search",
        token,
        Some(serde_json::json!({"name": "large-search"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let index: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let path = format!("/_api/v1/search/{}", index["id"].as_str().unwrap());
    for i in 0..505 {
        let title = if i == 0 { "unique-marker" } else { "other" };
        let (status, _) = json_req(
            &app,
            Method::POST,
            &format!("{path}/docs"),
            token,
            Some(serde_json::json!({"id": format!("doc-{i}"), "title": title})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let mut ids = std::collections::HashSet::new();
    for offset in (0..505).step_by(100) {
        let (status, page) = json_req(
            &app,
            Method::GET,
            &format!("{path}/docs?limit=100&offset={offset}"),
            token,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let docs: Vec<serde_json::Value> = serde_json::from_slice(&page).unwrap();
        assert!(docs.len() <= 100);
        for doc in docs {
            assert!(ids.insert(doc["id"].as_str().unwrap().to_string()));
        }
    }
    assert_eq!(ids.len(), 505);
    let (status, bytes) = json_req(
        &app,
        Method::POST,
        &format!("{path}/query"),
        token,
        Some(serde_json::json!({"query": "unique-marker", "limit": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(result["total"], 1);
    assert_eq!(result["hits"][0]["doc"]["id"], "doc-0");
}

#[tokio::test]
async fn database_search_scans_beyond_five_hundred_rows() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());
    let (status, body) = json_req(
        &app,
        Method::POST,
        "/_api/v1/dbs",
        token,
        Some(serde_json::json!({"name":"search-large-db"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let db: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let path = format!("/_api/v1/dbs/{}", db["id"].as_str().unwrap());
    let mut statements = vec![
        serde_json::json!({"sql":"CREATE TABLE entries (id INTEGER PRIMARY KEY, title TEXT)", "params":[]}),
    ];
    for i in 0..505 {
        let text = if i == 504 {
            "last-record-marker"
        } else {
            "other"
        };
        statements.push(serde_json::json!({"sql":"INSERT INTO entries (id, title) VALUES (?1, ?2)", "params":[i, text]}));
    }
    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!("{path}/execute"),
        token,
        Some(serde_json::json!({"statements":statements})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!("{path}/fts"),
        token,
        Some(serde_json::json!({"name":"entries_fts","table":"entries","columns":["title"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, body) = json_req(
        &app,
        Method::POST,
        &format!("{path}/fts/entries_fts/query"),
        token,
        Some(serde_json::json!({"query":"last-record-marker"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let result: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["rows"][0]["row"]["id"], 504);
    assert_eq!(result["index"]["rows_indexed"], 505);
}

async fn setup_harness_with(mutate_cfg: impl FnOnce(&mut Config)) -> TestHarness {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = Config::for_profile(
        Profile::LowMem,
        "127.0.0.1:0".parse().unwrap(),
        tmp.path().to_path_buf(),
        "integration-test-master-key-32bytes".to_string(),
    );
    cfg.blob_gc_grace_secs = 0;
    mutate_cfg(&mut cfg);

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
            kv: kv.clone(),
            queue: queue.clone(),
            objects: blobs.clone(),
            db: user_dbs.clone(),
            logs: logs.clone(),
        },
    )
    .await
    .unwrap();

    let admin = Caller::system("test");
    let (_meta, admin_token) = svc
        .create_token(&admin, "test-admin", TokenScope::Admin, None)
        .await
        .unwrap();

    TestHarness {
        _tmp: tmp,
        svc,
        kv,
        queue,
        user_dbs,
        blobs,
        logs,
        admin,
        admin_token,
    }
}

#[tokio::test]
async fn local_api_forwarding_security_and_http2_authority() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let hit_count = Arc::new(AtomicUsize::new(0));
    let hit_count_srv = hit_count.clone();
    let upstream_app = Router::new().fallback(any(move |req: AxumRequest| {
        let hits = hit_count_srv.clone();
        async move {
            hits.fetch_add(1, Ordering::SeqCst);
            let method = req.method().to_string();
            let path_query = req
                .uri()
                .path_and_query()
                .map(|pq| pq.as_str().to_string())
                .unwrap_or_default();
            let hdr = |name: &str| {
                req.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            Json(serde_json::json!({
                "method": method,
                "path_query": path_query,
                "host": hdr("host"),
                "authorization": hdr("authorization"),
                "cookie": hdr("cookie"),
                "x_request_id": hdr("x-request-id"),
                "x_forwarded_for": hdr("x-forwarded-for"),
                "x_custom_hop": hdr("x-custom-hop"),
            }))
        }
    }));
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, upstream_app).await;
    });

    // Config validation unit assertions
    for reserved in [
        "/_api",
        "/_api/v1",
        "/_blob",
        "/_panel",
        "/_internal",
        "/_healthz",
        "/",
    ] {
        assert!(
            LocalApiConfig::validated(
                "site.example.test",
                Some(reserved),
                upstream_addr,
                &["127.0.0.1:8080".parse().unwrap()],
            )
            .is_err(),
            "expected reserved prefix {reserved} to be rejected"
        );
    }
    assert!(LocalApiConfig::validated(
        "site.example.test:8080",
        Some("/api"),
        upstream_addr,
        &["127.0.0.1:8080".parse().unwrap()],
    )
    .is_err());
    assert!(LocalApiConfig::validated(
        "site.example.test",
        Some("/api"),
        "127.0.0.1:443".parse().unwrap(),
        &[
            "127.0.0.1:8080".parse().unwrap(),
            "0.0.0.0:443".parse().unwrap(),
        ],
    )
    .is_err());

    let ipv6_cfg = LocalApiConfig::validated(
        "[::1]",
        Some("/api/v1/"),
        upstream_addr,
        &["127.0.0.1:8080".parse().unwrap()],
    )
    .unwrap();
    assert_eq!(ipv6_cfg.host, "[::1]");
    assert_eq!(ipv6_cfg.prefix, "/api/v1");

    let valid_cfg = LocalApiConfig::validated(
        "API.Example.TEST.",
        Some("/api"),
        upstream_addr,
        &["127.0.0.1:8080".parse().unwrap()],
    )
    .unwrap();
    assert_eq!(valid_cfg.host, "api.example.test");

    let h = setup_harness_with(|cfg| {
        cfg.local_api = Some(valid_cfg);
        cfg.max_blob_upload_bytes = 64;
    })
    .await;
    let proj = h
        .svc
        .create_project(&h.admin, "local-api-site", None)
        .await
        .unwrap();
    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                spa_fallback: true,
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "api.example.test", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();
    let app = build_router(h.svc.clone());

    // 1. Query string %2F works while protected headers cannot be stripped via Connection
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/items?next=%2Fdashboard")
                .header(header::HOST, "API.EXAMPLE.TEST:8080")
                .header(header::AUTHORIZATION, "Bearer user-secret")
                .header(header::COOKIE, "sid=abc")
                .header("x-request-id", "req-keep-1")
                .header("x-forwarded-for", "1.2.3.4")
                .header("x-custom-hop", "drop-me")
                .header(
                    header::CONNECTION,
                    "x-custom-hop, host, authorization, cookie, x-request-id",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let echo: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(echo["path_query"], "/api/items?next=%2Fdashboard");
    assert_eq!(echo["host"], "API.EXAMPLE.TEST:8080");
    assert_eq!(echo["authorization"], "Bearer user-secret");
    assert_eq!(echo["cookie"], "sid=abc");
    assert_eq!(echo["x_request_id"], "req-keep-1");
    assert!(echo["x_forwarded_for"].is_null());
    assert!(echo["x_custom_hop"].is_null());
    assert_eq!(hit_count.load(Ordering::SeqCst), 1);

    // 2. HTTP/2-style request (no Host header, authority on URI) routes & synthesizes Host
    let h2_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("https://api.example.test:8443/api/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(h2_resp.status(), StatusCode::OK);
    let h2_echo: serde_json::Value =
        serde_json::from_slice(&h2_resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(h2_echo["host"], "api.example.test:8443");
    assert_eq!(h2_echo["path_query"], "/api/ping");
    assert_eq!(hit_count.load(Ordering::SeqCst), 2);

    // 3. Traversal/smuggling paths rejected with 400 and never forwarded
    for bad_path in [
        "/api/../_internal/cron",
        "/api/%2e%2e/secret",
        "/api/foo%2fbar",
        "/api/foo%5cbar",
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(bad_path)
                    .header(header::HOST, "api.example.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "path {bad_path} should fail with 400"
        );
    }

    // 4. CONNECT and Upgrade rejected with 405
    let connect_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::CONNECT)
                .uri("/api/tunnel")
                .header(header::HOST, "api.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(connect_resp.status(), StatusCode::METHOD_NOT_ALLOWED);

    let upgrade_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/ws")
                .header(header::HOST, "api.example.test")
                .header(header::UPGRADE, "websocket")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(upgrade_resp.status(), StatusCode::METHOD_NOT_ALLOWED);

    // 5. Oversized request body via Content-Length AND streaming chunked body return 413
    let cl_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/upload")
                .header(header::HOST, "api.example.test")
                .header(header::CONTENT_LENGTH, "128")
                .body(Body::from(vec![b'x'; 128]))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cl_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let stream = futures_util::stream::iter(vec![
        Ok::<_, std::io::Error>(Bytes::from(vec![b'a'; 40])),
        Ok::<_, std::io::Error>(Bytes::from(vec![b'b'; 40])),
    ]);
    let stream_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/upload-stream")
                .header(header::HOST, "api.example.test")
                .body(Body::from_stream(stream))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(hit_count.load(Ordering::SeqCst), 2);

    // 6. Bracketed IPv6 host normalization + :authority lookup
    let h_v6 = setup_harness_with(|cfg| {
        cfg.local_api = Some(ipv6_cfg);
    })
    .await;
    let proj_v6 = h_v6
        .svc
        .create_project(&h_v6.admin, "v6-site", None)
        .await
        .unwrap();
    h_v6.svc
        .create_deployment(
            &h_v6.admin,
            &proj_v6.id,
            CreateDeploymentInput {
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h_v6.svc
        .upsert_route(&h_v6.admin, "[::1]", &proj_v6.id, TargetEnv::Prod)
        .await
        .unwrap();
    let app_v6 = build_router(h_v6.svc.clone());
    let v6_resp = app_v6
        .oneshot(
            Request::builder()
                .uri("/api/v1/ok")
                .header(header::HOST, "[::1]:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(v6_resp.status(), StatusCode::OK);

    server.abort();
}

#[tokio::test]
async fn admin_cannot_bind_another_apps_private_bucket_as_assets() {
    let h = setup_harness().await;
    let app_a = h.svc.create_project(&h.admin, "app-a", None).await.unwrap();
    let app_b = h.svc.create_project(&h.admin, "app-b", None).await.unwrap();
    h.svc
        .create_bucket(&h.admin, "a-private", Some(&app_a.id), false)
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "shared-global", None, false)
        .await
        .unwrap();

    // Admin cannot bind App A's bucket onto App B
    let err = h
        .svc
        .create_deployment(
            &h.admin,
            &app_b.id,
            CreateDeploymentInput {
                assets_bucket: Some("a-private".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ferropod::core::model::FerroError::Forbidden(_)
    ));

    // Admin can bind App A's bucket onto App A and global bucket onto App B
    h.svc
        .create_deployment(
            &h.admin,
            &app_a.id,
            CreateDeploymentInput {
                assets_bucket: Some("a-private".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .create_deployment(
            &h.admin,
            &app_b.id,
            CreateDeploymentInput {
                assets_bucket: Some("shared-global".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn database_fts_ignores_unindexed_title_column() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());
    let (status, body) = json_req(
        &app,
        Method::POST,
        "/_api/v1/dbs",
        token,
        Some(serde_json::json!({"name": "fts-column-scope"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let db: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let path = format!("/_api/v1/dbs/{}", db["id"].as_str().unwrap());

    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!("{path}/execute"),
        token,
        Some(serde_json::json!({
            "statements": [
                {"sql": "CREATE TABLE docs (id INTEGER PRIMARY KEY, title TEXT, body TEXT)", "params": []},
                {"sql": "INSERT INTO docs (id, title, body) VALUES (1, 'confidential_title_secret', 'public body text')", "params": []}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = json_req(
        &app,
        Method::POST,
        &format!("{path}/fts"),
        token,
        Some(serde_json::json!({"name": "body_only", "table": "docs", "columns": ["body"]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, secret_res) = json_req(
        &app,
        Method::POST,
        &format!("{path}/fts/body_only/query"),
        token,
        Some(serde_json::json!({"query": "confidential_title_secret"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let secret_json: serde_json::Value = serde_json::from_slice(&secret_res).unwrap();
    assert_eq!(secret_json["rows"].as_array().unwrap().len(), 0);

    let (status, body_res) = json_req(
        &app,
        Method::POST,
        &format!("{path}/fts/body_only/query"),
        token,
        Some(serde_json::json!({"query": "public"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let body_json: serde_json::Value = serde_json::from_slice(&body_res).unwrap();
    assert_eq!(body_json["rows"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn search_docs_default_paginated_and_updated_desc_sort() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());

    let (status, created) = json_req(
        &app,
        Method::POST,
        "/_api/v1/search",
        token,
        Some(serde_json::json!({"name": "paginated-idx"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let idx: serde_json::Value = serde_json::from_slice(&created).unwrap();
    let path = format!("/_api/v1/search/{}", idx["id"].as_str().unwrap());

    for i in 0..120 {
        if i >= 115 {
            tokio::time::sleep(std::time::Duration::from_millis(3)).await;
        }
        let (status, _) = json_req(
            &app,
            Method::POST,
            &format!("{path}/docs"),
            token,
            Some(serde_json::json!({"id": format!("d-{i:03}"), "body": "common-keyword"})),
        )
        .await;
        assert!(status.is_success());
    }

    // Unparameterized GET /docs is capped at 100 by default
    let (status, default_docs) =
        json_req(&app, Method::GET, &format!("{path}/docs"), token, None).await;
    assert_eq!(status, StatusCode::OK);
    let default_arr: Vec<serde_json::Value> = serde_json::from_slice(&default_docs).unwrap();
    assert_eq!(default_arr.len(), 100);

    // updated_desc returns newest 5 matches in descending updated_at order
    let (status, res_bytes) = json_req(
        &app,
        Method::POST,
        &format!("{path}/query"),
        token,
        Some(serde_json::json!({"query": "common-keyword", "sort": "updated_desc", "limit": 5})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let res: serde_json::Value = serde_json::from_slice(&res_bytes).unwrap();
    assert_eq!(res["total"], 120);
    let hits = res["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 5);
    assert_eq!(hits[0]["doc"]["id"], "d-119");
    assert_eq!(hits[1]["doc"]["id"], "d-118");
}

#[tokio::test]
async fn search_mock_asset_removed() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());

    let mock_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_panel/assets/search_mock.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(mock_resp.status(), StatusCode::NOT_FOUND);

    let comp_resp = app
        .oneshot(
            Request::builder()
                .uri("/_panel/assets/search_components.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(comp_resp.status(), StatusCode::OK);
    let js = String::from_utf8(
        comp_resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(!js.contains("FerroSearchMock"));
}

#[tokio::test]
async fn cron_schedule_validation_rejects_invalid_expressions() {
    let h = setup_harness().await;
    let proj = h
        .svc
        .create_project(&h.admin, "cron-val-app", None)
        .await
        .unwrap();

    for bad in [
        "",
        "* * *",
        "99 * * * *",
        "@every 0s",
        "@every 999999999999999999h",
        "not-a-cron",
    ] {
        let err = h
            .svc
            .create_cron_job(&h.admin, "bad-cron", Some(&proj.id), bad, "/_internal/cron")
            .await
            .unwrap_err();
        assert!(
            matches!(err, ferropod::core::model::FerroError::InvalidArgument(_)),
            "expected InvalidArgument for '{bad}', got {err:?}"
        );
    }

    let valid = h
        .svc
        .create_cron_job(
            &h.admin,
            "good-cron",
            Some(&proj.id),
            "@daily",
            "/_internal/cron",
        )
        .await
        .unwrap();

    let upd_err = h
        .svc
        .update_cron_job(&h.admin, &valid.id, None, Some("60 * * * *"), None)
        .await
        .unwrap_err();
    assert!(matches!(
        upd_err,
        ferropod::core::model::FerroError::InvalidArgument(_)
    ));

    let updated = h
        .svc
        .update_cron_job(&h.admin, &valid.id, None, Some("@every 30s"), None)
        .await
        .unwrap();
    assert_eq!(updated.schedule, "@every 30s");
}

#[tokio::test]
async fn security_hardening_regression_checks() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let token = Some(h.admin_token.as_str());

    // 1. User DB execute rejects ATTACH, DETACH, PRAGMA
    let db_id = h
        .svc
        .create_user_db(&h.admin, "sec-db", None)
        .await
        .unwrap();
    for dangerous in [
        "ATTACH DATABASE '/tmp/pwn.db' AS pwn",
        "DETACH DATABASE main",
        "PRAGMA writable_schema = ON",
        "CREATE TABLE ok (id INT); ATTACH '/tmp/pwn.db' AS pwn",
    ] {
        let (st, _) = json_req(
            &app,
            Method::POST,
            &format!("/_api/v1/dbs/{db_id}/execute"),
            token,
            Some(serde_json::json!({ "statements": [{ "sql": dangerous }] })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "must reject SQL: {dangerous}");
    }

    // 2. Project-scoped token cannot attach another project's worker as queue consumer
    let proj_a = h.svc.create_project(&h.admin, "sec-a", None).await.unwrap();
    let proj_b = h.svc.create_project(&h.admin, "sec-b", None).await.unwrap();
    let (_, tok_a) = h
        .svc
        .create_token(
            &h.admin,
            "tok-sec-a",
            TokenScope::Project(proj_a.id.clone()),
            None,
        )
        .await
        .unwrap();
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues",
        Some(&tok_a),
        Some(serde_json::json!({
            "name": "hijack-q",
            "project_id": proj_a.id,
            "consumer_project_id": proj_b.id,
        })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 3. Blob XSS sandbox headers, weak ETag 304, bad bucket/key, presigned URL with space
    let (bad_bkt, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        token,
        Some(serde_json::json!({ "name": ".." })),
    )
    .await;
    assert_eq!(bad_bkt, StatusCode::BAD_REQUEST);

    h.svc
        .create_bucket(&h.admin, "sec-blobs", None, true)
        .await
        .unwrap();
    let put_space = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/sec-blobs/docs/my%20file.html")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "text/html")
                .body(Body::from("<script>alert(1)</script>"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_space.status(), StatusCode::CREATED);

    let (st_presign, presign_body) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets/sec-blobs/presign",
        token,
        Some(serde_json::json!({ "key": "docs/my file.html", "op": "get", "expires_secs": 60 })),
    )
    .await;
    assert_eq!(st_presign, StatusCode::OK);
    let presign_val: serde_json::Value = serde_json::from_slice(&presign_body).unwrap();
    let presign_url = presign_val["url"].as_str().unwrap();
    assert!(presign_url.contains("docs/my%20file.html?op=get"));

    let blob_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(presign_url)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(blob_get.status(), StatusCode::OK);
    assert_eq!(
        blob_get
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let csp = blob_get
        .headers()
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(csp.contains("sandbox"));
    let etag = blob_get
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let weak_etag_req = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_blob/sec-blobs/docs/my%20file.html")
                .header(header::IF_NONE_MATCH, format!("\"other\", W/{etag}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(weak_etag_req.status(), StatusCode::NOT_MODIFIED);

    let bad_key_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/sec-blobs/a/../b")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .body(Body::from("x"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_key_resp.status(), StatusCode::BAD_REQUEST);

    // 4. Overflow bounds: Queue u64::MAX delay, KV u64::MAX ttl, invalid token expiry days
    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                group: None,
                name: "sec-q".into(),
                project_id: None,
                consumer_project_id: None,
                description: None,
                max_concurrency: 1,
                rate_limit_per_sec: None,
                batch_size: 1,
                lease_secs: 30,
                max_attempts: 3,
                retry_backoff_secs: 5,
                dlq_retention_secs: None,
                ephemeral: false,
                max_pending_jobs: None,
                created_at: Utc::now().timestamp(),
            },
        )
        .await
        .unwrap();
    let job_id = h
        .svc
        .queue_enqueue(
            &h.admin,
            "sec-q",
            Bytes::from_static(b"\"ok\""),
            EnqueueOpts {
                delay_secs: Some(u64::MAX),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(job_id > 0);

    h.svc
        .create_kv_namespace(&h.admin, "sec-kv", None)
        .await
        .unwrap();
    h.svc
        .kv_put(
            &h.admin,
            "sec-kv",
            "kmax",
            Bytes::from_static(b"v"),
            None,
            Some(u64::MAX),
            false,
        )
        .await
        .unwrap();
    assert!(h
        .svc
        .kv_get(&h.admin, "sec-kv", "kmax")
        .await
        .unwrap()
        .is_some());

    for bad_days in [0i64, -5, 100_000] {
        let (st_tok, _) = json_req(
            &app,
            Method::POST,
            "/_api/v1/tokens",
            token,
            Some(serde_json::json!({
                "name": "bad-exp",
                "scope": "readonly",
                "expires_in_days": bad_days,
            })),
        )
        .await;
        assert_eq!(st_tok, StatusCode::BAD_REQUEST);
    }

    // 5. HTTPS redirect rejects Host injection / userinfo while accepting valid hosts
    let redir_app = ferropod::gateway::tls::build_https_redirect_router(8443);
    let evil_redir = redir_app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/login")
                .header(header::HOST, "trusted@evil.com:80")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(evil_redir.status(), StatusCode::BAD_REQUEST);

    let ok_redir = redir_app
        .oneshot(
            Request::builder()
                .uri("/login?a=1")
                .header(header::HOST, "example.com:8080")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_redir.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        ok_redir.headers().get(header::LOCATION).unwrap(),
        "https://example.com:8443/login?a=1"
    );
}

#[tokio::test]
async fn extended_creation_options_mandatory_kv_ttl_and_patch_endpoints() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());
    let parse_json = |b: Bytes| -> serde_json::Value { serde_json::from_slice(&b).unwrap() };

    // 1. App / Runtime creation with extended options + PATCH
    let (st, proj_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/runtimes",
        tok,
        Some(serde_json::json!({
            "name": "pro-app",
            "group": "core-services",
            "description": "Production API worker",
            "health_check_path": "/readyz",
            "max_concurrency": 128,
            "timeout_ms": 15000,
            "wasm_max_memory_mb": 48,
            "allow_private_outbound": true
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let proj_json = parse_json(proj_bytes);
    let proj_id = proj_json["id"].as_str().unwrap().to_string();
    assert_eq!(proj_json["max_concurrency"], 128);
    assert_eq!(proj_json["timeout_ms"], 15000);
    assert_eq!(proj_json["wasm_max_memory_mb"], 48);
    assert_eq!(proj_json["allow_private_outbound"], true);
    assert_eq!(proj_json["description"], "Production API worker");

    let (st, proj_upd_bytes) = json_req(
        &app,
        Method::PATCH,
        &format!("/_api/v1/runtimes/{proj_id}"),
        tok,
        Some(serde_json::json!({
            "max_concurrency": 256,
            "timeout_ms": 5000,
            "description": "Updated worker"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let proj_upd = parse_json(proj_upd_bytes);
    assert_eq!(proj_upd["max_concurrency"], 256);
    assert_eq!(proj_upd["timeout_ms"], 5000);
    assert_eq!(proj_upd["description"], "Updated worker");

    // 2. KV namespace: mandatory expiration (default_ttl_secs >= 1), quotas, and PATCH
    let (st_zero_ns, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/kv",
        tok,
        Some(serde_json::json!({
            "name": "bad_ns",
            "default_ttl_secs": 0
        })),
    )
    .await;
    assert_eq!(st_zero_ns, StatusCode::BAD_REQUEST);

    let (st_kv, kv_ns_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/kv",
        tok,
        Some(serde_json::json!({
            "name": "sessions_ns",
            "group": "cache",
            "project_id": proj_id,
            "description": "Session cache with mandatory TTL",
            "default_ttl_secs": 3600,
            "default_ephemeral": false,
            "max_value_bytes": 16,
            "max_keys": 2
        })),
    )
    .await;
    assert_eq!(st_kv, StatusCode::CREATED);
    let kv_ns = parse_json(kv_ns_bytes);
    assert_eq!(kv_ns["default_ttl_secs"], 3600);
    assert_eq!(kv_ns["max_value_bytes"], 16);
    assert_eq!(kv_ns["max_keys"], 2);

    // Reject ttl_secs = 0 on PUT
    let (st_put_zero, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/sessions_ns/k1",
        tok,
        Some(serde_json::json!({
            "value": "ok",
            "ttl_secs": 0
        })),
    )
    .await;
    assert_eq!(st_put_zero, StatusCode::BAD_REQUEST);

    // Omitting ttl_secs automatically applies namespace default_ttl_secs (never None!)
    let (st_put1, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/sessions_ns/k1",
        tok,
        Some(serde_json::json!({
            "value": "val1"
        })),
    )
    .await;
    assert_eq!(st_put1, StatusCode::NO_CONTENT);

    let (st_get1, got1_bytes) =
        json_req(&app, Method::GET, "/_api/v1/kv/sessions_ns/k1", tok, None).await;
    assert_eq!(st_get1, StatusCode::OK);
    let got1 = parse_json(got1_bytes);
    assert!(
        got1["expires_at"].as_i64().unwrap() > Utc::now().timestamp(),
        "KV entry must always have a positive expires_at timestamp"
    );

    // Value exceeding max_value_bytes (16) is rejected
    let (st_too_big, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/sessions_ns/k_big",
        tok,
        Some(serde_json::json!({
            "value": "01234567890123456789"
        })),
    )
    .await;
    assert_eq!(st_too_big, StatusCode::BAD_REQUEST);

    // Insert 2nd key (within max_keys = 2), then 3rd key fails max_keys quota
    let (st_put2, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/sessions_ns/k2",
        tok,
        Some(serde_json::json!({ "value": "val2" })),
    )
    .await;
    assert_eq!(st_put2, StatusCode::NO_CONTENT);

    let (st_put3, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/sessions_ns/k3",
        tok,
        Some(serde_json::json!({ "value": "val3" })),
    )
    .await;
    assert_eq!(st_put3, StatusCode::BAD_REQUEST);

    // PATCH KV namespace raises max_keys and updates default_ttl_secs
    let (st_kv_patch, kv_patched_bytes) = json_req(
        &app,
        Method::PATCH,
        "/_api/v1/kv/sessions_ns",
        tok,
        Some(serde_json::json!({
            "default_ttl_secs": 7200,
            "max_keys": 10
        })),
    )
    .await;
    assert_eq!(st_kv_patch, StatusCode::OK);
    let kv_patched = parse_json(kv_patched_bytes);
    assert_eq!(kv_patched["default_ttl_secs"], 7200);
    assert_eq!(kv_patched["max_keys"], 10);

    // 3. Database creation with init_sql, max_rows, read_only, and PATCH
    let (st_db, db_meta_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/dbs",
        tok,
        Some(serde_json::json!({
            "name": "configured_db",
            "group": "storage",
            "description": "Pre-initialized DB",
            "init_sql": "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL); INSERT INTO items (name) VALUES ('seed');",
            "max_rows": 2,
            "foreign_keys": true,
            "read_only": false
        })),
    )
    .await;
    assert_eq!(st_db, StatusCode::CREATED);
    let db_meta = parse_json(db_meta_bytes);
    let db_id = db_meta["id"].as_str().unwrap().to_string();
    assert_eq!(db_meta["max_rows"], 2);

    // Insert 2nd row succeeds; 3rd row exceeds max_rows (2)
    let (st_ins2, ins2_bytes) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/execute"),
        tok,
        Some(serde_json::json!({
            "statements": [{ "sql": "INSERT INTO items (name) VALUES ('second')", "params": [] }]
        })),
    )
    .await;
    assert_eq!(
        st_ins2,
        StatusCode::OK,
        "ins2 body: {}",
        String::from_utf8_lossy(&ins2_bytes)
    );

    let (st_ins3, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/execute"),
        tok,
        Some(serde_json::json!({
            "statements": [{ "sql": "INSERT INTO items (name) VALUES ('third')", "params": [] }]
        })),
    )
    .await;
    assert_eq!(st_ins3, StatusCode::BAD_REQUEST);

    // PATCH DB to read_only = true and max_rows = 100
    let (st_db_patch, db_patched_bytes) = json_req(
        &app,
        Method::PATCH,
        &format!("/_api/v1/dbs/{db_id}"),
        tok,
        Some(serde_json::json!({
            "max_rows": 100,
            "read_only": true
        })),
    )
    .await;
    assert_eq!(st_db_patch, StatusCode::OK);
    let db_patched = parse_json(db_patched_bytes);
    assert_eq!(db_patched["read_only"], true);

    let (st_ro_fail, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/execute"),
        tok,
        Some(serde_json::json!({
            "statements": [{ "sql": "INSERT INTO items (name) VALUES ('blocked')", "params": [] }]
        })),
    )
    .await;
    assert_eq!(st_ro_fail, StatusCode::FORBIDDEN);

    // 4. Bucket creation with allowed_content_types, cache_control, cors_origins
    let (st_bkt, bkt_meta_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        tok,
        Some(serde_json::json!({
            "name": "images-only",
            "public_read": true,
            "allowed_content_types": "image/*,text/plain",
            "cache_control": "public, max-age=3600",
            "cors_origins": "*"
        })),
    )
    .await;
    assert_eq!(st_bkt, StatusCode::CREATED);
    let bkt_meta = parse_json(bkt_meta_bytes);
    assert_eq!(bkt_meta["cache_control"], "public, max-age=3600");

    // Disallowed MIME type is rejected on PUT /_blob/images-only/app.json
    let bad_mime_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/images-only/app.json")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad_mime_resp.status(), StatusCode::BAD_REQUEST);

    // Allowed MIME type succeeds and returns custom Cache-Control & CORS headers on GET
    let ok_mime_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/images-only/readme.txt")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("hello"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok_mime_resp.status(), StatusCode::CREATED);

    let get_blob_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/_blob/images-only/readme.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_blob_resp.status(), StatusCode::OK);
    assert_eq!(
        get_blob_resp.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    assert_eq!(
        get_blob_resp
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "*"
    );

    // 5. Search index creation with group, writer_budget_mb, facet_fields + PATCH
    let (st_si, si_meta_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/search",
        tok,
        Some(serde_json::json!({
            "name": "catalog_idx",
            "group": "search-group",
            "description": "Product catalog search",
            "tokenizer": "unicode61",
            "writer_budget_mb": 24,
            "facet_fields": ["brand", "region"]
        })),
    )
    .await;
    assert_eq!(st_si, StatusCode::CREATED);
    let si_meta = parse_json(si_meta_bytes);
    let si_id = si_meta["id"].as_str().unwrap().to_string();
    assert_eq!(si_meta["group"], "search-group");
    assert_eq!(si_meta["writer_budget_mb"], 24);

    let (st_groups, groups_bytes) =
        json_req(&app, Method::GET, "/_api/v1/resource-groups", tok, None).await;
    assert_eq!(st_groups, StatusCode::OK);
    let groups_json = parse_json(groups_bytes);
    let groups_arr = groups_json.as_array().unwrap();
    assert!(groups_arr
        .iter()
        .any(|g| g.as_str() == Some("search-group")));

    let (st_si_patch, si_patched_bytes) = json_req(
        &app,
        Method::PATCH,
        &format!("/_api/v1/search/{si_id}"),
        tok,
        Some(serde_json::json!({
            "description": "Updated catalog search",
            "writer_budget_mb": 32
        })),
    )
    .await;
    assert_eq!(st_si_patch, StatusCode::OK);
    let si_patched = parse_json(si_patched_bytes);
    assert_eq!(si_patched["writer_budget_mb"], 32);
    assert_eq!(si_patched["description"], "Updated catalog search");

    // 6. Ephemeral in-memory Queue with max_pending_jobs backpressure cap
    let (st_eq, eq_meta_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues",
        tok,
        Some(serde_json::json!({
            "name": "mem-events",
            "group": "realtime",
            "ephemeral": true,
            "max_pending_jobs": 2
        })),
    )
    .await;
    assert_eq!(st_eq, StatusCode::CREATED);
    let eq_meta = parse_json(eq_meta_bytes);
    assert_eq!(eq_meta["ephemeral"], true);
    assert_eq!(eq_meta["max_pending_jobs"], 2);

    let (st_enq1, enq1_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/mem-events/enqueue",
        tok,
        Some(serde_json::json!({
            "payload": { "ev": 1 },
            "dedup_key": "ev-1"
        })),
    )
    .await;
    assert_eq!(st_enq1, StatusCode::ACCEPTED);
    let job1_id = parse_json(enq1_bytes)["job_id"].as_i64().unwrap();
    assert!(job1_id >= 1_000_000_000);

    // Dedup key returns existing in-memory job ID without consuming extra slot
    let (st_enq_dup, enq_dup_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/mem-events/enqueue",
        tok,
        Some(serde_json::json!({
            "payload": { "ev": 1 },
            "dedup_key": "ev-1"
        })),
    )
    .await;
    assert_eq!(st_enq_dup, StatusCode::ACCEPTED);
    assert_eq!(
        parse_json(enq_dup_bytes)["job_id"].as_i64().unwrap(),
        job1_id
    );

    let (st_enq2, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/mem-events/enqueue",
        tok,
        Some(serde_json::json!({ "payload": { "ev": 2 } })),
    )
    .await;
    assert_eq!(st_enq2, StatusCode::ACCEPTED);

    // 3rd distinct job hits max_pending_jobs (2) -> 429 Too Many Requests
    let (st_enq3, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/queues/mem-events/enqueue",
        tok,
        Some(serde_json::json!({ "payload": { "ev": 3 } })),
    )
    .await;
    assert_eq!(st_enq3, StatusCode::TOO_MANY_REQUESTS);

    let (st_qstats, qstats_bytes) = json_req(
        &app,
        Method::GET,
        "/_api/v1/queues/mem-events/stats",
        tok,
        None,
    )
    .await;
    assert_eq!(st_qstats, StatusCode::OK);
    assert_eq!(parse_json(qstats_bytes)["pending"], 2);
}

#[tokio::test]
async fn prometheus_metrics_and_healthz_access_log_skip() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // Call /_healthz multiple times; healthy probes should not spam access logs
    for _ in 0..3 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/_healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // Trigger one normal API request so HTTP counters & access logs record it
    let (st_status, _) = json_req(&app, Method::GET, "/_api/v1/status", tok, None).await;
    assert_eq!(st_status, StatusCode::OK);

    // Query Prometheus endpoint
    let prom_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_api/v1/metrics/prometheus")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(prom_resp.status(), StatusCode::OK);
    let ct = prom_resp
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("text/plain"));
    let body_bytes = prom_resp.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(text.contains("ferropod_http_requests_total{status_class=\"2xx\"}"));
    assert!(text.contains("ferropod_http_request_duration_ms_count"));
    assert!(text.contains("ferropod_projects_total"));

    // Confirm /_healthz did not produce access log rows
    let access_logs = h
        .logs
        .query(LogQuery {
            source: Some("access".into()),
            limit: Some(50),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(access_logs.iter().all(|l| !l.message.contains("/_healthz")));
}

#[tokio::test]
async fn openapi_llms_txt_and_lean_mcp_server_end_to_end() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());
    let pj = |b: Bytes| -> serde_json::Value { serde_json::from_slice(&b).unwrap() };

    // 1. Public OpenAPI 3.1.0 & llms.txt
    let (st_oa, oa_b) = json_req(&app, Method::GET, "/_api/v1/openapi.json", None, None).await;
    assert_eq!(st_oa, StatusCode::OK);
    let oa = pj(oa_b);
    assert_eq!(oa["openapi"], "3.1.0");
    assert!(oa["paths"]["/_mcp"].is_object());

    let llms_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/_api/v1/llms.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(llms_resp.status(), StatusCode::OK);
    let llms_body = String::from_utf8(
        llms_resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(llms_body.contains("deploy_stack"));

    // 2. Unauthenticated MCP POST -> 401
    let (st_unauth, _) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        None,
        Some(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })),
    )
    .await;
    assert_eq!(st_unauth, StatusCode::UNAUTHORIZED);

    // 3. MCP initialize & tools/list (10 tools)
    let (st_init, init_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" })),
    )
    .await;
    assert_eq!(st_init, StatusCode::OK);
    let init_json = pj(init_b);
    assert_eq!(init_json["result"]["serverInfo"]["name"], "ferropod");

    let (st_notif, _) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })),
    )
    .await;
    assert_eq!(st_notif, StatusCode::ACCEPTED);

    let (st_list, list_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })),
    )
    .await;
    assert_eq!(st_list, StatusCode::OK);
    let tools = pj(list_b)["result"]["tools"].as_array().unwrap().clone();
    assert_eq!(tools.len(), 10);

    // 4. deploy_stack inline static site + KV + route via MCP
    let (st_dep, dep_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "deploy_stack",
                "arguments": {
                    "name": "mcp-app",
                    "target_env": "prod",
                    "bindings": { "kv": { "CACHE": "mcp-kv" } },
                    "static_files": { "index.html": "<h1>Hello from MCP</h1>" },
                    "route_host": "mcp.local.dev"
                }
            }
        })),
    )
    .await;
    assert_eq!(st_dep, StatusCode::OK);
    let dep_res = pj(dep_b);
    assert_eq!(dep_res["result"]["isError"], false);

    let site_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::HOST, "mcp.local.dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(site_resp.status(), StatusCode::OK);

    // 5. manage_kv put/get & platform_overview & diagnose_logs via MCP
    let (_, kv_put_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "manage_kv",
                "arguments": { "action": "put", "namespace": "mcp-kv", "key": "k1", "value": "v1" }
            }
        })),
    )
    .await;
    assert_eq!(pj(kv_put_b)["result"]["isError"], false);

    let (_, kv_get_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        tok,
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "manage_kv",
                "arguments": { "action": "get", "namespace": "mcp-kv", "key": "k1" }
            }
        })),
    )
    .await;
    let kv_txt = pj(kv_get_b)["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(kv_txt.contains("v1"));

    // 6. Readonly token cannot invoke write tool (returns isError: true)
    let (_, ro_raw) = h
        .svc
        .create_token(&h.admin, "ro-mcp", TokenScope::Readonly, None)
        .await
        .unwrap();
    let (st_ro, ro_b) = json_req(
        &app,
        Method::POST,
        "/_mcp",
        Some(&ro_raw),
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "manage_kv",
                "arguments": { "action": "put", "namespace": "mcp-kv", "key": "k2", "value": "v2" }
            }
        })),
    )
    .await;
    assert_eq!(st_ro, StatusCode::OK);
    assert_eq!(pj(ro_b)["result"]["isError"], true);
}
