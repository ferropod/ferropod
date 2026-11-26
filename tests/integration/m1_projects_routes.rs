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
use ferropod::core::model::{BlobAuth, Caller, LogEntry, LogQuery, QueueConfig, TargetEnv, TokenScope};
use ferropod::core::service::CreateDeploymentInput;
use ferropod::core::traits::LogStore;
use ferropod::gateway::router::build_router;

use crate::common::{json_req, setup_harness, setup_harness_with};

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
        assert!(body.contains("#/ai"));
        assert!(body.contains("select-lang"));
        assert!(body.contains("user-scalable=no"));
        assert!(body.contains("maximum-scale=1.0"));
        assert!(!body.contains("data-nav=\"routes\""));
        assert!(!body.contains("href=\"#/routes\""));
    }

    for (asset, expected_ct) in [
        ("/_panel/assets/styles.css", "text/css"),
        ("/_panel/assets/i18n.js", "application/javascript"),
        ("/_panel/assets/i18n_zh_tw.json", "application/json"),
        ("/_panel/assets/i18n_ja.json", "application/json"),
        ("/_panel/assets/i18n_ko.json", "application/json"),
        ("/_panel/assets/i18n_es.json", "application/json"),
        ("/_panel/assets/app.js", "application/javascript"),
        ("/_panel/assets/ai_component.js", "application/javascript"),
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
        if expected_ct == "application/json" {
            let bytes = resp.into_body().collect().await.unwrap().to_bytes();
            let json: serde_json::Value = serde_json::from_slice(&bytes)
                .unwrap_or_else(|e| panic!("invalid JSON in {asset}: {e}"));
            assert!(
                json.get("app.pageTitle")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.is_empty()),
                "missing app.pageTitle in {asset}"
            );
        } else if asset == "/_panel/assets/styles.css" {
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
        } else if asset == "/_panel/assets/app.js" {
            let js = String::from_utf8(
                resp.into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .to_vec(),
            )
            .unwrap();
            assert!(!js.contains("function renderRoutes"));
            assert!(js.contains("openAppRouteModal"));
            assert!(js.contains("PATCH"));
        } else if matches!(
            asset,
            "/_panel/assets/runtime_component.js"
                | "/_panel/assets/ops_component.js"
                | "/_panel/assets/ai_component.js"
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
            if asset == "/_panel/assets/runtime_component.js" {
                assert!(js.contains("runtime-search-input"));
            }
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
    assert!(oa["paths"]["/_api/v1/apps"].is_object());

    let (st_apps, _) = json_req(&app, Method::GET, "/_api/v1/apps", tok, None).await;
    assert_eq!(st_apps, StatusCode::OK);

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
    assert_eq!(tools.len(), 11);

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

#[tokio::test]
async fn declarative_cli_init_and_idempotent_deploy_with_migrations() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let proj_dir = tempfile::tempdir().unwrap();
    let files = ferropod::deploy_cli::init_project(proj_dir.path(), "cli-demo", "static").unwrap();
    assert!(files
        .iter()
        .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("ferropod.toml")));

    let opts = ferropod::deploy_cli::DeployOptions {
        config_path: proj_dir.path().join("ferropod.toml"),
        server_url: format!("http://{addr}"),
        token: h.admin_token.clone(),
        target_env: TargetEnv::Prod,
        dry_run: false,
    };

    // First deploy uploads index.html, applies 0001_init.sql, creates deployment & route
    let first = ferropod::deploy_cli::deploy_manifest(opts.clone())
        .await
        .unwrap();
    assert!(first.deployment_id.is_some());
    assert_eq!(first.assets_uploaded, 1);
    assert_eq!(first.assets_skipped, 0);
    assert_eq!(first.migrations_applied, 1);
    assert_eq!(first.routes_bound, 1);

    // Verify deployed static site responds via its route host
    let http = reqwest::Client::new();
    let site_resp = http
        .get(format!("http://{addr}/"))
        .header("Host", "cli-demo.local")
        .send()
        .await
        .unwrap();
    assert_eq!(site_resp.status(), reqwest::StatusCode::OK);
    assert!(site_resp
        .text()
        .await
        .unwrap()
        .contains("Hello from cli-demo"));

    // Second deploy skips unchanged static assets (SHA-256) and already-applied migration
    let second = ferropod::deploy_cli::deploy_manifest(opts).await.unwrap();
    assert_eq!(second.assets_uploaded, 0);
    assert_eq!(second.assets_skipped, 1);
    assert_eq!(second.migrations_applied, 0);

    server_handle.abort();
}

#[tokio::test]
async fn dynamic_route_proxy_basic_auth_rate_limit_canary_health_and_rollback() {
    use axum::routing::get as axum_get;

    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());
    let pj = |b: Bytes| -> serde_json::Value { serde_json::from_slice(&b).unwrap() };

    // 1. Two independent mock upstreams for dynamic multi-route reverse proxy
    let up_a = axum::Router::new().route("/api/v1/ping", axum_get(|| async { "upstream-a" }));
    let la = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addra = la.local_addr().unwrap();
    let ha = tokio::spawn(async move {
        let _ = axum::serve(la, up_a).await;
    });

    let up_b = axum::Router::new().route("/proxy/hello", axum_get(|| async { "upstream-b" }));
    let lb = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addrb = lb.local_addr().unwrap();
    let hb = tokio::spawn(async move {
        let _ = axum::serve(lb, up_b).await;
    });

    // 2. Create project + two static deployments (v1 prod + v2 canary) for testing Basic Auth, Rate Limit, Canary Split, and Proxy
    let proj = h
        .svc
        .create_project(&h.admin, "edge-gw", None)
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "edge-v1", Some(&proj.id), true)
        .await
        .unwrap();
    h.svc
        .create_bucket(&h.admin, "edge-v2", Some(&proj.id), true)
        .await
        .unwrap();
    let auth = BlobAuth::Caller(&h.admin);
    h.svc
        .put_object_stream(
            &auth,
            "edge-v1",
            "index.html",
            Box::pin(futures_util::stream::once(async {
                Ok::<_, std::io::Error>(Bytes::from("v1-prod"))
            })),
            "text/plain",
            None,
        )
        .await
        .unwrap();
    h.svc
        .put_object_stream(
            &auth,
            "edge-v2",
            "index.html",
            Box::pin(futures_util::stream::once(async {
                Ok::<_, std::io::Error>(Bytes::from("v2-canary"))
            })),
            "text/plain",
            None,
        )
        .await
        .unwrap();

    let dep1 = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                assets_bucket: Some("edge-v1".into()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let dep2 = h
        .svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                assets_bucket: Some("edge-v2".into()),
                target_env: Some(TargetEnv::Preview),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Route A: Basic Auth + Rate Limit (5 rpm) + 50% Canary + Dynamic proxy /api
    let (st_ra, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        tok,
        Some(serde_json::json!({
            "host": "a.edge.test",
            "project": proj.id,
            "target_env": "prod",
            "proxy_prefix": "/api",
            "proxy_upstream": format!("http://{addra}"),
            "rate_limit_rpm": 12,
            "basic_auth_user": "alice",
            "basic_auth_password": "secret",
            "canary_deployment_id": dep2.id,
            "canary_weight": 50
        })),
    )
    .await;
    assert_eq!(st_ra, StatusCode::OK);

    // Route B: second host proxying /proxy to upstream B
    let (st_rb, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        tok,
        Some(serde_json::json!({
            "host": "b.edge.test",
            "project": proj.id,
            "target_env": "prod",
            "proxy_prefix": "/proxy",
            "proxy_upstream": format!("http://{addrb}"),
            "rate_limit_rpm": 2
        })),
    )
    .await;
    assert_eq!(st_rb, StatusCode::OK);

    // Verify Basic Auth challenge without credentials
    let no_auth = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .header(header::HOST, "a.edge.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_auth.status(), StatusCode::UNAUTHORIZED);

    // With valid Basic Auth, 10 requests produce deterministic 5 prod + 5 canary split
    use base64::Engine as _;
    let basic_hdr = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("alice:secret")
    );
    let mut canary_count = 0usize;
    let mut prod_count = 0usize;
    for _ in 0..10 {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::HOST, "a.edge.test")
                    .header(header::AUTHORIZATION, &basic_hdr)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
        if r.headers().get("x-ferropod-canary").unwrap() == "true" {
            canary_count += 1;
        } else {
            prod_count += 1;
        }
    }
    assert_eq!(canary_count, 5);
    assert_eq!(prod_count, 5);

    // Verify dynamic proxy on Route A (/api/v1/ping)
    let pa = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/ping")
                .header(header::HOST, "a.edge.test")
                .header(header::AUTHORIZATION, &basic_hdr)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(pa.status(), StatusCode::OK);
    let pa_body = pa.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&pa_body[..], b"upstream-a");

    // Verify dynamic proxy on Route B + rate_limit_rpm = 2 enforcement (3rd request -> 429)
    for idx in 0..3 {
        let pb = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/proxy/hello")
                    .header(header::HOST, "b.edge.test")
                    .header("x-forwarded-for", "203.0.113.10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        if idx < 2 {
            assert_eq!(pb.status(), StatusCode::OK);
            let pb_body = pb.into_body().collect().await.unwrap().to_bytes();
            assert_eq!(&pb_body[..], b"upstream-b");
        } else {
            assert_eq!(pb.status(), StatusCode::TOO_MANY_REQUESTS);
            assert!(pb.headers().contains_key("retry-after"));
        }
    }

    // 3. Health-check gated promote & 5xx spike auto-rollback using DeploymentHealthTracker
    let _ = dep1;
    let (st_prom, prom_b) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/projects/{}/promote", proj.id),
        tok,
        Some(serde_json::json!({ "deployment_id": dep2.id, "verify_health": true })),
    )
    .await;
    assert_eq!(st_prom, StatusCode::OK);
    assert_eq!(pj(prom_b)["id"], dep2.id);

    // Simulate 10 outcomes with >= 50% 5xx -> tracker triggers automatic rollback to dep1
    let mut should_rb = false;
    for i in 0..10 {
        if h.svc.health_tracker().record_outcome(&dep2.id, i < 6) {
            should_rb = true;
        }
    }
    assert!(should_rb);
    let rolled = h
        .svc
        .auto_rollback_deployment(&proj.id, &dep2.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rolled.id, dep1.id);

    // Also test Wasm Worker health check gate rejecting unhealthy WAT component (lacks incoming-handler)
    let broken_wat = r#"(component)"#;
    let wp = h
        .svc
        .create_project(&h.admin, "hc-worker", None)
        .await
        .unwrap();
    let (ap_bad, _, _) = h
        .svc
        .upload_project_artifact(&h.admin, &wp.id, "bad.wat", Bytes::from(broken_wat))
        .await
        .unwrap();
    let bad_dep = h
        .svc
        .create_deployment(
            &h.admin,
            &wp.id,
            CreateDeploymentInput {
                artifact_path: Some(ap_bad),
                target_env: Some(TargetEnv::Preview),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (st_hc_fail, _) = json_req(
        &app,
        Method::POST,
        &format!("/_api/v1/projects/{}/promote", wp.id),
        tok,
        Some(serde_json::json!({ "deployment_id": bad_dep.id, "verify_health": true })),
    )
    .await;
    assert_eq!(st_hc_fail, StatusCode::BAD_REQUEST);

    ha.abort();
    hb.abort();
}

#[tokio::test]
async fn static_site_cache_control_and_streaming_dynamic_proxy() {
    let h = setup_harness().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // 1. Create project & bucket with Cache-Control
    let proj = h
        .svc
        .create_project(&h.admin, "cached-site-app", None)
        .await
        .unwrap();
    let (st_bkt, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/buckets",
        tok,
        Some(serde_json::json!({
            "name": "cached-site",
            "project_id": proj.id,
            "cache_control": "public, max-age=3600"
        })),
    )
    .await;
    assert_eq!(st_bkt, StatusCode::CREATED);

    let put_stream: ferropod::core::model::ByteStream =
        Box::pin(futures_util::stream::once(async {
            Ok::<_, std::io::Error>(Bytes::from_static(b"<html>hello cached</html>"))
        }));
    h.svc
        .put_object_stream(
            &BlobAuth::Caller(&h.admin),
            "cached-site",
            "index.html",
            put_stream,
            "text/html",
            None,
        )
        .await
        .unwrap();

    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                assets_bucket: Some("cached-site".into()),
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Upstream server streaming multi-chunk request/response
    let upstream_router = Router::new().route(
        "/stream/echo",
        any(|req: AxumRequest| async move {
            let body_bytes = req.into_body().collect().await.unwrap().to_bytes();
            let chunks = vec![
                Ok::<_, std::io::Error>(Bytes::from_static(b"head:")),
                Ok::<_, std::io::Error>(body_bytes),
                Ok::<_, std::io::Error>(Bytes::from_static(b":tail")),
            ];
            axum::response::Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain")
                .body(Body::from_stream(futures_util::stream::iter(chunks)))
                .unwrap()
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_addr = listener.local_addr().unwrap();
    let upstream_task = tokio::spawn(async move {
        axum::serve(listener, upstream_router).await.unwrap();
    });

    let (st_rt, _) = json_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        tok,
        Some(serde_json::json!({
            "host": "cached.edge.test",
            "project": proj.id,
            "target_env": "prod",
            "proxy_prefix": "/stream",
            "proxy_upstream": format!("http://{upstream_addr}")
        })),
    )
    .await;
    assert_eq!(st_rt, StatusCode::OK);

    // Static GET emits bucket Cache-Control and ETag
    let r_get = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/index.html")
                .header(header::HOST, "cached.edge.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_get.status(), StatusCode::OK);
    assert_eq!(
        r_get.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );
    let etag = r_get
        .headers()
        .get(header::ETAG)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let get_bytes = r_get.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&get_bytes[..], b"<html>hello cached</html>");

    // Static HEAD & 304 also emit Cache-Control
    let r_head = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri("/index.html")
                .header(header::HOST, "cached.edge.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_head.status(), StatusCode::OK);
    assert_eq!(
        r_head.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );

    let r_304 = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/index.html")
                .header(header::HOST, "cached.edge.test")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_304.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        r_304.headers().get(header::CACHE_CONTROL).unwrap(),
        "public, max-age=3600"
    );

    // Bucket update invalidates cached Cache-Control
    let (st_patch, _) = json_req(
        &app,
        Method::PATCH,
        "/_api/v1/buckets/cached-site",
        tok,
        Some(serde_json::json!({ "cache_control": "no-store" })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);

    let r_after = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/index.html")
                .header(header::HOST, "cached.edge.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_after.status(), StatusCode::OK);
    assert_eq!(
        r_after.headers().get(header::CACHE_CONTROL).unwrap(),
        "no-store"
    );

    // Dynamic reverse proxy streams multi-chunk request & response bodies
    let req_chunks = vec![
        Ok::<_, std::io::Error>(Bytes::from_static(b"part1+")),
        Ok::<_, std::io::Error>(Bytes::from_static(b"part2")),
    ];
    let r_stream = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/stream/echo")
                .header(header::HOST, "cached.edge.test")
                .body(Body::from_stream(futures_util::stream::iter(req_chunks)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_stream.status(), StatusCode::OK);
    let stream_bytes = r_stream.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&stream_bytes[..], b"head:part1+part2:tail");

    upstream_task.abort();
}
