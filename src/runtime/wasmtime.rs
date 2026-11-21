use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use futures_util::TryStreamExt;
use http::{header, HeaderMap, HeaderName, HeaderValue, Method, Uri};
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config as WasmConfig, Engine, OptLevel, Precompiled, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::bindings::ProxyPre;
use wasmtime_wasi_http::{RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView};

use crate::config::Config;
use crate::core::model::{
    BlobAuth, ByteStream, Caller, Deployment, EnqueueOpts, FerroError, Job, JobOutcome,
    LogEntry, RequestCtx, Result, SqlValue, WorkerHttpRequest, WorkerHttpResponse,
};
use crate::core::service::PlatformService;
use crate::core::traits::WorkerRuntime;

mod host_wit {
    wasmtime::component::bindgen!({
        path: "wit",
        world: "ferropod:platform/host-imports",
        imports: { default: async },
        require_store_data_send: true,
    });
}

mod queue_wit {
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
            "ferropod:platform/env": super::host_wit::ferropod::platform::env,
        },
        require_store_data_send: true,
    });
}

mod scheduled_wit {
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
            "ferropod:platform/env": super::host_wit::ferropod::platform::env,
        },
        require_store_data_send: true,
    });
}

use host_wit::ferropod::platform::{bucket as wit_bucket, db as wit_db, env as wit_env, kv as wit_kv, log as wit_log, queue as wit_queue};
use queue_wit::exports::ferropod::platform::queue_consumer as wit_qc;
use scheduled_wit::exports::ferropod::platform::scheduled as wit_sched;

const EPOCH_TICK_MS: u64 = 100;
const PIPE_CAPACITY_BYTES: usize = 64 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 32 * 1024 * 1024;

struct CachedComponent {
    component: Component,
    last_used: u64,
}

/// Wasmtime component runtime executing Workers (`.wasm`/`.wat`/`.cwasm`) with bounded memory,
/// Cranelift compile concurrency caps, epoch interruption, and WIT host bindings.
pub struct WasmtimeRuntime {
    engine: Engine,
    linker: Linker<HostState>,
    config: Config,
    svc: OnceLock<PlatformService>,
    compile_sem: Arc<Semaphore>,
    cache: Mutex<HashMap<String, CachedComponent>>,
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

        let engine = Engine::new(&wasm_cfg)
            .map_err(|e| FerroError::Internal(anyhow!("failed to initialize wasmtime engine: {e:#}")))?;

        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|e| FerroError::Internal(anyhow!("failed to link wasi p2: {e:#}")))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .map_err(|e| FerroError::Internal(anyhow!("failed to link wasi http: {e:#}")))?;
        host_wit::HostImports::add_to_linker::<HostState, HasSelf<HostState>>(&mut linker, |s| s)
            .map_err(|e| FerroError::Internal(anyhow!("failed to link ferropod host imports: {e:#}")))?;

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

    fn sanitize_rel_path(rel_path: &str) -> Result<String> {
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

    fn to_cwasm_rel_path(clean_rel: &str) -> String {
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

    async fn insert_cached(&self, key: String, component: Component) {
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
                last_used: tick,
            },
        );
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
        let tmp = PathBuf::from(format!("{}.tmp.{}", dest.display(), Uuid::new_v4().simple()));
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

    async fn compile_or_store_artifact(
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
                Self::write_file_atomic(&cwasm_disk, &bytes).await?;
                let comp = self.mmap_deserialize_component(&cwasm_disk)?;
                (true, comp)
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

        self.insert_cached(cwasm_rel.clone(), component.clone()).await;
        Ok((cwasm_rel, precompiled, component))
    }

    async fn get_or_load_component(&self, deployment: &Deployment) -> Result<Component> {
        if let Some(comp) = self.get_cached(&deployment.id).await {
            return Ok(comp);
        }

        let raw_path = deployment.artifact_path.as_deref().ok_or_else(|| {
            FerroError::InvalidArgument(format!(
                "deployment '{}' has no artifact_path",
                deployment.id
            ))
        })?;
        let clean_rel = Self::sanitize_rel_path(raw_path)?;
        if let Some(comp) = self.get_cached(&clean_rel).await {
            self.insert_cached(deployment.id.clone(), comp.clone()).await;
            return Ok(comp);
        }

        let artifacts_dir = self.config.artifacts_dir();
        let disk_path = artifacts_dir.join(&clean_rel);
        let cwasm_rel = Self::to_cwasm_rel_path(&clean_rel);
        let cwasm_disk = artifacts_dir.join(&cwasm_rel);

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

        self.insert_cached(cwasm_rel, component.clone()).await;
        self.insert_cached(deployment.id.clone(), component.clone())
            .await;
        Ok(component)
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

        let limits = StoreLimitsBuilder::new()
            .memory_size(self.config.wasm_max_memory_bytes)
            .table_elements(self.config.wasm_max_table_elements)
            .instances(16)
            .tables(32)
            .memories(8)
            .build();

        let caller = Caller::project(
            format!("worker:{}", ctx.project_id),
            ctx.project_id.clone(),
        );
        let state = HostState {
            wasi: wasi_builder.build(),
            http: WasiHttpCtx::new(),
            hooks: FerroHttpHooks,
            table: ResourceTable::new(),
            limits,
            svc,
            ctx: ctx.clone(),
            caller,
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        let ticks = (self.config.wasm_epoch_timeout_ms / EPOCH_TICK_MS).max(1);
        store.set_epoch_deadline(ticks);
        store.epoch_deadline_trap();

        Ok((store, stdout, stderr))
    }

    fn flush_stdio_logs(
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

    fn map_exec_err(err: wasmtime::Error) -> FerroError {
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

    async fn invoke_http_component(
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

        let timeout_dur = Duration::from_millis(self.config.wasm_epoch_timeout_ms + 500);
        let res = tokio::time::timeout(timeout_dur, async {
            let resp_opt = match rx.await {
                Ok(Ok(resp)) => {
                    let (parts, body) = resp.into_parts();
                    let collected = body
                        .collect()
                        .await
                        .map_err(|e| FerroError::Internal(anyhow!("collect worker body: {e}")))?
                        .to_bytes();
                    if collected.len() > MAX_HTTP_BODY_BYTES {
                        return Err(FerroError::InvalidArgument(
                            "worker response body exceeded limit".into(),
                        ));
                    }
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
                self.config.wasm_epoch_timeout_ms
            ))),
        }
    }
}

#[async_trait]
impl WorkerRuntime for WasmtimeRuntime {
    async fn store_artifact(
        &self,
        project_id: &str,
        rel_path: &str,
        artifact: Bytes,
    ) -> Result<(String, bool)> {
        let (stored, precompiled, _) = self
            .compile_or_store_artifact(project_id, rel_path, artifact)
            .await?;
        Ok((stored, precompiled))
    }

    async fn compile_and_load(&self, deployment: &Deployment, artifact: Bytes) -> Result<()> {
        let raw_rel = deployment.artifact_path.as_deref().unwrap_or("worker.cwasm");
        let (_, _, comp) = self
            .compile_or_store_artifact(&deployment.project_id, raw_rel, artifact)
            .await?;
        self.insert_cached(deployment.id.clone(), comp).await;
        Ok(())
    }

    async fn preload_deployment(&self, deployment: &Deployment) -> Result<()> {
        if deployment.artifact_path.is_some() {
            let _ = self.get_or_load_component(deployment).await?;
        }
        Ok(())
    }

    async fn handle_http(
        &self,
        ctx: RequestCtx,
        req: WorkerHttpRequest,
    ) -> Result<WorkerHttpResponse> {
        let component = self.get_or_load_component(&ctx.deployment).await?;
        self.invoke_http_component(&component, &ctx, req).await
    }

    async fn handle_queue(&self, ctx: RequestCtx, jobs: Vec<Job>) -> Result<Vec<JobOutcome>> {
        if jobs.is_empty() {
            return Ok(Vec::new());
        }
        let component = self.get_or_load_component(&ctx.deployment).await?;
        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| FerroError::Internal(anyhow!("instantiate_pre failed: {e:#}")))?;

        let queue_name = jobs[0].queue.clone();

        if let Ok(queue_pre) = queue_wit::QueueWorkerPre::new(instance_pre) {
            let (mut store, stdout, stderr) = self.build_store(&ctx)?;
            let wit_jobs: Vec<wit_qc::QueueJob> = jobs
                .iter()
                .map(|j| wit_qc::QueueJob {
                    id: j.id,
                    queue: j.queue.clone(),
                    payload: j.payload.clone(),
                    attempts: j.attempts,
                    max_attempts: j.max_attempts,
                    dedup_key: j.dedup_key.clone(),
                    created_at: j.created_at,
                })
                .collect();

            let timeout_dur = Duration::from_millis(self.config.wasm_epoch_timeout_ms + 500);
            let exec = tokio::time::timeout(timeout_dur, async {
                let worker = queue_pre
                    .instantiate_async(&mut store)
                    .await
                    .map_err(Self::map_exec_err)?;
                worker
                    .ferropod_platform_queue_consumer()
                    .call_handle_batch(&mut store, &queue_name, &wit_jobs)
                    .await
                    .map_err(Self::map_exec_err)
            })
            .await;

            self.flush_stdio_logs(&ctx, &stdout, &stderr);

            let outcomes = match exec {
                Ok(res) => res?,
                Err(_) => {
                    return Err(FerroError::Internal(anyhow!(
                        "queue worker execution timed out"
                    )));
                }
            };

            let mapped = outcomes
                .into_iter()
                .map(|o| match o {
                    wit_qc::JobOutcome::Ack => JobOutcome::Ack,
                    wit_qc::JobOutcome::Retry(r) => JobOutcome::Retry {
                        delay_secs: r.delay_secs,
                        error: r.error,
                    },
                    wit_qc::JobOutcome::Dead(err) => JobOutcome::Dead { error: err },
                })
                .collect();
            return Ok(mapped);
        }

        // HTTP fallback for workers that only export `wasi:http/incoming-handler`.
        let payload_json = serde_json::to_vec(&jobs)
            .map_err(|e| FerroError::Internal(anyhow!("serialize queue batch: {e}")))?;
        let req = WorkerHttpRequest {
            method: "POST".into(),
            uri: format!("/_internal/queue/{queue_name}"),
            headers: vec![
                ("content-type".into(), b"application/json".to_vec()),
                ("host".into(), b"localhost".to_vec()),
            ],
            body: Box::pin(futures_util::stream::once(async move {
                Ok::<_, std::io::Error>(Bytes::from(payload_json))
            })),
        };
        let resp = self.invoke_http_component(&component, &ctx, req).await?;
        let ok = (200..300).contains(&resp.status);
        let outcome = if ok {
            JobOutcome::Ack
        } else {
            JobOutcome::Retry {
                delay_secs: 5,
                error: format!("queue HTTP handler returned status {}", resp.status),
            }
        };
        Ok(vec![outcome; jobs.len()])
    }

    async fn handle_cron(&self, ctx: RequestCtx, expr: &str) -> Result<()> {
        let component = self.get_or_load_component(&ctx.deployment).await?;
        let instance_pre = self
            .linker
            .instantiate_pre(&component)
            .map_err(|e| FerroError::Internal(anyhow!("instantiate_pre failed: {e:#}")))?;

        let cron_job = ctx.cron_job.clone();
        if let Ok(sched_pre) = scheduled_wit::ScheduledWorkerPre::new(instance_pre) {
            let (mut store, stdout, stderr) = self.build_store(&ctx)?;
            let event = match &cron_job {
                Some(cj) => wit_sched::CronEvent {
                    cron_id: cj.id.clone(),
                    name: cj.name.clone(),
                    schedule: expr.to_string(),
                    target: cj.target.clone(),
                    scheduled_time_ms: Utc::now().timestamp_millis(),
                },
                None => wit_sched::CronEvent {
                    cron_id: ctx.request_id.clone(),
                    name: "cron".into(),
                    schedule: expr.to_string(),
                    target: "/".into(),
                    scheduled_time_ms: Utc::now().timestamp_millis(),
                },
            };

            let timeout_dur = Duration::from_millis(self.config.wasm_epoch_timeout_ms + 500);
            let exec = tokio::time::timeout(timeout_dur, async {
                let worker = sched_pre
                    .instantiate_async(&mut store)
                    .await
                    .map_err(Self::map_exec_err)?;
                worker
                    .ferropod_platform_scheduled()
                    .call_handle_cron(&mut store, &event)
                    .await
                    .map_err(Self::map_exec_err)
            })
            .await;

            self.flush_stdio_logs(&ctx, &stdout, &stderr);

            let guest_res = match exec {
                Ok(res) => res?,
                Err(_) => {
                    return Err(FerroError::Internal(anyhow!(
                        "scheduled worker execution timed out"
                    )));
                }
            };

            return guest_res.map_err(|e| FerroError::Internal(anyhow!("cron handler error: {e}")));
        }

        // HTTP fallback for workers that only export `wasi:http/incoming-handler`.
        let target = cron_job
            .as_ref()
            .map(|c| c.target.as_str())
            .filter(|t| t.starts_with('/'))
            .unwrap_or("/_internal/cron");
        let req = WorkerHttpRequest {
            method: "POST".into(),
            uri: target.to_string(),
            headers: vec![
                ("host".into(), b"localhost".to_vec()),
                ("x-ferropod-cron".into(), expr.as_bytes().to_vec()),
            ],
            body: Box::pin(futures_util::stream::empty()),
        };
        let resp = self.invoke_http_component(&component, &ctx, req).await?;
        if (200..300).contains(&resp.status) {
            Ok(())
        } else {
            Err(FerroError::Internal(anyhow!(
                "cron HTTP handler returned status {}",
                resp.status
            )))
        }
    }
}

struct FerroHttpHooks;

impl WasiHttpHooks for FerroHttpHooks {
    fn default_scheme(&mut self) -> Option<http::uri::Scheme> {
        Some(http::uri::Scheme::HTTP)
    }

    fn send_request(
        &mut self,
        request: http::Request<WasiBody>,
        options: Option<RequestOptions>,
        _fut: Box<
            dyn std::future::Future<Output = std::result::Result<(), wasmtime_wasi_http::Error>>
                + Send,
        >,
    ) -> Box<
        dyn std::future::Future<
                Output = std::result::Result<
                    (
                        http::Response<WasiBody>,
                        Box<
                            dyn std::future::Future<
                                    Output = std::result::Result<(), wasmtime_wasi_http::Error>,
                                > + Send,
                        >,
                    ),
                    wasmtime_wasi_http::Error,
                >,
            > + Send,
    > {
        Box::new(async move {
            let (res, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            Ok((
                res.map(BodyExt::boxed_unsync),
                Box::new(io)
                    as Box<
                        dyn std::future::Future<
                                Output = std::result::Result<(), wasmtime_wasi_http::Error>,
                            > + Send,
                    >,
            ))
        })
    }
}

struct HostState {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    hooks: FerroHttpHooks,
    table: ResourceTable,
    limits: StoreLimits,
    svc: PlatformService,
    ctx: RequestCtx,
    caller: Caller,
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
        let ns = self.resolve_kv(&binding)?;
        self.svc
            .kv_incr(&self.caller, ns, &key, delta, ttl_secs)
            .await
            .map_err(|e| e.to_string())
    }

    async fn delete(&mut self, binding: String, key: String) -> std::result::Result<(), String> {
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
        let bucket_name = self.resolve_bucket(&binding)?;
        let auth = BlobAuth::Caller(&self.caller);
        let Some((_meta, mut stream, _len, _)) = self
            .svc
            .get_object_stream(&auth, bucket_name, &key, None)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let mut buf = Vec::new();
        while let Some(chunk) = stream.try_next().await.map_err(|e| e.to_string())? {
            buf.extend_from_slice(&chunk);
        }
        Ok(Some(buf))
    }

    async fn get_object(
        &mut self,
        binding: String,
        key: String,
    ) -> std::result::Result<Option<wit_bucket::ObjectData>, String> {
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
        let mut buf = Vec::new();
        while let Some(chunk) = stream.try_next().await.map_err(|e| e.to_string())? {
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
