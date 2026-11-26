use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use serde::Deserialize;

use crate::core::service::PlatformService;

mod ai_realtime;
mod blobs;
mod dbs;
mod kv;
mod ops;
mod projects;
mod queues;
mod search;

pub use ai_realtime::{
    ai_chat_completions, ai_embeddings, ai_list_models, realtime_publish, realtime_sse,
};

#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    pub(super) token: Option<String>,
}

pub fn router() -> Router<PlatformService> {
    Router::new()
        .route("/openapi.json", get(ops::api_openapi))
        .route("/llms.txt", get(ops::api_llms_txt))
        .route("/status", get(ops::status))
        .route("/metrics", get(ops::metrics))
        .route("/metrics/prometheus", get(ops::metrics_prometheus))
        .route("/resource-groups", get(ops::list_resource_groups))
        .route("/system/info", get(ops::system_info))
        .route("/system/update/check", get(ops::system_update_check))
        .route("/system/releases", get(ops::system_releases))
        .route("/system/update/apply", post(ops::system_update_apply))
        .route(
            "/settings",
            get(ops::get_platform_settings).patch(ops::update_platform_settings),
        )
        .route("/settings/reset", post(ops::reset_platform_settings))
        .route("/settings/dns-check", post(ops::settings_dns_check))
        .route("/settings/alerts/test", post(ops::settings_test_alert))
        .route(
            "/alerts/webhooks",
            get(ai_realtime::alert_webhooks_list).post(ai_realtime::alert_webhooks_create),
        )
        .route(
            "/alerts/webhooks/{id}",
            patch(ai_realtime::alert_webhooks_update).delete(ai_realtime::alert_webhooks_delete),
        )
        .route("/alerts/webhooks/{id}/test", post(ai_realtime::alert_webhooks_test))
        .route("/realtime/channels", get(ai_realtime::realtime_list_channels))
        .route("/backups", get(ops::list_backups).post(ops::create_backup))
        .route(
            "/backups/schedule",
            get(ops::get_backup_schedule).patch(ops::update_backup_schedule),
        )
        .route("/backups/{name}", delete(ops::delete_backup))
        .route("/backups/{name}/download", get(ops::download_backup))
        .route("/backups/{name}/restore", post(ops::restore_backup))
        .route("/crons", get(projects::cron_list).post(projects::cron_create))
        .route("/crons/{id}", patch(projects::cron_update).delete(projects::cron_delete))
        .route("/crons/{id}/run", post(projects::cron_run))
        .merge(projects::runtime_routes("/apps"))
        .merge(projects::runtime_routes("/runtimes"))
        .merge(projects::runtime_routes("/projects"))
        .route("/routes", get(projects::list_routes).post(projects::upsert_route))
        .route("/routes/{host}", patch(projects::update_route).delete(projects::delete_route))
        .route("/tokens", get(projects::list_tokens).post(projects::create_token))
        .route("/tokens/{id}", delete(projects::revoke_token))
        .route("/kv", get(kv::kv_list_namespaces).post(kv::kv_create_namespace))
        .route(
            "/kv/{ns}",
            get(kv::kv_list)
                .patch(kv::kv_update_namespace)
                .delete(kv::kv_delete_namespace),
        )
        .route("/kv/{ns}/{key}", get(kv::kv_get).put(kv::kv_put).delete(kv::kv_delete))
        .route("/kv/{ns}/{key}/incr", post(kv::kv_incr))
        .route("/queues", get(queues::queue_list).post(queues::queue_create))
        .route("/queues/{name}", patch(queues::queue_update).delete(queues::queue_delete))
        .route("/queues/{name}/enqueue", post(queues::queue_enqueue))
        .route("/queues/{name}/stats", get(queues::queue_stats))
        .route("/queues/{name}/inspect", get(queues::queue_inspect))
        .route("/queues/{name}/dlq/replay", post(queues::queue_dlq_replay))
        .route("/queues/{name}/purge", post(queues::queue_purge))
        .route("/buckets", get(blobs::bucket_list).post(blobs::bucket_create))
        .route(
            "/buckets/{name}",
            patch(blobs::bucket_update).delete(blobs::bucket_delete),
        )
        .route("/buckets/{name}/presign", post(blobs::bucket_presign))
        .route("/buckets/{name}/browse", get(blobs::bucket_browse))
        .route("/buckets/{name}/prefix", delete(blobs::bucket_delete_prefix))
        .route("/dbs", get(dbs::db_list).post(dbs::db_create))
        .route("/dbs/{id}", patch(dbs::db_update).delete(dbs::db_delete))
        .route("/dbs/{id}/schema", get(dbs::db_schema))
        .route("/dbs/{id}/query", post(dbs::db_query))
        .route("/dbs/{id}/execute", post(dbs::db_execute))
        .route("/dbs/{id}/fts", get(dbs::db_fts_list).post(dbs::db_fts_create))
        .route("/dbs/{id}/fts/{name}", delete(dbs::db_fts_drop))
        .route("/dbs/{id}/fts/{name}/query", post(dbs::db_fts_query))
        .route(
            "/search",
            get(search::search_list_indexes).post(search::search_create_index),
        )
        .route(
            "/search/{id}",
            patch(search::search_update_index).delete(search::search_delete_index),
        )
        .route("/search/{id}/commit", post(search::search_commit_index))
        .route(
            "/search/{id}/docs",
            get(search::search_list_docs).post(search::search_upsert_doc),
        )
        .route("/search/{id}/docs/{doc_id}", delete(search::search_delete_doc))
        .route("/search/{id}/query", post(search::search_query_index))
        .route("/logs", get(ops::logs_query))
        .route("/logs/stream", get(ops::logs_stream))
        .route(
            "/ai/providers",
            get(ai_realtime::ai_list_providers).post(ai_realtime::ai_create_provider),
        )
        .route(
            "/ai/providers/{name}",
            patch(ai_realtime::ai_update_provider).delete(ai_realtime::ai_delete_provider),
        )
        .route("/ai/usage", get(ai_realtime::ai_usage_summary))
        .route("/ai/cache/clear", post(ai_realtime::ai_clear_cache))
}

