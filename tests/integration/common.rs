#![allow(dead_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use bytes::Bytes;
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

use ferropod::config::{Config, Profile};
use ferropod::core::model::{Caller, TokenScope};
use ferropod::core::service::{PlatformDeps, PlatformService};
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore,
    log_store::DailyLogStore, queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};

pub struct TestHarness {
    pub _tmp: TempDir,
    pub svc: PlatformService,
    pub kv: Arc<TursoKvStore>,
    pub queue: Arc<TursoQueueStore>,
    pub user_dbs: Arc<TursoUserDbManager>,
    pub blobs: Arc<FsBlobStore>,
    pub logs: Arc<DailyLogStore>,
    pub admin: Caller,
    pub admin_token: String,
}

pub async fn setup_harness() -> TestHarness {
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

pub async fn setup_harness_with(mutate_cfg: impl FnOnce(&mut Config)) -> TestHarness {
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

pub async fn json_req(
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
