use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use bytes::Bytes;
use chrono::Utc;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

use ferropod::config::{Config, Profile};
use ferropod::core::model::{
    Bindings, BlobAuth, Caller, EnqueueOpts, JobStatus, LogEntry, LogQuery, QueueConfig, SqlValue,
    TargetEnv, TokenScope,
};
use ferropod::core::service::{CreateDeploymentInput, PlatformDeps, PlatformService};
use ferropod::core::traits::{DbManager, KvStore, LogStore, ObjectStore, QueueStore};
use ferropod::gateway::router::build_router;
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore, log_store::DailyLogStore,
    queue_store::TursoQueueStore, user_db::TursoUserDbManager,
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
                max_concurrency: 4,
                rate_limit_per_sec: None,
                batch_size: 20,
                lease_secs: 30,
                max_attempts: 2,
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
        .create_token(&h.admin, "token-a", TokenScope::Project(proj_a.id.clone()), None)
        .await
        .unwrap();
    let (_, tok_b) = h
        .svc
        .create_token(&h.admin, "token-b", TokenScope::Project(proj_b.id.clone()), None)
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
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
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
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
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
                    "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT NOT NULL)".into(),
                    vec![],
                ),
                (
                    "INSERT INTO items (v) VALUES ('alpha')".into(),
                    vec![],
                ),
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

    let (st, _) = json_req(&app, Method::GET, "/_api/v1/kv/ns_a/foo", Some(&tok_ro), None).await;
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
    let (st, _) = json_req(&app, Method::GET, "/_blob/bkt-a/secret.txt", Some(&tok_b), None).await;
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
        assert_eq!(st, StatusCode::BAD_REQUEST, "SQL should be rejected: {bad_sql}");
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
    let mut sub_a = h
        .svc
        .subscribe_logs(&Caller::project("tok_a", &proj_a.id));
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
    assert!(entries.iter().all(|e| e.project_id.as_deref() == Some(proj_a.id.as_str())));
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
            svc.upsert_route(&caller, &format!("host-{i}.local"), &pid, TargetEnv::Preview)
                .await
                .unwrap();
        }));
    }
    for f in route_futs {
        f.await.unwrap();
    }
    for i in 0..10 {
        assert!(
            h.svc.lookup_route(&format!("host-{i}.local")).await.is_some(),
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
        .create_token(&h.admin, "ui-proj", TokenScope::Project(proj.id.clone()), None)
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
            resp.into_body().collect().await.unwrap().to_bytes().to_vec(),
        )
        .unwrap();
        assert!(body.contains("/_panel/assets/styles.css"));
        assert!(body.contains("/_panel/assets/i18n.js"));
        assert!(body.contains("/_panel/assets/app.js"));
    }

    for (asset, expected_ct) in [
        ("/_panel/assets/styles.css", "text/css"),
        ("/_panel/assets/i18n.js", "application/javascript"),
        ("/_panel/assets/app.js", "application/javascript"),
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
    let resolved = resolve_tls(&cfg, vec!["app.example.com".into(), "panel.example.com".into()]).unwrap();
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
            format!("GET /_panel HTTP/1.1\r\nHost: localhost:{http_port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut http_resp = String::new();
    http_stream.read_to_string(&mut http_resp).await.unwrap();
    assert!(http_resp.starts_with("HTTP/1.1 308"), "unexpected HTTP response: {http_resp}");
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
    assert!(https_resp.starts_with("HTTP/1.1 200"), "unexpected HTTPS response: {https_resp}");
    assert!(https_resp.contains(r#"{"status":"ok"}"#), "unexpected body: {https_resp}");

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
    assert_eq!(status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&runtime));
    let runtime: serde_json::Value = serde_json::from_slice(&runtime).unwrap();
    let id = runtime["id"].as_str().unwrap();
    assert_eq!(runtime["group"], "分析");
    let (_, legacy) = json_req(&app, Method::GET, &format!("/_api/v1/projects/{id}"), admin, None).await;
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&legacy).unwrap(), runtime);

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
            assert_eq!(status, StatusCode::CREATED, "{path}: {}", String::from_utf8_lossy(&body));
        }
        let (status, body) = json_req(&app, Method::GET, &path, admin, None).await;
        assert_eq!(status, StatusCode::OK);
        let resources: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        for name in names {
            assert!(resources.iter().any(|r| r["name"] == name && r["group"] == "分析"));
        }
    }

    let (_, admin_groups) = json_req(&app, Method::GET, "/_api/v1/resource-groups", admin, None).await;
    assert_eq!(serde_json::from_slice::<Vec<String>>(&admin_groups).unwrap(), ["分析"]);

    let other = h.svc.create_project(&h.admin, "ungrouped-worker", None).await.unwrap();
    let (_, scoped_token) = h.svc.create_token(
        &h.admin,
        "group-scope",
        TokenScope::Project(other.id),
        None,
    ).await.unwrap();
    let (status, scoped_groups) = json_req(
        &app,
        Method::GET,
        "/_api/v1/resource-groups",
        Some(&scoped_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(serde_json::from_slice::<Vec<String>>(&scoped_groups).unwrap().is_empty());
}
