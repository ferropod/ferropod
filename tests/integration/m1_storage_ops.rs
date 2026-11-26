use std::collections::HashMap;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use bytes::Bytes;
use chrono::Utc;
use http_body_util::BodyExt;
use tower::ServiceExt;

use ferropod::core::model::{
    Bindings, BlobAuth, Caller, EnqueueOpts, JobStatus, LogEntry, LogQuery,
    QueueConfig, SqlValue, TargetEnv, TokenScope,
};
use ferropod::core::service::CreateDeploymentInput;
use ferropod::core::traits::{DbManager, KvStore, LogStore, ObjectStore, QueueStore};
use ferropod::gateway::router::build_router;

use crate::common::{json_req, setup_harness};

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
    for chunk in statements.chunks(200) {
        let (status, _) = json_req(
            &app,
            Method::POST,
            &format!("{path}/execute"),
            token,
            Some(serde_json::json!({"statements": chunk})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }
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
async fn hybrid_vector_search_and_ai_gateway_failover_cache_usage() {
    use axum::routing::post as axum_post;

    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());
    let pj = |b: Bytes| -> serde_json::Value { serde_json::from_slice(&b).unwrap() };

    // 1. Vector & Hybrid BM25+Cosine search
    let (st_idx, idx_b) = json_req(
        &app,
        Method::POST,
        "/_api/v1/search",
        tok,
        Some(serde_json::json!({ "name": "rag-docs" })),
    )
    .await;
    assert_eq!(st_idx, StatusCode::CREATED);
    let idx_id = pj(idx_b)["id"].as_str().unwrap().to_string();

    for doc in [
        serde_json::json!({
            "id": "d1",
            "title": "Rust Memory Model",
            "body": "Ownership and borrowing in Rust",
            "embedding": [0.95, 0.05, 0.0]
        }),
        serde_json::json!({
            "id": "d2",
            "title": "Cooking Pasta",
            "body": "Boil salted water",
            "embedding": [0.0, 0.1, 0.99]
        }),
    ] {
        let (st_u, _) = json_req(
            &app,
            Method::POST,
            &format!("/_api/v1/search/{idx_id}/docs"),
            tok,
            Some(doc),
        )
        .await;
        assert_eq!(st_u, StatusCode::CREATED);
    }

    // Vector-only query near [1, 0, 0] returns d1 on top
    let (st_vq, vq_b) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/query"),
        tok,
        Some(serde_json::json!({
            "query": "",
            "vector": [1.0, 0.0, 0.0],
            "limit": 2
        })),
    )
    .await;
    assert_eq!(st_vq, StatusCode::OK);
    let vq = pj(vq_b);
    assert_eq!(vq["hits"][0]["doc"]["id"], "d1");

    // Hybrid query combining BM25 + vector
    let (st_hq, hq_b) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/query"),
        tok,
        Some(serde_json::json!({
            "query": "Rust",
            "vector": [1.0, 0.0, 0.0],
            "alpha": 0.5,
            "limit": 2
        })),
    )
    .await;
    assert_eq!(st_hq, StatusCode::OK);
    assert_eq!(pj(hq_b)["hits"][0]["doc"]["id"], "d1");

    // 2. Built-in AI Gateway: primary returns 500 -> automatic failover to backup + response caching + RPM limit
    let fail_app = axum::Router::new().route(
        "/v1/chat/completions",
        axum_post(|| async {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({ "error": "upstream overloaded" })),
            )
        }),
    );
    let l1 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr1 = l1.local_addr().unwrap();
    let h1 = tokio::spawn(async move {
        let _ = axum::serve(l1, fail_app).await;
    });

    let ok_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ok_hits_srv = ok_hits.clone();
    let ok_app = axum::Router::new().route(
        "/v1/chat/completions",
        axum_post(move || {
            let hits = ok_hits_srv.clone();
            async move {
                hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                axum::Json(serde_json::json!({
                    "id": "chatcmpl-1",
                    "model": "gpt-fallback",
                    "choices": [{ "message": { "role": "assistant", "content": "hi from fallback" } }],
                    "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 }
                }))
            }
        }),
    );
    let l2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = l2.local_addr().unwrap();
    let h2 = tokio::spawn(async move {
        let _ = axum::serve(l2, ok_app).await;
    });

    // Register backup provider first, then primary pointing to fallback
    let (st_fb, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/ai/providers",
        tok,
        Some(serde_json::json!({
            "name": "backup",
            "base_url": format!("http://{addr2}/v1"),
            "api_key": "sk-backup",
            "default_model": "gpt-fallback"
        })),
    )
    .await;
    assert_eq!(st_fb, StatusCode::CREATED);

    let (st_pri, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/ai/providers",
        tok,
        Some(serde_json::json!({
            "name": "primary",
            "base_url": format!("http://{addr1}/v1"),
            "api_key": "sk-primary",
            "fallback_provider": "backup",
            "rate_limit_rpm": 2,
            "cache_ttl_secs": 60
        })),
    )
    .await;
    assert_eq!(st_pri, StatusCode::CREATED);

    // Confirm API key is never leaked in list response
    let (st_plist, plist_b) = json_req(&app, Method::GET, "/_api/v1/ai/providers", tok, None).await;
    assert_eq!(st_plist, StatusCode::OK);
    let plist_str = String::from_utf8(plist_b.to_vec()).unwrap();
    assert!(!plist_str.contains("sk-primary"));

    // Request 1 hits primary (500) -> fails over to backup (200), X-Ferropod-Cache: MISS
    let r1 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/_ai/v1/chat/completions")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-ferropod-provider", "primary")
                .body(Body::from(
                    serde_json::json!({ "messages": [{ "role": "user", "content": "hello" }] })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    assert_eq!(r1.headers().get("x-ferropod-cache").unwrap(), "MISS");
    assert_eq!(r1.headers().get("x-ferropod-provider").unwrap(), "backup");

    // Request 2 with identical body hits response cache without contacting upstream
    let r2 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/_ai/v1/chat/completions")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-ferropod-provider", "primary")
                .body(Body::from(
                    serde_json::json!({ "messages": [{ "role": "user", "content": "hello" }] })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    assert_eq!(r2.headers().get("x-ferropod-cache").unwrap(), "HIT");
    assert_eq!(ok_hits.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Request 3 hits primary RPM limit (2) -> 429
    let r3 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/_ai/v1/chat/completions")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-ferropod-provider", "primary")
                .body(Body::from(
                    serde_json::json!({ "messages": [{ "role": "user", "content": "hello3" }] })
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);

    // Verify token usage accounting
    let (st_usage, usage_b) = json_req(&app, Method::GET, "/_api/v1/ai/usage", tok, None).await;
    assert_eq!(st_usage, StatusCode::OK);
    let usage = pj(usage_b);
    assert_eq!(usage["total_tokens"], 20);
    assert_eq!(usage["cached_requests"], 1);

    h1.abort();
    h2.abort();
}

#[tokio::test]
async fn delete_non_empty_bucket_removes_objects_and_bucket() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        tok,
        Some(serde_json::json!({ "name": "non-empty-del" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    for (key, content) in [
        ("hello.txt", "hello-content"),
        ("nested/file.txt", "nested-content"),
    ] {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/_blob/non-empty-del/{key}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                    .header(header::CONTENT_TYPE, "text/plain")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let (st_del, _) = json_req(
        &app,
        Method::DELETE,
        "/_api/v1/buckets/non-empty-del",
        tok,
        None,
    )
    .await;
    assert_eq!(st_del, StatusCode::NO_CONTENT);

    let (st_get, _) = json_req(&app, Method::GET, "/_blob/non-empty-del/hello.txt", tok, None).await;
    assert_eq!(st_get, StatusCode::NOT_FOUND);

    let (st_list, list_bytes) = json_req(&app, Method::GET, "/_api/v1/buckets", tok, None).await;
    assert_eq!(st_list, StatusCode::OK);
    let buckets: Vec<serde_json::Value> = serde_json::from_slice(&list_bytes).unwrap();
    assert!(!buckets.iter().any(|b| b["name"] == "non-empty-del"));

    assert!(h.blobs.list("non-empty-del", None, 10).await.unwrap().is_empty());
    assert_eq!(h.blobs.run_gc_batch(10).await.unwrap(), 2);

    let (st_del_again, _) = json_req(
        &app,
        Method::DELETE,
        "/_api/v1/buckets/non-empty-del",
        tok,
        None,
    )
    .await;
    assert_eq!(st_del_again, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn blob_folder_browse_and_prefix_delete() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        tok,
        Some(serde_json::json!({ "name": "folder-bkt" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    // Upload nested objects; omit Content-Type on assets/app.js and send empty Content-Type on root.txt
    // to verify mime_guess fallback works in both cases.
    let uploads = [
        ("root.txt", "root-body", Some("")),
        ("assets/app.js", "console.log(1);", None),
        ("assets/img/logo.png", "png-data", Some("image/png")),
        ("assets/img/bg.jpg", "jpg-data", Some("image/jpeg")),
        ("assets2/other.txt", "other-body", Some("text/plain")),
    ];
    for (key, content, ct) in uploads {
        let mut req_builder = Request::builder()
            .method(Method::PUT)
            .uri(format!("/_blob/folder-bkt/{key}"))
            .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token));
        if let Some(ctype) = ct {
            req_builder = req_builder.header(header::CONTENT_TYPE, ctype);
        }
        let resp = app
            .clone()
            .oneshot(req_builder.body(Body::from(content)).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED, "failed putting {key}");
    }

    // Browse root directory
    let (st_root, root_bytes) =
        json_req(&app, Method::GET, "/_api/v1/buckets/folder-bkt/browse", tok, None).await;
    assert_eq!(st_root, StatusCode::OK);
    let root_res: serde_json::Value = serde_json::from_slice(&root_bytes).unwrap();
    assert_eq!(root_res["prefix"], "");
    assert_eq!(root_res["truncated"], false);
    let root_folders = root_res["folders"].as_array().unwrap();
    assert_eq!(root_folders.len(), 2);
    assert_eq!(root_folders[0]["prefix"], "assets/");
    assert_eq!(root_folders[0]["name"], "assets");
    assert_eq!(root_folders[0]["object_count"], 3);
    assert_eq!(root_folders[1]["prefix"], "assets2/");
    assert_eq!(root_folders[1]["name"], "assets2");
    assert_eq!(root_folders[1]["object_count"], 1);
    let root_objs = root_res["objects"].as_array().unwrap();
    assert_eq!(root_objs.len(), 1);
    assert_eq!(root_objs[0]["key"], "root.txt");
    assert_eq!(root_objs[0]["content_type"], "text/plain");

    // Browse assets/ directory
    let (st_assets, assets_bytes) = json_req(
        &app,
        Method::GET,
        "/_api/v1/buckets/folder-bkt/browse?prefix=assets/",
        tok,
        None,
    )
    .await;
    assert_eq!(st_assets, StatusCode::OK);
    let assets_res: serde_json::Value = serde_json::from_slice(&assets_bytes).unwrap();
    assert_eq!(assets_res["prefix"], "assets/");
    let assets_folders = assets_res["folders"].as_array().unwrap();
    assert_eq!(assets_folders.len(), 1);
    assert_eq!(assets_folders[0]["prefix"], "assets/img/");
    assert_eq!(assets_folders[0]["name"], "img");
    assert_eq!(assets_folders[0]["object_count"], 2);
    let assets_objs = assets_res["objects"].as_array().unwrap();
    assert_eq!(assets_objs.len(), 1);
    assert_eq!(assets_objs[0]["key"], "assets/app.js");
    let js_ct = assets_objs[0]["content_type"].as_str().unwrap();
    assert!(
        js_ct.contains("javascript"),
        "expected javascript mime type, got {js_ct}"
    );

    // Search within assets/
    let (st_search, search_bytes) = json_req(
        &app,
        Method::GET,
        "/_api/v1/buckets/folder-bkt/browse?prefix=assets/&search=logo",
        tok,
        None,
    )
    .await;
    assert_eq!(st_search, StatusCode::OK);
    let search_res: serde_json::Value = serde_json::from_slice(&search_bytes).unwrap();
    assert!(search_res["folders"].as_array().unwrap().is_empty());
    let search_objs = search_res["objects"].as_array().unwrap();
    assert_eq!(search_objs.len(), 1);
    assert_eq!(search_objs[0]["key"], "assets/img/logo.png");

    // Empty prefix deletion must be rejected
    let (st_empty_del, _) = json_req(
        &app,
        Method::DELETE,
        "/_api/v1/buckets/folder-bkt/prefix?prefix=",
        tok,
        None,
    )
    .await;
    assert_eq!(st_empty_del, StatusCode::BAD_REQUEST);

    // Delete assets/ prefix recursively
    let (st_del_pfx, del_pfx_bytes) = json_req(
        &app,
        Method::DELETE,
        "/_api/v1/buckets/folder-bkt/prefix?prefix=assets/",
        tok,
        None,
    )
    .await;
    assert_eq!(st_del_pfx, StatusCode::OK);
    let del_res: serde_json::Value = serde_json::from_slice(&del_pfx_bytes).unwrap();
    assert_eq!(del_res["prefix"], "assets/");
    assert_eq!(del_res["deleted"], 3);

    // Verify root.txt and assets2/other.txt remain untouched while assets/* are gone
    let (st_after, after_bytes) =
        json_req(&app, Method::GET, "/_api/v1/buckets/folder-bkt/browse", tok, None).await;
    assert_eq!(st_after, StatusCode::OK);
    let after_res: serde_json::Value = serde_json::from_slice(&after_bytes).unwrap();
    let after_folders = after_res["folders"].as_array().unwrap();
    assert_eq!(after_folders.len(), 1);
    assert_eq!(after_folders[0]["prefix"], "assets2/");
    let after_objs = after_res["objects"].as_array().unwrap();
    assert_eq!(after_objs.len(), 1);
    assert_eq!(after_objs[0]["key"], "root.txt");
}
