use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use bytes::Bytes;
use chrono::Utc;
use tokio::net::TcpListener;
use tower::ServiceExt;

use ferropod::core::model::{BlobAuth, BucketOpts, EnqueueOpts, FerroError, QueueConfig, TokenScope, UserDbOpts};
use ferropod::gateway::router::build_router;

use crate::common::{json_req, setup_harness};

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
async fn service_rate_limits_and_db_row_cap_guardrails() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // 1. Bucket rate limit (public_read + rate_limit_rpm = 3: 1 PUT + 2 GET = 3, 3rd GET -> 429)
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        tok,
        Some(serde_json::json!({ "name": "rl-bkt", "public_read": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let put_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/_blob/rl-bkt/hello.txt")
                .header(header::AUTHORIZATION, format!("Bearer {}", h.admin_token))
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from("hi"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::CREATED);

    let (st_patch, _) = json_req(
        &app,
        Method::PATCH,
        "/_api/v1/buckets/rl-bkt",
        tok,
        Some(serde_json::json!({ "rate_limit_rpm": 3 })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);

    for i in 0..2 {
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/_blob/rl-bkt/hello.txt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "bucket GET #{i}");
    }
    let resp_429 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/_blob/rl-bkt/hello.txt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp_429.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp_429.headers().contains_key("retry-after"));

    // 2. KV namespace rate limit (rate_limit_rpm = 2)
    let (st, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/kv",
        tok,
        Some(serde_json::json!({ "name": "rl-kv", "rate_limit_rpm": 2 })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);

    let (st_put, _) = json_req(
        &app,
        Method::PUT,
        "/_api/v1/kv/rl-kv/k1",
        tok,
        Some(serde_json::json!({ "value": "v1" })),
    )
    .await;
    assert!(st_put.is_success());

    let (st_get1, _) = json_req(&app, Method::GET, "/_api/v1/kv/rl-kv/k1", tok, None).await;
    assert_eq!(st_get1, StatusCode::OK);

    let (st_get2, _) = json_req(&app, Method::GET, "/_api/v1/kv/rl-kv/k1", tok, None).await;
    assert_eq!(st_get2, StatusCode::TOO_MANY_REQUESTS);

    // 3. DB streaming row cap (max_rows = 2) + rate limit (rate_limit_rpm = 2)
    let (st, db_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/dbs",
        tok,
        Some(serde_json::json!({
            "name": "rl-db",
            "init_sql": "CREATE TABLE t (id INTEGER PRIMARY KEY); INSERT INTO t VALUES (1),(2),(3),(4);",
            "max_rows": 2,
            "query_timeout_ms": 5000,
            "rate_limit_rpm": 2
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let db_meta: serde_json::Value = serde_json::from_slice(&db_bytes).unwrap();
    let db_id = db_meta["id"].as_str().unwrap();

    let (st_q1, q1_bytes) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/query"),
        tok,
        Some(serde_json::json!({ "sql": "SELECT id FROM t ORDER BY id" })),
    )
    .await;
    assert_eq!(st_q1, StatusCode::OK);
    let q1: serde_json::Value = serde_json::from_slice(&q1_bytes).unwrap();
    assert_eq!(q1["rows"].as_array().unwrap().len(), 2);

    let (st_q2, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/query"),
        tok,
        Some(serde_json::json!({ "sql": "SELECT id FROM t ORDER BY id" })),
    )
    .await;
    assert_eq!(st_q2, StatusCode::OK);

    let (st_q3, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/dbs/{db_id}/query"),
        tok,
        Some(serde_json::json!({ "sql": "SELECT id FROM t ORDER BY id" })),
    )
    .await;
    assert_eq!(st_q3, StatusCode::TOO_MANY_REQUESTS);

    // 4. Search max_docs = 1 + rate_limit_rpm = 3
    let (st, idx_bytes) = json_req(
        &app,
        Method::POST,
        "/_api/v1/search",
        tok,
        Some(serde_json::json!({
            "name": "rl-search",
            "max_docs": 1,
            "rate_limit_rpm": 3
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED);
    let idx_meta: serde_json::Value = serde_json::from_slice(&idx_bytes).unwrap();
    let idx_id = idx_meta["id"].as_str().unwrap();

    // #1 insert new doc -> ok
    let (st_d1, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/docs"),
        tok,
        Some(serde_json::json!({ "id": "d1", "title": "hello" })),
    )
    .await;
    assert!(st_d1.is_success());

    // #2 update existing doc -> ok (still 1 doc)
    let (st_d1_upd, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/docs"),
        tok,
        Some(serde_json::json!({ "id": "d1", "title": "updated" })),
    )
    .await;
    assert!(st_d1_upd.is_success());

    // #3 insert 2nd distinct doc -> rejected by max_docs
    let (st_d2, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/docs"),
        tok,
        Some(serde_json::json!({ "id": "d2", "title": "overflow" })),
    )
    .await;
    assert_eq!(st_d2, StatusCode::BAD_REQUEST);

    // #4 query hits rate_limit_rpm = 3
    let (st_sq, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/search/{idx_id}/query"),
        tok,
        Some(serde_json::json!({ "query": "updated" })),
    )
    .await;
    assert_eq!(st_sq, StatusCode::TOO_MANY_REQUESTS);

    // 5. Gateway auth brute-force protection: 30 failed 401 attempts trigger 429 on 31st
    for i in 0..30 {
        let (st_bad, _) =
            json_req(&app, Method::GET, "/_api/v1/status", Some("fp_invalid"), None).await;
        assert_eq!(st_bad, StatusCode::UNAUTHORIZED, "attempt #{i}");
    }
    let (st_locked, _) =
        json_req(&app, Method::GET, "/_api/v1/status", Some("fp_invalid"), None).await;
    assert_eq!(st_locked, StatusCode::TOO_MANY_REQUESTS);

    // 6. execute_user_db_batch rejects batches with > 200 statements
    let batch_db = h
        .svc
        .create_user_db_with_opts(
            &h.admin,
            "batch-guard-db",
            UserDbOpts {
                init_sql: Some("CREATE TABLE b (id INTEGER PRIMARY KEY);".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let too_many_stmts = vec![("SELECT 1".to_string(), vec![]); 201];
    let batch_err = h
        .svc
        .execute_user_db_batch(&h.admin, &batch_db.id, too_many_stmts)
        .await
        .unwrap_err();
    assert!(matches!(batch_err, FerroError::InvalidArgument(_)));

    // 7. DB max_rows cached check enforces limit on INSERT and recovers after DELETE
    let cache_db = h
        .svc
        .create_user_db_with_opts(
            &h.admin,
            "cache-rows-db",
            UserDbOpts {
                init_sql: Some("CREATE TABLE items (id INTEGER PRIMARY KEY);".to_string()),
                max_rows: Some(2),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &cache_db.id,
            vec![("INSERT INTO items VALUES (1)".to_string(), vec![])],
        )
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &cache_db.id,
            vec![("INSERT INTO items VALUES (2)".to_string(), vec![])],
        )
        .await
        .unwrap();
    let overflow_err = h
        .svc
        .execute_user_db_batch(
            &h.admin,
            &cache_db.id,
            vec![("INSERT INTO items VALUES (3)".to_string(), vec![])],
        )
        .await
        .unwrap_err();
    assert!(matches!(overflow_err, FerroError::InvalidArgument(_)));

    // DELETE invalidates row count cache so subsequent INSERT succeeds
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &cache_db.id,
            vec![("DELETE FROM items WHERE id = 1".to_string(), vec![])],
        )
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &cache_db.id,
            vec![("INSERT INTO items VALUES (4)".to_string(), vec![])],
        )
        .await
        .unwrap();

    // 8. queue_enqueue rejects payloads > 512 KiB
    h.svc
        .queue_create(
            &h.admin,
            QueueConfig {
                name: "big-payload-q".to_string(),
                project_id: None,
                consumer_project_id: None,
                max_concurrency: 2,
                rate_limit_per_sec: None,
                batch_size: 10,
                lease_secs: 30,
                max_attempts: 3,
                created_at: Utc::now().timestamp_millis(),
                group: None,
                description: None,
                retry_backoff_secs: 5,
                dlq_retention_secs: None,
                ephemeral: false,
                max_pending_jobs: None,
            },
        )
        .await
        .unwrap();
    let oversized_err = h
        .svc
        .queue_enqueue(
            &h.admin,
            "big-payload-q",
            Bytes::from(vec![b'x'; 512 * 1024 + 1]),
            EnqueueOpts::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(oversized_err, FerroError::InvalidArgument(_)));

    // 9. Internal BlobAuth::Caller enforces bucket rate_limit_rpm on head_object / get_object_stream
    h.svc
        .create_bucket(&h.admin, "internal-rl-bkt", None, false)
        .await
        .unwrap();
    let auth = BlobAuth::Caller(&h.admin);
    let stream = Box::pin(futures_util::stream::once(async {
        Ok::<_, std::io::Error>(Bytes::from_static(b"internal"))
    }));
    h.svc
        .put_object_stream(&auth, "internal-rl-bkt", "a.txt", stream, "text/plain", None)
        .await
        .unwrap();
    h.svc
        .update_bucket_with_opts(
            &h.admin,
            "internal-rl-bkt",
            BucketOpts {
                rate_limit_rpm: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    assert!(h
        .svc
        .head_object(&auth, "internal-rl-bkt", "a.txt")
        .await
        .unwrap()
        .is_some());
    assert!(h
        .svc
        .get_object_stream(&auth, "internal-rl-bkt", "a.txt", None)
        .await
        .unwrap()
        .is_some());
    let internal_blob_err = match h.svc.head_object(&auth, "internal-rl-bkt", "a.txt").await {
        Ok(_) => panic!("expected RateLimited on 3rd internal blob call"),
        Err(e) => e,
    };
    assert!(matches!(internal_blob_err, FerroError::RateLimited(_)));
}

#[tokio::test]
async fn multi_tenant_bulkhead_realtime_db_cache_and_alert_cooldown() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // 1. Hold all data-plane http_concurrency permits -> control-plane /_healthz and /_api/v1/status still succeed,
    //    while a non-control-plane route is rejected with 429.
    let mut data_permits = Vec::new();
    while let Some(p) = h.svc.try_acquire_http_permit() {
        data_permits.push(p);
    }
    assert!(!data_permits.is_empty());

    let (st_hz, _) = json_req(&app, Method::GET, "/_healthz", None, None).await;
    assert_eq!(st_hz, StatusCode::OK);
    let (st_api, _) = json_req(&app, Method::GET, "/_api/v1/status", tok, None).await;
    assert_eq!(st_api, StatusCode::OK);
    let (st_data, _) = json_req(&app, Method::GET, "/some-worker-path", None, None).await;
    assert_eq!(st_data, StatusCode::TOO_MANY_REQUESTS);
    drop(data_permits);

    // 2. Realtime publish rejects payloads > 64 KiB and enforces SSE connection limit + drop cleanup
    let big_rt_err = h
        .svc
        .realtime_publish(
            &h.admin,
            "cap-ch",
            Some("big"),
            serde_json::json!({ "blob": "a".repeat(65 * 1024) }),
        )
        .await
        .unwrap_err();
    assert!(matches!(big_rt_err, FerroError::InvalidArgument(_)));

    let mut subs = Vec::new();
    for _ in 0..128 {
        subs.push(h.svc.realtime_subscribe(&h.admin, "cap-ch").await.unwrap());
    }
    let full_err = match h.svc.realtime_subscribe(&h.admin, "cap-ch").await {
        Ok(_) => panic!("expected RateLimited when channel SSE limit is reached"),
        Err(e) => e,
    };
    assert!(matches!(full_err, FerroError::RateLimited(_)));
    subs.pop();
    assert!(h.svc.realtime_subscribe(&h.admin, "cap-ch").await.is_ok());
    drop(subs);

    // 3. DB metadata cache invalidates immediately on update_user_db
    let cached_db = h
        .svc
        .create_user_db_with_opts(
            &h.admin,
            "meta-cache-db",
            UserDbOpts {
                init_sql: Some("CREATE TABLE m (id INTEGER PRIMARY KEY);".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    h.svc
        .execute_user_db_batch(
            &h.admin,
            &cached_db.id,
            vec![("INSERT INTO m VALUES (1)".to_string(), vec![])],
        )
        .await
        .unwrap();
    h.svc
        .update_user_db(
            &h.admin,
            &cached_db.id,
            UserDbOpts {
                read_only: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ro_err = h
        .svc
        .execute_user_db_batch(
            &h.admin,
            &cached_db.id,
            vec![("INSERT INTO m VALUES (2)".to_string(), vec![])],
        )
        .await
        .unwrap_err();
    assert!(matches!(ro_err, FerroError::Forbidden(_)));

    // 4. Alert cooldown suppresses duplicate alerts within 60s
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_srv = hits.clone();
    let hook_router = Router::new().route(
        "/hook",
        axum::routing::post(move || {
            let hits_srv = hits_srv.clone();
            async move {
                hits_srv.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        axum::serve(listener, hook_router).await.unwrap();
    });

    h.svc
        .create_alert_webhook(
            &h.admin,
            ferropod::core::model::AlertWebhookOpts {
                name: Some("cd-hook".to_string()),
                url: Some(format!("http://{hook_addr}/hook")),
                secret: None,
                clear_secret: None,
                events: Some(vec!["worker_oom".to_string()]),
                enabled: Some(true),
            },
        )
        .await
        .unwrap();

    h.svc.fire_alert(
        "worker_oom",
        "Worker OOM 1".into(),
        serde_json::json!({ "project_id": "proj_cd" }),
    );
    h.svc.fire_alert(
        "worker_oom",
        "Worker OOM 2".into(),
        serde_json::json!({ "project_id": "proj_cd" }),
    );
    for _ in 0..80 {
        if hits.load(Ordering::SeqCst) >= 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    srv.abort();
}
