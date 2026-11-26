use std::collections::HashMap;

use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use wasmtime::StoreLimits;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

use crate::core::model::{
    BlobAuth, ByteStream, Caller, EnqueueOpts, LogEntry, RequestCtx, SearchQueryInput, SqlValue,
};
use crate::core::service::PlatformService;

use super::outbound::FerroHttpHooks;
use super::{wit_ai, wit_bucket, wit_db, wit_env, wit_kv, wit_log, wit_queue, wit_search};

pub(super) struct HostState {
    pub(super) wasi: WasiCtx,
    pub(super) http: WasiHttpCtx,
    pub(super) hooks: FerroHttpHooks,
    pub(super) table: ResourceTable,
    pub(super) limits: StoreLimits,
    pub(super) svc: PlatformService,
    pub(super) ctx: RequestCtx,
    pub(super) caller: Caller,
    pub(super) hostcalls_used: u32,
    pub(super) max_hostcalls: u32,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.hooks,
        }
    }
}

impl HostState {
    fn resolve_kv<'a>(&'a self, binding: &str) -> std::result::Result<&'a str, String> {
        self.ctx
            .deployment
            .bindings
            .kv
            .get(binding)
            .map(String::as_str)
            .ok_or_else(|| format!("kv binding '{binding}' is not configured"))
    }

    fn resolve_db<'a>(&'a self, binding: &str) -> std::result::Result<&'a str, String> {
        self.ctx
            .deployment
            .bindings
            .db
            .get(binding)
            .map(String::as_str)
            .ok_or_else(|| format!("db binding '{binding}' is not configured"))
    }

    fn resolve_queue<'a>(&'a self, binding: &str) -> std::result::Result<&'a str, String> {
        self.ctx
            .deployment
            .bindings
            .queue
            .get(binding)
            .map(String::as_str)
            .ok_or_else(|| format!("queue binding '{binding}' is not configured"))
    }

    fn resolve_bucket<'a>(&'a self, binding: &str) -> std::result::Result<&'a str, String> {
        self.ctx
            .deployment
            .bindings
            .bucket
            .get(binding)
            .map(String::as_str)
            .ok_or_else(|| format!("bucket binding '{binding}' is not configured"))
    }

    fn resolve_search<'a>(&'a self, binding: &str) -> std::result::Result<&'a str, String> {
        self.ctx
            .deployment
            .bindings
            .search
            .get(binding)
            .map(String::as_str)
            .ok_or_else(|| format!("search binding '{binding}' is not configured"))
    }

    fn consume_hostcall(&mut self, op: &str) -> std::result::Result<(), String> {
        if self.hostcalls_used >= self.max_hostcalls {
            return Err(format!(
                "worker exceeded max hostcall budget ({}) during '{op}'",
                self.max_hostcalls
            ));
        }
        self.hostcalls_used += 1;
        Ok(())
    }
}

impl wit_log::Host for HostState {
    async fn emit(&mut self, level: wit_log::Level, message: String, meta_json: Option<String>) {
        let lvl = match level {
            wit_log::Level::Trace => "trace",
            wit_log::Level::Debug => "debug",
            wit_log::Level::Info => "info",
            wit_log::Level::Warn => "warn",
            wit_log::Level::Error => "error",
        };
        self.svc.emit_log(LogEntry {
            id: None,
            ts: Utc::now().timestamp_millis(),
            project_id: Some(self.ctx.project_id.clone()),
            request_id: Some(self.ctx.request_id.clone()),
            source: "runtime".into(),
            level: lvl.into(),
            message,
            meta_json,
        });
    }
}

impl wit_kv::Host for HostState {
    async fn get(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<Vec<u8>>, String> {
        self.consume_hostcall("kv.get")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_get(&self.caller, ns, &key)
            .await
            .map(|opt| opt.map(|v| v.value))
            .map_err(|e| e.to_string())
    }

    async fn get_entry(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<wit_kv::Entry>, String> {
        self.consume_hostcall("kv.get_entry")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_get(&self.caller, ns, &key)
            .await
            .map(|opt| {
                opt.map(|v| wit_kv::Entry {
                    value: v.value,
                    metadata: v.metadata,
                    expires_at: v.expires_at,
                    updated_at: v.updated_at,
                })
            })
            .map_err(|e| e.to_string())
    }

    async fn put(
        &mut self,
        binding: String,
        key: String,
        value: Vec<u8>,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
        ephemeral: bool,
    ) -> std::result::Result<(), String> {
        self.consume_hostcall("kv.put")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_put(
                &self.caller,
                ns,
                &key,
                Bytes::from(value),
                metadata,
                ttl_secs,
                ephemeral,
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn put_if_absent(
        &mut self,
        binding: String,
        key: String,
        value: Vec<u8>,
        metadata: Option<String>,
        ttl_secs: Option<u64>,
    ) -> std::result::Result<bool, String> {
        self.consume_hostcall("kv.put_if_absent")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_put_if_absent(
                &self.caller,
                ns,
                &key,
                Bytes::from(value),
                metadata,
                ttl_secs,
            )
            .await
            .map_err(|e| e.to_string())
    }

    async fn incr(
        &mut self,
        binding: String,
        key: String,
        delta: i64,
        ttl_secs: Option<u64>,
    ) -> std::result::Result<i64, String> {
        self.consume_hostcall("kv.incr")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_incr(&self.caller, ns, &key, delta, ttl_secs)
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete(&mut self, binding: String, key: String) -> std::result::Result<(), String> {
        self.consume_hostcall("kv.delete")?;
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_delete(&self.caller, ns, &key)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list(
        &mut self,
        binding: String,
        prefix: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> std::result::Result<wit_kv::ListPage, String> {
        self.consume_hostcall("kv.list")?;
        let ns = self.resolve_kv(&binding)?;
        let (keys, next_cursor) = self
            .svc
            .kv_list_page(
                &self.caller,
                ns,
                prefix.as_deref(),
                cursor.as_deref(),
                limit as usize,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_kv::ListPage {
            keys: keys
                .into_iter()
                .map(|k| wit_kv::KeyMeta {
                    key: k.key,
                    size: k.size as u64,
                    metadata: k.metadata,
                    expires_at: k.expires_at,
                    updated_at: k.updated_at,
                })
                .collect(),
            cursor: next_cursor,
        })
    }
}

fn map_wit_sql_param(v: wit_db::SqlValue) -> SqlValue {
    match v {
        wit_db::SqlValue::Null => SqlValue::Null,
        wit_db::SqlValue::Boolean(b) => SqlValue::Bool(b),
        wit_db::SqlValue::Integer(i) => SqlValue::Integer(i),
        wit_db::SqlValue::Real(f) => SqlValue::Real(f),
        wit_db::SqlValue::Text(s) => SqlValue::Text(s),
        wit_db::SqlValue::Blob(b) => SqlValue::Blob(b),
    }
}

fn map_json_to_wit_sql(v: serde_json::Value) -> wit_db::SqlValue {
    match v {
        serde_json::Value::Null => wit_db::SqlValue::Null,
        serde_json::Value::Bool(b) => wit_db::SqlValue::Boolean(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                wit_db::SqlValue::Integer(i)
            } else if let Some(u) = n.as_u64() {
                wit_db::SqlValue::Integer(i64::try_from(u).unwrap_or(i64::MAX))
            } else {
                wit_db::SqlValue::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => wit_db::SqlValue::Text(s),
        other => wit_db::SqlValue::Text(other.to_string()),
    }
}

impl wit_db::Host for HostState {
    async fn query(
        &mut self,
        binding: String,
        sql: String,
        params: Vec<wit_db::SqlValue>,
    ) -> std::result::Result<wit_db::QueryResult, String> {
        self.consume_hostcall("db.query")?;
        let db_id = self.resolve_db(&binding)?;
        let mapped: Vec<SqlValue> = params.into_iter().map(map_wit_sql_param).collect();
        let qr = self
            .svc
            .query_user_db(&self.caller, db_id, &sql, mapped)
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_db::QueryResult {
            columns: qr.columns,
            rows: qr
                .rows
                .into_iter()
                .map(|r| wit_db::Row {
                    values: r.into_iter().map(map_json_to_wit_sql).collect(),
                })
                .collect(),
        })
    }

    async fn execute(
        &mut self,
        binding: String,
        sql: String,
        params: Vec<wit_db::SqlValue>,
    ) -> std::result::Result<u64, String> {
        self.consume_hostcall("db.execute")?;
        let db_id = self.resolve_db(&binding)?;
        let mapped: Vec<SqlValue> = params.into_iter().map(map_wit_sql_param).collect();
        self.svc
            .execute_user_db_batch(&self.caller, db_id, vec![(sql, mapped)])
            .await
            .map_err(|e| e.to_string())
    }
}

impl wit_queue::Host for HostState {
    async fn enqueue(
        &mut self,
        binding: String,
        payload: Vec<u8>,
        options: Option<wit_queue::EnqueueOptions>,
    ) -> std::result::Result<i64, String> {
        self.consume_hostcall("queue.enqueue")?;
        let q_name = self.resolve_queue(&binding)?;
        let opts = match options {
            Some(o) => EnqueueOpts {
                delay_secs: o.delay_secs,
                priority: o.priority,
                dedup_key: o.dedup_key,
                max_attempts: o.max_attempts,
            },
            None => EnqueueOpts::default(),
        };
        self.svc
            .queue_enqueue(&self.caller, q_name, Bytes::from(payload), opts)
            .await
            .map_err(|e| e.to_string())
    }
}

fn to_wit_object_meta(m: crate::core::model::ObjectMeta) -> wit_bucket::ObjectMeta {
    wit_bucket::ObjectMeta {
        bucket: m.bucket,
        key: m.key,
        sha256: m.sha256,
        size: m.size,
        content_type: m.content_type,
        custom_meta: m.custom_meta,
        updated_at: m.updated_at,
    }
}

impl wit_bucket::Host for HostState {
    async fn get(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<Vec<u8>>, String> {
        self.consume_hostcall("bucket.get")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        let Some((meta, mut stream, _len, _)) = self
            .svc
            .get_object_stream(&auth, bucket_name, &key, None)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let max_bytes = self.svc.config().wasm_max_memory_bytes;
        if usize::try_from(meta.size).unwrap_or(usize::MAX) > max_bytes {
            return Err("bucket object exceeds worker memory limit".into());
        }
        let mut buf = Vec::with_capacity(meta.size as usize);
        while let Some(chunk) = stream.try_next().await.map_err(|e| e.to_string())? {
            if buf.len().saturating_add(chunk.len()) > max_bytes {
                return Err("bucket object exceeds worker memory limit".into());
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(Some(buf))
    }

    async fn get_object(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<wit_bucket::ObjectData>, String> {
        self.consume_hostcall("bucket.get_object")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        let Some((meta, mut stream, _len, _)) = self
            .svc
            .get_object_stream(&auth, bucket_name, &key, None)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let max_bytes = self.svc.config().wasm_max_memory_bytes;
        if usize::try_from(meta.size).unwrap_or(usize::MAX) > max_bytes {
            return Err("bucket object exceeds worker memory limit".into());
        }
        let mut buf = Vec::with_capacity(meta.size as usize);
        while let Some(chunk) = stream.try_next().await.map_err(|e| e.to_string())? {
            if buf.len().saturating_add(chunk.len()) > max_bytes {
                return Err("bucket object exceeds worker memory limit".into());
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(Some(wit_bucket::ObjectData {
            meta: to_wit_object_meta(meta),
            body: buf,
        }))
    }

    async fn head(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<wit_bucket::ObjectMeta>, String> {
        self.consume_hostcall("bucket.head")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        self.svc
            .head_object(&auth, bucket_name, &key)
            .await
            .map(|opt| opt.map(to_wit_object_meta))
            .map_err(|e| e.to_string())
    }

    async fn put(
        &mut self,
        binding: String,
        key: String,
        body: Vec<u8>,
        content_type: Option<String>,
        custom_meta: Option<String>,
    ) -> std::result::Result<wit_bucket::ObjectMeta, String> {
        self.consume_hostcall("bucket.put")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        let ct = content_type
            .as_deref()
            .unwrap_or("application/octet-stream");
        let stream: ByteStream = Box::pin(futures_util::stream::once(async move {
            Ok::<_, std::io::Error>(Bytes::from(body))
        }));
        self.svc
            .put_object_stream(&auth, bucket_name, &key, stream, ct, custom_meta)
            .await
            .map(to_wit_object_meta)
            .map_err(|e| e.to_string())
    }

    async fn delete(&mut self, binding: String, key: String) -> std::result::Result<bool, String> {
        self.consume_hostcall("bucket.delete")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        self.svc
            .delete_object(&auth, bucket_name, &key)
            .await
            .map_err(|e| e.to_string())
    }

    async fn list(
        &mut self,
        binding: String,
        prefix: Option<String>,
        cursor: Option<String>,
        limit: u32,
    ) -> std::result::Result<wit_bucket::ListPage, String> {
        self.consume_hostcall("bucket.list")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        let (items, next_cursor) = self
            .svc
            .list_objects_page(
                &auth,
                bucket_name,
                prefix.as_deref(),
                cursor.as_deref(),
                limit as usize,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_bucket::ListPage {
            objects: items.into_iter().map(to_wit_object_meta).collect(),
            cursor: next_cursor,
        })
    }

    async fn presign_url(
        &mut self,
        binding: String,
        key: String,
        method: String,
        expires_secs: u64,
    ) -> std::result::Result<String, String> {
        self.consume_hostcall("bucket.presign_url")?;
        let bucket_name = self.resolve_bucket(&binding)?;
        let (url, _exp) = self
            .svc
            .presign_blob_url(&self.caller, &method, bucket_name, &key, expires_secs)
            .await
            .map_err(|e| e.to_string())?;
        Ok(url)
    }
}

impl wit_env::Host for HostState {
    async fn get(&mut self, key: String) -> Option<String> {
        self.ctx.deployment.bindings.env.get(&key).cloned()
    }

    async fn secret(&mut self, key: String) -> Option<String> {
        self.svc
            .get_secret(&self.caller, &self.ctx.project_id, &key)
            .await
            .ok()
            .flatten()
    }
}

impl wit_search::Host for HostState {
    async fn query(
        &mut self,
        binding: String,
        query: String,
        fuzzy: bool,
        limit: u32,
    ) -> std::result::Result<wit_search::SearchResult, String> {
        self.consume_hostcall("search.query")?;
        let index_ref = self.resolve_search(&binding)?;
        let res = self
            .svc
            .query_search_index(
                &self.caller,
                index_ref,
                SearchQueryInput {
                    query,
                    fuzzy,
                    sort: None,
                    limit: Some(limit as usize),
                    facets: HashMap::new(),
                    vector: None,
                    alpha: None,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_search::SearchResult {
            hits: res
                .hits
                .into_iter()
                .map(|h| {
                    let id = h
                        .doc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_default();
                    wit_search::Hit {
                        id,
                        score: h.score,
                        doc_json: h.doc.to_string(),
                    }
                })
                .collect(),
            total: res.total as u32,
            took_ms: res.took_ms,
        })
    }

    async fn vector_query(
        &mut self,
        binding: String,
        query: String,
        vector: Vec<f32>,
        alpha: f64,
        limit: u32,
    ) -> std::result::Result<wit_search::SearchResult, String> {
        self.consume_hostcall("search.vector_query")?;
        let index_ref = self.resolve_search(&binding)?;
        let res = self
            .svc
            .query_search_index(
                &self.caller,
                index_ref,
                SearchQueryInput {
                    query,
                    fuzzy: true,
                    sort: None,
                    limit: Some(limit as usize),
                    facets: HashMap::new(),
                    vector: Some(vector),
                    alpha: Some(alpha),
                },
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_search::SearchResult {
            hits: res
                .hits
                .into_iter()
                .map(|h| {
                    let id = h
                        .doc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_default();
                    wit_search::Hit {
                        id,
                        score: h.score,
                        doc_json: h.doc.to_string(),
                    }
                })
                .collect(),
            total: res.total as u32,
            took_ms: res.took_ms,
        })
    }

    async fn upsert(
        &mut self,
        binding: String,
        doc_json: String,
    ) -> std::result::Result<String, String> {
        self.consume_hostcall("search.upsert")?;
        let index_ref = self.resolve_search(&binding)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&doc_json).map_err(|e| format!("invalid doc json: {e}"))?;
        let saved = self
            .svc
            .upsert_search_doc(&self.caller, index_ref, parsed)
            .await
            .map_err(|e| e.to_string())?;
        Ok(saved
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_default())
    }

    async fn delete(
        &mut self,
        binding: String,
        doc_id: String,
    ) -> std::result::Result<bool, String> {
        self.consume_hostcall("search.delete")?;
        let index_ref = self.resolve_search(&binding)?;
        self.svc
            .delete_search_doc(&self.caller, index_ref, &doc_id)
            .await
            .map_err(|e| e.to_string())
    }
}

impl wit_ai::Host for HostState {
    async fn chat_completions(
        &mut self,
        provider: Option<String>,
        body_json: String,
    ) -> std::result::Result<wit_ai::ChatResponse, String> {
        self.consume_hostcall("ai.chat_completions")?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body_json).map_err(|e| format!("invalid json body: {e}"))?;
        let res = self
            .svc
            .ai_proxy(
                &self.caller,
                provider.as_deref(),
                "chat/completions",
                parsed,
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_ai::ChatResponse {
            status: res.status,
            provider: res.provider_name,
            cached: res.cached,
            body_json: res.body.to_string(),
        })
    }

    async fn embeddings(
        &mut self,
        provider: Option<String>,
        body_json: String,
    ) -> std::result::Result<wit_ai::ChatResponse, String> {
        self.consume_hostcall("ai.embeddings")?;
        let parsed: serde_json::Value =
            serde_json::from_str(&body_json).map_err(|e| format!("invalid json body: {e}"))?;
        let res = self
            .svc
            .ai_proxy(&self.caller, provider.as_deref(), "embeddings", parsed)
            .await
            .map_err(|e| e.to_string())?;
        Ok(wit_ai::ChatResponse {
            status: res.status,
            provider: res.provider_name,
            cached: res.cached,
            body_json: res.body.to_string(),
        })
    }
}
