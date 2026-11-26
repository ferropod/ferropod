use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::anyhow;
use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use http::{header, HeaderMap, HeaderName, HeaderValue, Method, Uri};
use http_body_util::{BodyExt, LengthLimitError, Limited, StreamBody};
use hyper::body::Frame;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{
    Config as WasmConfig, Engine, OptLevel, Precompiled, Store, StoreLimitsBuilder, Trap,
};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{ResourceTable, WasiCtx};
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::bindings::ProxyPre;
use wasmtime_wasi_http::{WasiBody, WasiHttpCtx, WasiHttpView};

use crate::config::Config;
use crate::core::model::{
    ByteStream, Caller, Deployment, FerroError, LogEntry, RequestCtx, Result, WorkerHttpRequest,
    WorkerHttpResponse,
};
use crate::core::service::PlatformService;

mod host_bindings;
mod invoke;
mod outbound;

use host_bindings::HostState;
use outbound::FerroHttpHooks;

pub(super) mod host_wit {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "ferropod:platform/host-imports",
        imports: { default: async },
        require_store_data_send: true,
    });
}

pub(super) mod queue_wit {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "ferropod:platform/queue-worker",
        imports: { default: async },
        exports: { default: async },
        with: {
            "ferropod:platform/log": super::host_wit::ferropod::platform::log,
            "ferropod:platform/kv": super::host_wit::ferropod::platform::kv,
            "ferropod:platform/db": super::host_wit::ferropod::platform::db,
            "ferropod:platform/queue": super::host_wit::ferropod::platform::queue,
            "ferropod:platform/bucket": super::host_wit::ferropod::platform::bucket,
            "ferropod:platform/search": super::host_wit::ferropod::platform::search,
            "ferropod:platform/ai": super::host_wit::ferropod::platform::ai,
            "ferropod:platform/env": super::host_wit::ferropod::platform::env,
        },
        require_store_data_send: true,
    });
}

pub(super) mod scheduled_wit {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "ferropod:platform/scheduled-worker",
        imports: { default: async },
        exports: { default: async },
        with: {
            "ferropod:platform/log": super::host_wit::ferropod::platform::log,
            "ferropod:platform/kv": super::host_wit::ferropod::platform::kv,
            "ferropod:platform/db": super::host_wit::ferropod::platform::db,
            "ferropod:platform/queue": super::host_wit::ferropod::platform::queue,
            "ferropod:platform/bucket": super::host_wit::ferropod::platform::bucket,
            "ferropod:platform/search": super::host_wit::ferropod::platform::search,
            "ferropod:platform/ai": super::host_wit::ferropod::platform::ai,
            "ferropod:platform/env": super::host_wit::ferropod::platform::env,
        },
        require_store_data_send: true,
    });
}

pub(super) use host_wit::ferropod::platform::{
    ai as wit_ai, bucket as wit_bucket, db as wit_db, env as wit_env, kv as wit_kv, log as wit_log,
    queue as wit_queue, search as wit_search,
};
pub(super) use queue_wit::exports::ferropod::platform::queue_consumer as wit_qc;
pub(super) use scheduled_wit::exports::ferropod::platform::scheduled as wit_sched;

const EPOCH_TICK_MS: u64 = 100;
const PIPE_CAPACITY_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_HOSTCALLS_PER_INVOCATION: u32 = 500;
const DEFAULT_MAX_OUTBOUND_HTTP_PER_INVOCATION: u32 = 50;

pub(super) struct CachedComponent {
    pub(super) component: Component,
    pub(super) artifact_key: Option<String>,
    pub(super) last_used: u64,
}

/// Wasmtime component runtime executing Workers (`.wasm`/`.wat`/`.cwasm`) with bounded memory,
/// Cranelift compile concurrency caps, epoch interruption, and WIT host bindings.
pub struct WasmtimeRuntime {
    engine: Engine,
    linker: Linker<HostState>,
    pub(super) config: Config,
    svc: OnceLock<PlatformService>,
    compile_sem: Arc<Semaphore>,
    pub(super) cache: Mutex<HashMap<String, CachedComponent>>,
    seq: AtomicU64,
    stop_epoch: Arc<AtomicBool>,
}

impl Drop for WasmtimeRuntime {
    fn drop(&mut self) {
        self.stop_epoch.store(true, Ordering::Relaxed);
    }
}

impl WasmtimeRuntime {
    pub fn new(config: Config) -> Result<Self> {
        let mut wasm_cfg = WasmConfig::new();
        wasm_cfg.wasm_component_model(true);
        wasm_cfg.epoch_interruption(true);
        wasm_cfg.cranelift_opt_level(OptLevel::SpeedAndSize);
        wasm_cfg.parallel_compilation(config.wasm_compile_concurrency > 1);

        let engine = Engine::new(&wasm_cfg).map_err(|e| {
            FerroError::Internal(anyhow!("failed to initialize wasmtime engine: {e:#}"))
        })?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| FerroError::Internal(anyhow!("failed to link wasi p2: {e:#}")))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .map_err(|e| FerroError::Internal(anyhow!("failed to link wasi http: {e:#}")))?;
        host_wit::HostImports::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)
            .map_err(|e| {
            FerroError::Internal(anyhow!("failed to link ferropod host imports: {e:#}"))
        })?;

        let stop_epoch = Arc::new(AtomicBool::new(false));
        let epoch_engine = engine.clone();
        let epoch_stop = stop_epoch.clone();
        std::thread::Builder::new()
            .name("ferropod-wasm-epoch".into())
            .spawn(move || {
                while !epoch_stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                    if epoch_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    epoch_engine.increment_epoch();
                }
            })
            .map_err(|e| FerroError::Internal(anyhow!("failed to spawn epoch thread: {e}")))?;

        Ok(Self {
            engine,
            linker,
            compile_sem: Arc::new(Semaphore::new(config.wasm_compile_concurrency.max(1))),
            cache: Mutex::new(HashMap::new()),
            seq: AtomicU64::new(1),
            config,
            svc: OnceLock::new(),
            stop_epoch,
        })
    }

    pub fn bind_service(&self, svc: PlatformService) {
        let _ = self.svc.set(svc);
    }

    fn service(&self) -> Result<PlatformService> {
        self.svc
            .get()
            .cloned()
            .ok_or_else(|| FerroError::Internal(anyhow!("wasmtime runtime service not bound")))
    }

    pub(super) fn sanitize_rel_path(rel_path: &str) -> Result<String> {
        let trimmed = rel_path.trim().trim_start_matches('/');
        if trimmed.is_empty() || trimmed.contains('\0') {
            return Err(FerroError::InvalidArgument(
                "artifact path cannot be empty".into(),
            ));
        }
        let mut parts = Vec::new();
        for seg in trimmed.split('/') {
            if seg.is_empty() || seg == "." || seg == ".." || seg.contains('\\') {
                return Err(FerroError::InvalidArgument(format!(
                    "invalid artifact path '{rel_path}'"
                )));
            }
            parts.push(seg);
        }
        Ok(parts.join("/"))
    }

    pub(super) fn to_cwasm_rel_path(clean_rel: &str) -> String {
        if clean_rel.ends_with(".cwasm") {
            clean_rel.to_string()
        } else if let Some(stem) = clean_rel.strip_suffix(".wasm") {
            format!("{stem}.cwasm")
        } else if let Some(stem) = clean_rel.strip_suffix(".wat") {
            format!("{stem}.cwasm")
        } else {
            format!("{clean_rel}.cwasm")
        }
    }

    pub(super) async fn insert_cached(
        &self,
        key: String,
        component: Component,
        artifact_key: Option<String>,
    ) {
        let tick = self.seq.fetch_add(1, Ordering::Relaxed);
        let max_entries = self.config.wasm_cache_max_compiled.max(1);
        let mut guard = self.cache.lock().await;
        if !guard.contains_key(&key) && guard.len() >= max_entries {
            if let Some(lru_key) = guard
                .iter()
                .min_by_key(|(_, v)| v.last_used)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&lru_key);
            }
        }
        guard.insert(
            key,
            CachedComponent {
                component,
                artifact_key,
                last_used: tick,
            },
        );
    }

    async fn invalidate_artifact_cache(&self, artifact_key: &str) {
        let mut guard = self.cache.lock().await;
        guard.retain(|k, v| k != artifact_key && v.artifact_key.as_deref() != Some(artifact_key));
    }

    pub(super) async fn invalidate_project_cache(&self, project_id: &str) {
        let prefix = format!("{project_id}/");
        let mut guard = self.cache.lock().await;
        guard.retain(|k, v| {
            !k.starts_with(&prefix)
                && !v
                    .artifact_key
                    .as_deref()
                    .is_some_and(|a| a.starts_with(&prefix))
        });
    }

    pub(super) fn resolve_project_artifact_paths(
        &self,
        project_id: &str,
        raw_path: &str,
    ) -> Result<(String, PathBuf, String, PathBuf)> {
        let clean_rel = Self::sanitize_rel_path(raw_path)?;
        let artifacts_dir = self.config.artifacts_dir();
        let effective_rel = if clean_rel.contains('/') {
            let prefix = format!("{project_id}/");
            if !clean_rel.starts_with(&prefix) {
                return Err(FerroError::Forbidden(format!(
                    "artifact path '{clean_rel}' does not belong to app '{project_id}'"
                )));
            }
            clean_rel
        } else {
            format!("{project_id}/{clean_rel}")
        };
        let disk_path = artifacts_dir.join(&effective_rel);
        let cwasm_rel = Self::to_cwasm_rel_path(&effective_rel);
        let cwasm_disk = artifacts_dir.join(&cwasm_rel);
        Ok((effective_rel, disk_path, cwasm_rel, cwasm_disk))
    }

    async fn get_cached(&self, key: &str) -> Option<Component> {
        let tick = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut guard = self.cache.lock().await;
        if let Some(entry) = guard.get_mut(key) {
            entry.last_used = tick;
            return Some(entry.component.clone());
        }
        None
    }

    async fn write_file_atomic(dest: &Path, bytes: &[u8]) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| FerroError::Internal(anyhow!("create artifact dir: {e}")))?;
        }
        let tmp = PathBuf::from(format!(
            "{}.tmp.{}",
            dest.display(),
            Uuid::new_v4().simple()
        ));
        tokio::fs::write(&tmp, bytes)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("write temp artifact: {e}")))?;
        if let Err(e) = tokio::fs::rename(&tmp, dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(FerroError::Internal(anyhow!("rename artifact: {e}")));
        }
        Ok(())
    }

    fn mmap_deserialize_component(&self, path: &Path) -> Result<Component> {
        unsafe { Component::deserialize_file(&self.engine, path) }.map_err(|e| {
            FerroError::InvalidArgument(format!(
                "failed to load precompiled component '{}': {e:#}",
                path.display()
            ))
        })
    }

    async fn compile_and_write_cwasm(&self, bytes: Bytes, cwasm_dest: &Path) -> Result<Component> {
        let _permit = self
            .compile_sem
            .acquire()
            .await
            .map_err(|_| FerroError::Internal(anyhow!("compile semaphore closed")))?;

        let engine = self.engine.clone();
        let serialized = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let component = Component::new(&engine, &bytes).map_err(|e| {
                FerroError::InvalidArgument(format!("invalid Wasm component artifact: {e:#}"))
            })?;
            component.serialize().map_err(|e| {
                FerroError::Internal(anyhow!("failed to serialize AOT component: {e:#}"))
            })
        })
        .await
        .map_err(|e| FerroError::Internal(anyhow!("compile worker panicked: {e}")))??;

        Self::write_file_atomic(cwasm_dest, &serialized).await?;
        self.mmap_deserialize_component(cwasm_dest)
    }

    pub(super) async fn compile_or_store_artifact(
        &self,
        project_id: &str,
        rel_path: &str,
        bytes: Bytes,
    ) -> Result<(String, bool, Component)> {
        let clean = Self::sanitize_rel_path(rel_path)?;
        let scoped_rel = if clean.starts_with(&format!("{project_id}/")) {
            clean
        } else {
            format!("{project_id}/{clean}")
        };
        let cwasm_rel = Self::to_cwasm_rel_path(&scoped_rel);
        let cwasm_disk = self.config.artifacts_dir().join(&cwasm_rel);

        let (precompiled, component) = match Engine::detect_precompiled(&bytes) {
            Some(Precompiled::Component) => {
                return Err(FerroError::InvalidArgument(
                    "precompiled uploads are not accepted; upload a Wasm component".into(),
                ));
            }
            Some(Precompiled::Module) => {
                return Err(FerroError::InvalidArgument(
                    "precompiled core Wasm module is not supported; expected a Wasm component (.cwasm)"
                        .into(),
                ));
            }
            None => {
                let comp = self.compile_and_write_cwasm(bytes, &cwasm_disk).await?;
                (false, comp)
            }
        };

        self.invalidate_artifact_cache(&cwasm_rel).await;
        self.insert_cached(
            cwasm_rel.clone(),
            component.clone(),
            Some(cwasm_rel.clone()),
        )
        .await;
        Ok((cwasm_rel, precompiled, component))
    }

    pub(super) async fn get_or_load_component(&self, deployment: &Deployment) -> Result<Component> {
        let raw_path = deployment.artifact_path.as_deref().ok_or_else(|| {
            FerroError::InvalidArgument(format!(
                "deployment '{}' has no artifact_path",
                deployment.id
            ))
        })?;
        let (clean_rel, disk_path, cwasm_rel, cwasm_disk) =
            self.resolve_project_artifact_paths(&deployment.project_id, raw_path)?;

        if let Some(comp) = self.get_cached(&deployment.id).await {
            return Ok(comp);
        }

        if let Some(comp) = self.get_cached(&cwasm_rel).await {
            self.insert_cached(deployment.id.clone(), comp.clone(), Some(cwasm_rel))
                .await;
            return Ok(comp);
        }

        let component = if cwasm_disk.exists()
            && matches!(
                Engine::detect_precompiled_file(&cwasm_disk),
                Ok(Some(Precompiled::Component))
            ) {
            self.mmap_deserialize_component(&cwasm_disk)?
        } else if disk_path.exists() {
            if matches!(
                Engine::detect_precompiled_file(&disk_path),
                Ok(Some(Precompiled::Component))
            ) {
                self.mmap_deserialize_component(&disk_path)?
            } else {
                let raw_bytes = tokio::fs::read(&disk_path).await.map_err(|e| {
                    FerroError::NotFound(format!("artifact '{clean_rel}' read failed: {e}"))
                })?;
                self.compile_and_write_cwasm(Bytes::from(raw_bytes), &cwasm_disk)
                    .await?
            }
        } else {
            return Err(FerroError::NotFound(format!(
                "artifact '{clean_rel}' not found under artifacts directory"
            )));
        };

        self.insert_cached(
            cwasm_rel.clone(),
            component.clone(),
            Some(cwasm_rel.clone()),
        )
        .await;
        self.insert_cached(deployment.id.clone(), component.clone(), Some(cwasm_rel))
            .await;
        Ok(component)
    }

    fn default_max_memory_bytes(&self) -> usize {
        self.svc
            .get()
            .map(|s| s.effective_wasm_max_memory_bytes())
            .unwrap_or(self.config.wasm_max_memory_bytes)
    }

    pub(super) fn default_epoch_timeout_ms(&self) -> u64 {
        self.svc
            .get()
            .map(|s| s.effective_wasm_epoch_timeout_ms())
            .unwrap_or(self.config.wasm_epoch_timeout_ms)
    }

    fn default_allow_private_outbound(&self) -> bool {
        self.svc
            .get()
            .map(|s| s.effective_wasm_allow_private_outbound())
            .unwrap_or(self.config.wasm_allow_private_outbound)
    }

    fn build_store(
        &self,
        ctx: &RequestCtx,
    ) -> Result<(Store<HostState>, MemoryOutputPipe, MemoryOutputPipe)> {
        let svc = self.service()?;
        let stdout = MemoryOutputPipe::new(PIPE_CAPACITY_BYTES);
        let stderr = MemoryOutputPipe::new(PIPE_CAPACITY_BYTES);

        let mut wasi_builder = WasiCtx::builder();
        wasi_builder.stdout(stdout.clone());
        wasi_builder.stderr(stderr.clone());
        for (k, v) in &ctx.deployment.bindings.env {
            wasi_builder.env(k, v);
        }

        let mem_limit = ctx
            .wasm_max_memory_mb
            .filter(|mb| *mb > 0)
            .map(|mb| mb.saturating_mul(1024 * 1024))
            .unwrap_or_else(|| self.default_max_memory_bytes());
        let limits = StoreLimitsBuilder::new()
            .memory_size(mem_limit)
            .table_elements(self.config.wasm_max_table_elements)
            .instances(16)
            .tables(32)
            .memories(8)
            .build();

        let caller = Caller::project(format!("worker:{}", ctx.project_id), ctx.project_id.clone());
        let state = HostState {
            wasi: wasi_builder.build(),
            http: WasiHttpCtx::new(),
            hooks: FerroHttpHooks {
                blocked_upstream: self.config.local_api.as_ref().map(|c| c.upstream),
                allow_private_outbound: self.default_allow_private_outbound()
                    || ctx.allow_private_outbound,
                outbound_http_used: Arc::new(AtomicU32::new(0)),
                max_outbound_http: DEFAULT_MAX_OUTBOUND_HTTP_PER_INVOCATION,
            },
            table: ResourceTable::new(),
            limits,
            svc,
            ctx: ctx.clone(),
            caller,
            hostcalls_used: 0,
            max_hostcalls: DEFAULT_MAX_HOSTCALLS_PER_INVOCATION,
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        let timeout_ms = ctx
            .timeout_ms
            .filter(|t| *t > 0)
            .unwrap_or_else(|| self.default_epoch_timeout_ms());
        let ticks = (timeout_ms / EPOCH_TICK_MS).max(1);
        store.set_epoch_deadline(ticks);
        store.epoch_deadline_trap();

        Ok((store, stdout, stderr))
    }

    pub(super) fn flush_stdio_logs(
        &self,
        ctx: &RequestCtx,
        stdout: &MemoryOutputPipe,
        stderr: &MemoryOutputPipe,
    ) {
        let Ok(svc) = self.service() else {
            return;
        };
        let now = Utc::now().timestamp_millis();
        for (level, pipe) in [("info", stdout), ("error", stderr)] {
            let bytes = pipe.contents();
            if bytes.is_empty() {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for line in text.lines() {
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    continue;
                }
                svc.emit_log(LogEntry {
                    id: None,
                    ts: now,
                    project_id: Some(ctx.project_id.clone()),
                    request_id: Some(ctx.request_id.clone()),
                    source: "runtime".into(),
                    level: level.into(),
                    message: trimmed.to_string(),
                    meta_json: None,
                });
            }
        }
    }

    pub(super) fn map_exec_err(err: wasmtime::Error) -> FerroError {
        if let Some(Trap::Interrupt) = err.downcast_ref::<Trap>() {
            return FerroError::Internal(anyhow!("worker execution timed out (epoch interrupt)"));
        }
        let msg = format!("{err:#}");
        if msg.contains("epoch") || msg.contains("interrupt") {
            FerroError::Internal(anyhow!("worker execution timed out: {msg}"))
        } else {
            FerroError::Internal(anyhow!("worker execution failed: {msg}"))
        }
    }

    pub(super) async fn invoke_http_component(
        &self,
        component: &Component,
        ctx: &RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse> {
        let instance_pre = self
            .linker
            .instantiate_pre(component)
            .map_err(|e| FerroError::Internal(anyhow!("instantiate_pre failed: {e:#}")))?;
        let proxy_pre = ProxyPre::new(instance_pre).map_err(|e| {
            FerroError::InvalidArgument(format!(
                "worker does not export wasi:http/incoming-handler: {e:#}"
            ))
        })?;

        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| FerroError::InvalidArgument(format!("invalid HTTP method: {e}")))?;
        let uri: Uri = req
            .uri
            .parse()
            .map_err(|e| FerroError::InvalidArgument(format!("invalid request URI: {e}")))?;
        let mut header_map = HeaderMap::new();
        for (k, v) in req.headers {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_bytes(&v),
            ) {
                header_map.append(name, val);
            }
        }
        if !header_map.contains_key(header::HOST) && uri.authority().is_none() {
            header_map.insert(header::HOST, HeaderValue::from_static("localhost"));
        }

        let mapped_stream = req
            .body
            .map_ok(Frame::data)
            .map_err(|e| wasmtime_wasi_http::Error::InternalError(Some(e.to_string())));
        let boxed_body: WasiBody = StreamBody::new(mapped_stream).boxed_unsync();

        let mut builder = hyper::Request::builder().method(method).uri(uri);
        if let Some(h) = builder.headers_mut() {
            *h = header_map;
        }
        let hyper_req = builder
            .body(boxed_body)
            .map_err(|e| FerroError::Internal(anyhow!("build hyper request: {e}")))?;

        let (mut store, stdout, stderr) = self.build_store(ctx)?;
        let incoming = store
            .data_mut()
            .http()
            .new_incoming_request(Scheme::Http, hyper_req)
            .map_err(|e| FerroError::Internal(anyhow!("new_incoming_request: {e:#}")))?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        let outparam = store
            .data_mut()
            .http()
            .new_response_outparam(tx)
            .map_err(|e| FerroError::Internal(anyhow!("new_response_outparam: {e:#}")))?;

        let task = tokio::task::spawn(async move {
            let proxy = proxy_pre.instantiate_async(&mut store).await?;
            proxy
                .wasi_http_incoming_handler()
                .call_handle(&mut store, incoming, outparam)
                .await?;
            Ok::<_, wasmtime::Error>(())
        });

        let eff_timeout_ms = ctx
            .timeout_ms
            .filter(|t| *t > 0)
            .unwrap_or_else(|| self.default_epoch_timeout_ms());
        let timeout_dur = Duration::from_millis(eff_timeout_ms + 500);
        let res = tokio::time::timeout(timeout_dur, async {
            let resp_opt = match rx.await {
                Ok(Ok(resp)) => {
                    let (parts, body) = resp.into_parts();
                    let collected = Limited::new(body, MAX_HTTP_BODY_BYTES)
                        .collect()
                        .await
                        .map_err(|e| {
                            if e.is::<LengthLimitError>() {
                                FerroError::InvalidArgument(
                                    "worker response body exceeded limit".into(),
                                )
                            } else {
                                FerroError::Internal(anyhow!("collect worker body: {e}"))
                            }
                        })?
                        .to_bytes();
                    Some(Ok((parts, collected)))
                }
                Ok(Err(err_code)) => Some(Err(err_code)),
                Err(_) => None,
            };

            let task_res = task
                .await
                .map_err(|e| FerroError::Internal(anyhow!("worker task join error: {e}")))?;
            if let Err(e) = task_res {
                return Err(Self::map_exec_err(e));
            }

            match resp_opt {
                Some(Ok((parts, body_bytes))) => {
                    let headers = parts
                        .headers
                        .iter()
                        .map(|(k, v)| (k.as_str().to_string(), v.as_bytes().to_vec()))
                        .collect();
                    let stream: ByteStream = Box::pin(futures_util::stream::once(async move {
                        Ok::<_, std::io::Error>(body_bytes)
                    }));
                    Ok(WorkerHttpResponse {
                        status: parts.status.as_u16(),
                        headers,
                        body: stream,
                    })
                }
                Some(Err(code)) => Err(FerroError::Internal(anyhow!(
                    "worker returned HTTP error code: {code:?}"
                ))),
                None => Err(FerroError::Internal(anyhow!(
                    "worker completed without sending HTTP response"
                ))),
            }
        })
        .await;

        self.flush_stdio_logs(ctx, &stdout, &stderr);

        match res {
            Ok(inner) => inner,
            Err(_) => Err(FerroError::Internal(anyhow!(
                "worker execution timed out after {}ms",
                eff_timeout_ms
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, SocketAddr};

    use super::invoke::parse_cron_http_target;
    use super::outbound::{is_blocked_socket_addr, is_prohibited_outbound_ip};

    #[test]
    fn cron_http_target_parses_method_and_path() {
        assert_eq!(
            parse_cron_http_target("GET /custom-cron?full=1"),
            ("GET".into(), "/custom-cron?full=1".into())
        );
        assert_eq!(
            parse_cron_http_target("post /_internal/cron"),
            ("POST".into(), "/_internal/cron".into())
        );
        assert_eq!(
            parse_cron_http_target("/nightly-sync"),
            ("POST".into(), "/nightly-sync".into())
        );
        assert_eq!(
            parse_cron_http_target("invalid-target"),
            ("POST".into(), "/_internal/cron".into())
        );
    }

    #[test]
    fn outbound_ssrf_ip_classification() {
        for blocked in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.0.5",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
        ] {
            let ip: IpAddr = blocked.parse().unwrap();
            assert!(is_prohibited_outbound_ip(ip), "expected {blocked} blocked");
        }
        assert!(!is_prohibited_outbound_ip("93.184.216.34".parse().unwrap()));
        assert!(!is_prohibited_outbound_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));

        let blocked_up: SocketAddr = "127.0.0.1:3000".parse().unwrap();
        assert!(is_blocked_socket_addr(
            "[::ffff:127.0.0.1]:3000".parse().unwrap(),
            Some(blocked_up),
            true,
        ));
        assert!(!is_blocked_socket_addr(
            "127.0.0.1:3001".parse().unwrap(),
            Some(blocked_up),
            true,
        ));
    }
}
