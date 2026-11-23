use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::routing::post;
use axum::Router;
use bytes::Bytes;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tower::ServiceExt;

use ferropod::config::{Config, Profile};
use ferropod::core::crypto::Crypto;
use ferropod::core::model::{Caller, CronJobOpts, TargetEnv, TokenScope};
use ferropod::core::service::{CreateDeploymentInput, PlatformDeps, PlatformService};
use ferropod::gateway::router::build_router;
use ferropod::gateway::tls::build_dynamic_http_router;
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore,
    log_store::DailyLogStore, queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};

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
    tmp: TempDir,
    svc: PlatformService,
    admin: Caller,
    admin_token: String,
}

async fn setup() -> Harness {
    let tmp = tempfile::tempdir().unwrap();
    let svc = open_service(tmp.path().to_path_buf()).await;
    let admin = Caller::system("test");
    let (_meta, admin_token) = svc
        .create_token(&admin, "test-admin", TokenScope::Admin, None)
        .await
        .unwrap();
    Harness {
        tmp,
        svc,
        admin,
        admin_token,
    }
}

async fn send_req(
    app: &Router,
    method: Method,
    uri: &str,
    host: Option<&str>,
    token: Option<&str>,
    body: Option<serde_json::Value>,
) -> (StatusCode, HeaderMap, Bytes) {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(h) = host {
        builder = builder.header(header::HOST, h);
    }
    if let Some(t) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {t}"));
    }
    let req = match body {
        Some(v) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap(),
        None => builder.body(Body::empty()).unwrap(),
    };
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes)
}

#[tokio::test]
async fn settings_crud_persistence_validation_and_reset() {
    let h = setup().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // 1. GET initial defaults
    let (st, _, body) = send_req(&app, Method::GET, "/_api/v1/settings", None, tok, None).await;
    assert_eq!(st, StatusCode::OK);
    let initial: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(initial["effective"]["trust_proxy_headers"], true);
    assert_eq!(initial["effective"]["has_alert_webhook_secret"], false);
    assert_eq!(initial["runtime"]["profile"], "LowMem");
    assert_eq!(initial["runtime"]["insecure_default_master_key"], false);

    // 2. Validation rejections
    for bad_patch in [
        serde_json::json!({ "panel_domain": "https://bad.example.com" }),
        serde_json::json!({ "base_domain": "apps.example.com:8080" }),
        serde_json::json!({ "log_retention_days": 0 }),
        serde_json::json!({ "wasm_max_memory_mb": 2 }),
        serde_json::json!({ "auto_rollback_error_rate_pct": 101 }),
        serde_json::json!({ "alert_webhook_url": "ftp://hooks.example.com" }),
    ] {
        let (bad_st, _, _) = send_req(
            &app,
            Method::PATCH,
            "/_api/v1/settings",
            None,
            tok,
            Some(bad_patch),
        )
        .await;
        assert_eq!(bad_st, StatusCode::BAD_REQUEST);
    }

    // 3. PATCH valid settings + webhook secret
    let (st_patch, _, body_patch) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        tok,
        Some(serde_json::json!({
            "instance_name": "edge-eu-1",
            "panel_domain": "Panel.Example.COM.",
            "base_domain": "*.Apps.Example.COM",
            "public_scheme": "https",
            "log_retention_days": 14,
            "wasm_max_memory_mb": 64,
            "auto_rollback_min_requests": 4,
            "alert_webhook_url": "https://alerts.example.com/hook",
            "alert_webhook_secret": "super-secret-hmac-key"
        })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);
    let raw_str = String::from_utf8_lossy(&body_patch);
    assert!(!raw_str.contains("super-secret-hmac-key"));
    let patched: serde_json::Value = serde_json::from_slice(&body_patch).unwrap();
    assert_eq!(patched["effective"]["instance_name"], "edge-eu-1");
    assert_eq!(patched["effective"]["panel_domain"], "panel.example.com");
    assert_eq!(patched["effective"]["base_domain"], "apps.example.com");
    assert_eq!(patched["effective"]["public_scheme"], "https");
    assert_eq!(patched["effective"]["log_retention_days"], 14);
    assert_eq!(patched["effective"]["wasm_max_memory_mb"], 64);
    assert_eq!(patched["effective"]["auto_rollback_min_requests"], 4);
    assert_eq!(patched["effective"]["has_alert_webhook_secret"], true);

    // 4. Verify status endpoint exposes instance_name, panel_domain, base_domain
    let (st_status, _, status_body) =
        send_req(&app, Method::GET, "/_api/v1/status", None, tok, None).await;
    assert_eq!(st_status, StatusCode::OK);
    let status_json: serde_json::Value = serde_json::from_slice(&status_body).unwrap();
    assert_eq!(status_json["instance_name"], "edge-eu-1");
    assert_eq!(status_json["panel_domain"], "panel.example.com");
    assert_eq!(status_json["base_domain"], "apps.example.com");

    // 5. Re-open PlatformService from same data_dir to confirm persistence
    h.svc.flush_and_checkpoint().await.unwrap();
    drop(app);
    drop(h.svc);
    let reopened = open_service(h.tmp.path().to_path_buf()).await;
    let reloaded = reopened.get_platform_settings(&h.admin).await.unwrap();
    assert_eq!(
        reloaded.effective.instance_name.as_deref(),
        Some("edge-eu-1")
    );
    assert_eq!(
        reloaded.effective.panel_domain.as_deref(),
        Some("panel.example.com")
    );
    assert_eq!(reloaded.effective.log_retention_days, 14);
    assert!(reloaded.effective.has_alert_webhook_secret);

    // 6. Clear single field via null back to default & test section reset
    let reopened_app = build_router(reopened.clone());
    let (st_clear, _, clear_body) = send_req(
        &reopened_app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        tok,
        Some(serde_json::json!({ "log_retention_days": null })),
    )
    .await;
    assert_eq!(st_clear, StatusCode::OK);
    let cleared: serde_json::Value = serde_json::from_slice(&clear_body).unwrap();
    assert_eq!(
        cleared["effective"]["log_retention_days"],
        cleared["defaults"]["log_retention_days"]
    );
    assert_eq!(cleared["effective"]["wasm_max_memory_mb"], 64);

    let (st_sec_reset, _, sec_reset_body) = send_req(
        &reopened_app,
        Method::POST,
        "/_api/v1/settings/reset",
        None,
        tok,
        Some(serde_json::json!({ "section": "alerts" })),
    )
    .await;
    assert_eq!(st_sec_reset, StatusCode::OK);
    let sec_reset: serde_json::Value = serde_json::from_slice(&sec_reset_body).unwrap();
    assert!(sec_reset["effective"]["alert_webhook_url"].is_null());
    assert_eq!(sec_reset["effective"]["has_alert_webhook_secret"], false);
    assert_eq!(sec_reset["effective"]["wasm_max_memory_mb"], 64);

    // 7. Full reset restores all overrides to defaults
    let (st_full_reset, _, full_reset_body) = send_req(
        &reopened_app,
        Method::POST,
        "/_api/v1/settings/reset",
        None,
        tok,
        None,
    )
    .await;
    assert_eq!(st_full_reset, StatusCode::OK);
    let full_reset: serde_json::Value = serde_json::from_slice(&full_reset_body).unwrap();
    assert!(full_reset["effective"]["instance_name"].is_null());
    assert!(full_reset["effective"]["panel_domain"].is_null());
    assert_eq!(
        full_reset["effective"]["wasm_max_memory_mb"],
        full_reset["defaults"]["wasm_max_memory_mb"]
    );
}

#[tokio::test]
async fn settings_rbac_enforced() {
    let h = setup().await;
    let proj = h
        .svc
        .create_project(&h.admin, "rbac-app", None)
        .await
        .unwrap();
    let (_, ro_tok) = h
        .svc
        .create_token(&h.admin, "ro", TokenScope::Readonly, None)
        .await
        .unwrap();
    let (_, proj_tok) = h
        .svc
        .create_token(&h.admin, "proj", TokenScope::Project(proj.id.clone()), None)
        .await
        .unwrap();

    let app = build_router(h.svc.clone());

    // Readonly can read settings, forbidden on mutations/actions
    let (st, _, _) = send_req(
        &app,
        Method::GET,
        "/_api/v1/settings",
        None,
        Some(&ro_tok),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        Some(&ro_tok),
        Some(serde_json::json!({ "instance_name": "nope" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = send_req(
        &app,
        Method::POST,
        "/_api/v1/settings/reset",
        None,
        Some(&ro_tok),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = send_req(
        &app,
        Method::POST,
        "/_api/v1/settings/dns-check",
        None,
        Some(&ro_tok),
        Some(serde_json::json!({ "domain": "example.com" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = send_req(
        &app,
        Method::POST,
        "/_api/v1/settings/alerts/test",
        None,
        Some(&ro_tok),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Project-scoped token is forbidden from both reading and writing settings
    let (st, _, _) = send_req(
        &app,
        Method::GET,
        "/_api/v1/settings",
        None,
        Some(&proj_tok),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        Some(&proj_tok),
        Some(serde_json::json!({ "instance_name": "nope" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn panel_domain_isolation_and_anti_lockout() {
    let h = setup().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // Anti-lockout rejects enabling restriction from a mismatched non-loopback host without confirmation
    let (st_reject, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        Some("old.example.com"),
        tok,
        Some(serde_json::json!({
            "panel_domain": "panel.example.com",
            "restrict_panel_to_domain": true
        })),
    )
    .await;
    assert_eq!(st_reject, StatusCode::BAD_REQUEST);

    // Explicit confirm_lockout_risk=true allows saving
    let (st_ok, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        Some("old.example.com"),
        tok,
        Some(serde_json::json!({
            "panel_domain": "panel.example.com",
            "restrict_panel_to_domain": true,
            "confirm_lockout_risk": true
        })),
    )
    .await;
    assert_eq!(st_ok, StatusCode::OK);

    // Foreign host receives 404 on /_panel, /_api/v1/status, and /_mcp
    for path in ["/_panel", "/_api/v1/status", "/_mcp"] {
        let method = if path == "/_mcp" {
            Method::POST
        } else {
            Method::GET
        };
        let (st, _, _) = send_req(&app, method, path, Some("foreign.example.com"), tok, None).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "path {path} should be hidden");
    }

    // /_healthz stays reachable on any host
    let (st_hz, _, _) = send_req(
        &app,
        Method::GET,
        "/_healthz",
        Some("foreign.example.com"),
        None,
        None,
    )
    .await;
    assert_eq!(st_hz, StatusCode::OK);

    // Matching panel_domain (with optional port) and loopback hosts remain accessible
    for allowed_host in [
        "panel.example.com",
        "panel.example.com:8443",
        "localhost:8080",
        "127.0.0.1:8080",
    ] {
        let (st_api, _, _) = send_req(
            &app,
            Method::GET,
            "/_api/v1/status",
            Some(allowed_host),
            tok,
            None,
        )
        .await;
        assert_eq!(st_api, StatusCode::OK, "host {allowed_host} should work");

        let (st_ui, _, _) =
            send_req(&app, Method::GET, "/_panel", Some(allowed_host), None, None).await;
        assert_eq!(st_ui, StatusCode::OK);
    }
}

#[tokio::test]
async fn trust_proxy_headers_hot_toggle_controls_rate_limit_keying() {
    let h = setup().await;
    let proj = h
        .svc
        .create_project(&h.admin, "rl-app", None)
        .await
        .unwrap();
    h.svc
        .create_deployment(
            &h.admin,
            &proj.id,
            CreateDeploymentInput {
                target_env: Some(TargetEnv::Prod),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // Create route with rate_limit_rpm = 1
    let (st_rt, _, _) = send_req(
        &app,
        Method::POST,
        "/_api/v1/routes",
        None,
        tok,
        Some(serde_json::json!({
            "host": "rl.example.test",
            "project": proj.id,
            "target_env": "prod",
            "rate_limit_rpm": 1
        })),
    )
    .await;
    assert_eq!(st_rt, StatusCode::OK);

    // 1. Default trust_proxy_headers=true: distinct X-Forwarded-For IPs have separate buckets
    let (st1, _, _) = send_req_with_xff(&app, "rl.example.test", "198.51.100.1").await;
    let (st2, _, _) = send_req_with_xff(&app, "rl.example.test", "198.51.100.2").await;
    // Neither has an active artifact/bucket so route falls through to 404 (not 429)
    assert_ne!(st1, StatusCode::TOO_MANY_REQUESTS);
    assert_ne!(st2, StatusCode::TOO_MANY_REQUESTS);

    // 2. Disable trust_proxy_headers at runtime: spoofed X-Forwarded-For shares fallback bucket -> 429
    let (st_patch, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        tok,
        Some(serde_json::json!({ "trust_proxy_headers": false })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);

    let (st3, _, _) = send_req_with_xff(&app, "rl.example.test", "203.0.113.10").await;
    let (st4, _, _) = send_req_with_xff(&app, "rl.example.test", "203.0.113.11").await;
    assert_ne!(st3, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(st4, StatusCode::TOO_MANY_REQUESTS);
}

async fn send_req_with_xff(app: &Router, host: &str, xff: &str) -> (StatusCode, HeaderMap, Bytes) {
    let req = Request::builder()
        .method(Method::GET)
        .uri("/")
        .header(header::HOST, host)
        .header("x-forwarded-for", xff)
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let st = resp.status();
    let hdr = resp.headers().clone();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    (st, hdr, body)
}

#[tokio::test]
async fn custom_auto_rollback_policy_and_signed_webhook_alerts() {
    let h = setup().await;

    // 1. Verify custom auto-rollback policy parameters
    h.svc.health_tracker().mark_promoted("dep_custom");
    assert!(!h.svc.health_tracker().record_outcome_with_policy(
        "dep_custom",
        true,
        true,
        300,
        2,
        50
    ));
    assert!(h.svc.health_tracker().record_outcome_with_policy(
        "dep_custom",
        true,
        true,
        300,
        2,
        50
    ));
    h.svc.health_tracker().mark_promoted("dep_disabled");
    assert!(!h.svc.health_tracker().record_outcome_with_policy(
        "dep_disabled",
        true,
        false,
        300,
        1,
        1
    ));

    // 2. Start local webhook receiver verifying HMAC signature + payload
    let (tx, mut rx) = mpsc::unbounded_channel::<(HeaderMap, Bytes)>();
    let hook_app = Router::new().route(
        "/hook",
        post(move |headers: HeaderMap, body: Bytes| {
            let tx = tx.clone();
            async move {
                let _ = tx.send((headers, body));
                StatusCode::OK
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hook_addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        let _ = axum::serve(listener, hook_app).await;
    });

    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());
    let hook_url = format!("http://{hook_addr}/hook");
    let secret = "webhook-hmac-secret-123";

    let (st_patch, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        tok,
        Some(serde_json::json!({
            "instance_name": "prod-sg",
            "alert_webhook_url": hook_url,
            "alert_webhook_secret": secret,
            "alert_on_cron_failure": true
        })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);

    // 3. Test Alert endpoint
    let (st_test, _, body_test) = send_req(
        &app,
        Method::POST,
        "/_api/v1/settings/alerts/test",
        None,
        tok,
        None,
    )
    .await;
    assert_eq!(st_test, StatusCode::OK);
    let res_test: serde_json::Value = serde_json::from_slice(&body_test).unwrap();
    assert_eq!(res_test["ok"], true);
    assert_eq!(res_test["status_code"], 200);

    let (hdr1, raw1) = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hdr1.get("x-ferropod-event").unwrap(), "test");
    let expected_sig1 = format!(
        "sha256={}",
        Crypto::hmac_sha256_hex(secret.as_bytes(), &raw1)
    );
    assert_eq!(hdr1.get("x-ferropod-signature").unwrap(), &expected_sig1);
    let json1: serde_json::Value = serde_json::from_slice(&raw1).unwrap();
    assert_eq!(json1["event"], "test");
    assert_eq!(json1["instance"], "prod-sg");

    // 4. Real cron failure triggers async signed webhook alert
    let proj = h
        .svc
        .create_project(&h.admin, "cron-alert-app", None)
        .await
        .unwrap();
    let job = h
        .svc
        .create_cron_job_with_opts(
            &h.admin,
            "failing-job",
            CronJobOpts {
                project_id: Some(proj.id.clone()),
                schedule: Some("@hourly".into()),
                target: Some("/tick".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let _ = h.svc.trigger_cron_job(&h.admin, &job.id).await.unwrap();

    let (hdr2, raw2) = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(hdr2.get("x-ferropod-event").unwrap(), "cron_failure");
    let expected_sig2 = format!(
        "sha256={}",
        Crypto::hmac_sha256_hex(secret.as_bytes(), &raw2)
    );
    assert_eq!(hdr2.get("x-ferropod-signature").unwrap(), &expected_sig2);
    let json2: serde_json::Value = serde_json::from_slice(&raw2).unwrap();
    assert_eq!(json2["event"], "cron_failure");
    assert_eq!(json2["detail"]["name"], "failing-job");

    srv.abort();
}

#[tokio::test]
async fn dynamic_https_redirect_tls_domains_and_embedded_settings_asset() {
    let h = setup().await;
    let app = build_router(h.svc.clone());
    let tok = Some(h.admin_token.as_str());

    // 1. Dynamic HTTP -> HTTPS redirect router reflects hot setting updates without restarting
    let dyn_http = build_dynamic_http_router(h.svc.clone(), app.clone(), 443);

    let (st_off, _, _) = send_req(
        &dyn_http,
        Method::GET,
        "/_panel",
        Some("panel.example.com"),
        None,
        None,
    )
    .await;
    assert_eq!(st_off, StatusCode::OK);

    let (st_patch, _, _) = send_req(
        &app,
        Method::PATCH,
        "/_api/v1/settings",
        None,
        tok,
        Some(serde_json::json!({
            "panel_domain": "panel.example.com",
            "extra_tls_domains": ["api.example.com", "localhost", "127.0.0.1"],
            "http_redirect_https": true
        })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK);

    let (st_on, hdr_on, _) = send_req(
        &dyn_http,
        Method::GET,
        "/_panel",
        Some("panel.example.com"),
        None,
        None,
    )
    .await;
    assert_eq!(st_on, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        hdr_on.get(header::LOCATION).unwrap(),
        "https://panel.example.com/_panel"
    );

    // /_healthz stays 200 even when redirect is enabled
    let (st_hz, _, _) = send_req(
        &dyn_http,
        Method::GET,
        "/_healthz",
        Some("panel.example.com"),
        None,
        None,
    )
    .await;
    assert_eq!(st_hz, StatusCode::OK);

    // 2. effective_tls_domains merges panel_domain, extra_tls_domains, and routes while skipping localhost/IPs
    let proj = h
        .svc
        .create_project(&h.admin, "tls-app", None)
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "blog.example.com", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();
    h.svc
        .upsert_route(&h.admin, "127.0.0.1", &proj.id, TargetEnv::Prod)
        .await
        .unwrap();
    let domains = h.svc.effective_tls_domains().await;
    assert_eq!(
        domains,
        vec![
            "api.example.com".to_string(),
            "blog.example.com".to_string(),
            "panel.example.com".to_string(),
        ]
    );

    // 3. Embedded settings_component.js asset serves 200 with ETag and 304 on If-None-Match
    let (st_js, hdr_js, body_js) = send_req(
        &app,
        Method::GET,
        "/_panel/assets/settings_component.js",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(st_js, StatusCode::OK);
    assert!(String::from_utf8_lossy(&body_js).contains("fp-platform-settings"));
    let etag = hdr_js.get(header::ETAG).unwrap().to_str().unwrap();

    let req_304 = Request::builder()
        .method(Method::GET)
        .uri("/_panel/assets/settings_component.js")
        .header(header::IF_NONE_MATCH, etag)
        .body(Body::empty())
        .unwrap();
    let resp_304 = app.clone().oneshot(req_304).await.unwrap();
    assert_eq!(resp_304.status(), StatusCode::NOT_MODIFIED);
}
