use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use axum_server::accept::Accept;
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use futures_util::StreamExt;
use rustls_acme::{axum::AxumAcceptor, caches::DirCache, AcmeConfig};
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::core::service::PlatformService;

#[derive(Debug, Serialize)]
struct HealthzResp {
    status: &'static str,
}

/// Validated TLS runtime mode resolved from [`Config`] and active route domains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTls {
    /// Plain HTTP only on `cfg.bind_addr`.
    Disabled,
    /// Native HTTPS using user-provided PEM certificate chain and private key.
    Manual {
        bind: SocketAddr,
        cert_path: PathBuf,
        key_path: PathBuf,
    },
    /// Native HTTPS using automatic Let's Encrypt ACME (TLS-ALPN-01) certificates.
    Acme {
        bind: SocketAddr,
        domains: Vec<String>,
        email: Option<String>,
        staging: bool,
        cache_dir: PathBuf,
    },
}

fn normalize_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('.').to_ascii_lowercase();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Validate and resolve TLS settings.
pub fn resolve_tls(cfg: &Config, route_domains: Vec<String>) -> Result<ResolvedTls> {
    let tls = &cfg.tls;
    let has_cert = tls.cert_path.is_some();
    let has_key = tls.key_path.is_some();
    let has_manual = has_cert || has_key;
    let has_acme = !tls.acme_domains.is_empty() || tls.acme_email.is_some() || tls.acme_staging;

    if has_cert != has_key {
        bail!("--tls-cert and --tls-key must both be provided for manual TLS mode");
    }
    if has_manual && has_acme {
        bail!("cannot combine manual TLS (--tls-cert/--tls-key) with ACME (--acme-domain/--acme-email)");
    }

    if !has_manual && !has_acme && tls.https_bind.is_none() {
        return Ok(ResolvedTls::Disabled);
    }

    let bind = tls
        .https_bind
        .unwrap_or_else(|| "0.0.0.0:443".parse().expect("valid default HTTPS bind"));

    if has_manual {
        return Ok(ResolvedTls::Manual {
            bind,
            cert_path: tls.cert_path.clone().expect("checked above"),
            key_path: tls.key_path.clone().expect("checked above"),
        });
    }

    // ACME mode (either explicit ACME flags, or `--https-bind` without manual certs).
    let mut set = BTreeSet::new();
    for d in &tls.acme_domains {
        if let Some(norm) = normalize_domain(d) {
            set.insert(norm);
        }
    }
    for d in route_domains {
        if let Some(norm) = normalize_domain(&d) {
            set.insert(norm);
        }
    }

    if set.is_empty() {
        bail!(
            "HTTPS is enabled without --tls-cert/--tls-key, so ACME requires at least one domain via --acme-domain (or an existing route domain in the database)"
        );
    }

    let email = tls
        .acme_email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    Ok(ResolvedTls::Acme {
        bind,
        domains: set.into_iter().collect(),
        email,
        staging: tls.acme_staging,
        cache_dir: cfg.certs_dir(),
    })
}

/// Extract host without port from an HTTP `Host` header value (supports IPv4, domains, and `[IPv6]`).
fn strip_host_port(host_header: &str) -> &str {
    let trimmed = host_header.trim();
    if let Some(rest) = trimmed.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return &trimmed[..=end + 1];
        }
        return trimmed;
    }
    match trimmed.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => trimmed,
    }
}

/// Build an HTTP-to-HTTPS `308 Permanent Redirect` router that keeps `/_healthz` reachable over HTTP.
pub fn build_https_redirect_router(https_port: u16) -> Router {
    Router::new()
        .route(
            "/_healthz",
            get(|| async { Json(HealthzResp { status: "ok" }) }),
        )
        .fallback(move |req: Request<Body>| async move { redirect_to_https(req, https_port) })
}

/// Build a dynamic HTTP router that enforces HTTP->HTTPS redirects live based on
/// [`PlatformService::effective_http_redirect_https`] while always keeping `/_healthz` accessible.
pub fn build_dynamic_http_router(svc: PlatformService, app: Router, https_port: u16) -> Router {
    let default_redirect = svc.config().tls.http_redirect_https;
    build_dynamic_http_router_with_default(svc, app, https_port, default_redirect)
}

fn build_dynamic_http_router_with_default(
    svc: PlatformService,
    app: Router,
    https_port: u16,
    default_redirect: bool,
) -> Router {
    app.layer(middleware::from_fn_with_state(
        svc,
        move |State(svc): State<PlatformService>, req: Request<Body>, next: Next| async move {
            let redirect_enabled = svc
                .settings_overrides()
                .http_redirect_https
                .unwrap_or(default_redirect);
            if redirect_enabled && req.uri().path() != "/_healthz" {
                return redirect_to_https(req, https_port);
            }
            next.run(req).await
        },
    ))
}

#[derive(Clone)]
struct HotSwapAcmeAcceptor {
    inner: Arc<RwLock<AxumAcceptor>>,
}

impl HotSwapAcmeAcceptor {
    fn new(acceptor: AxumAcceptor) -> Self {
        Self {
            inner: Arc::new(RwLock::new(acceptor)),
        }
    }

    fn swap(&self, next: AxumAcceptor) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        *guard = next;
    }
}

impl<I, S> Accept<I, S> for HotSwapAcmeAcceptor
where
    I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    S: Send + 'static,
{
    type Stream = <AxumAcceptor as Accept<I, S>>::Stream;
    type Service = <AxumAcceptor as Accept<I, S>>::Service;
    type Future = <AxumAcceptor as Accept<I, S>>::Future;

    fn accept(&self, stream: I, service: S) -> Self::Future {
        let acceptor = self.inner.read().unwrap_or_else(|e| e.into_inner()).clone();
        acceptor.accept(stream, service)
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn spawn_acme_instance(
    domains: Vec<String>,
    email: Option<String>,
    staging: bool,
    cache_dir: PathBuf,
    svc: PlatformService,
) -> (AxumAcceptor, AbortOnDrop) {
    let mut acme_cfg = AcmeConfig::new(domains)
        .cache(DirCache::new(cache_dir))
        .directory_lets_encrypt(!staging);
    if let Some(addr) = email.as_deref() {
        acme_cfg = acme_cfg.contact_push(format!("mailto:{addr}"));
    }

    let mut state = acme_cfg.state();
    let acceptor = state.axum_acceptor(state.default_rustls_config());
    let task = tokio::spawn(async move {
        while let Some(ev) = state.next().await {
            match ev {
                Ok(ok) => {
                    let msg = format!("{ok:?}");
                    info!(event = %msg, "acme event");
                    svc.update_tls_runtime(|rt| {
                        rt.last_acme_event = Some(msg);
                        rt.last_acme_error = None;
                    });
                }
                Err(err) => {
                    let msg = format!("{err:?}");
                    error!(error = %msg, "acme error");
                    svc.update_tls_runtime(|rt| {
                        rt.last_acme_error = Some(msg);
                    });
                }
            }
        }
    });
    (acceptor, AbortOnDrop(task))
}

fn is_valid_redirect_host(host: &str) -> bool {
    !host.is_empty()
        && !host.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '@' | '?' | '#')
        })
}

fn redirect_to_https(req: Request<Body>, https_port: u16) -> Response {
    let Some(raw_host) = crate::gateway::extract_request_host(&req) else {
        return (StatusCode::BAD_REQUEST, "missing Host header").into_response();
    };
    let host = strip_host_port(raw_host);
    if !is_valid_redirect_host(host) {
        return (StatusCode::BAD_REQUEST, "invalid Host header").into_response();
    }
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    if !path_and_query.starts_with('/') {
        return (StatusCode::BAD_REQUEST, "invalid request path").into_response();
    }

    let target = if https_port == 443 {
        format!("https://{host}{path_and_query}")
    } else {
        format!("https://{host}:{https_port}{path_and_query}")
    };
    let Ok(location) = HeaderValue::try_from(target) else {
        return (StatusCode::BAD_REQUEST, "invalid redirect location").into_response();
    };
    Response::builder()
        .status(StatusCode::PERMANENT_REDIRECT)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap_or_else(|_| (StatusCode::BAD_REQUEST, "invalid redirect response").into_response())
}

/// Install the ring crypto provider for rustls (idempotent).
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Serve HTTP (and optional native HTTPS via manual PEM certs or Let's Encrypt ACME) with graceful shutdown.
pub async fn serve_http_and_https<F>(
    cfg: &Config,
    svc: &PlatformService,
    app: Router,
    shutdown: F,
) -> Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let eff = svc.effective_settings();
    let mut effective_cfg = cfg.clone();
    if cfg.tls.cert_path.is_none() && cfg.tls.key_path.is_none() && eff.acme_enabled {
        if effective_cfg.tls.https_bind.is_none() {
            effective_cfg.tls.https_bind =
                Some("0.0.0.0:443".parse().expect("valid default HTTPS bind"));
        }
        effective_cfg.tls.acme_email = eff.acme_email.clone();
        effective_cfg.tls.acme_staging = eff.acme_staging;
    }

    let route_domains = svc.effective_tls_domains().await;
    let resolved = resolve_tls(&effective_cfg, route_domains)?;

    match resolved {
        ResolvedTls::Disabled => {
            svc.update_tls_runtime(|rt| {
                rt.tls_mode = "disabled".to_string();
                rt.tls_active = false;
                rt.https_bind = None;
                rt.active_domains.clear();
            });
            let listener = tokio::net::TcpListener::bind(cfg.bind_addr)
                .await
                .with_context(|| format!("failed to bind HTTP address {}", cfg.bind_addr))?;
            info!(bind = %cfg.bind_addr, profile = ?cfg.profile, "ferropod listening (HTTP)");
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await?;
            Ok(())
        }
        ResolvedTls::Manual {
            bind: https_bind,
            cert_path,
            key_path,
        } => {
            ensure_crypto_provider();
            let rustls_cfg = RustlsConfig::from_pem_file(&cert_path, &key_path)
                .await
                .with_context(|| {
                    format!(
                        "failed to load TLS cert '{}' / key '{}'",
                        cert_path.display(),
                        key_path.display()
                    )
                })?;

            svc.update_tls_runtime(|rt| {
                rt.tls_mode = "manual".to_string();
                rt.tls_active = true;
                rt.https_bind = Some(https_bind.to_string());
                rt.active_domains.clear();
            });

            let http_handle = Handle::new();
            let https_handle = Handle::new();
            spawn_shutdown_bridge(shutdown, http_handle.clone(), https_handle.clone());

            let http_app = build_dynamic_http_router_with_default(
                svc.clone(),
                app.clone(),
                https_bind.port(),
                cfg.tls.http_redirect_https,
            );

            info!(
                http_bind = %cfg.bind_addr,
                https_bind = %https_bind,
                redirect = svc
                    .settings_overrides()
                    .http_redirect_https
                    .unwrap_or(cfg.tls.http_redirect_https),
                profile = ?cfg.profile,
                "ferropod listening (HTTP + Manual HTTPS)"
            );

            let http_fut = axum_server::bind(cfg.bind_addr)
                .handle(http_handle)
                .serve(http_app.into_make_service_with_connect_info::<SocketAddr>());
            let https_fut = axum_server::bind_rustls(https_bind, rustls_cfg)
                .handle(https_handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            tokio::try_join!(http_fut, https_fut)?;
            Ok(())
        }
        ResolvedTls::Acme {
            bind: https_bind,
            domains,
            email,
            staging,
            cache_dir,
        } => {
            ensure_crypto_provider();
            tokio::fs::create_dir_all(&cache_dir)
                .await
                .with_context(|| {
                    format!("failed to create ACME cache dir {}", cache_dir.display())
                })?;

            svc.update_tls_runtime(|rt| {
                rt.tls_mode = "acme".to_string();
                rt.tls_active = true;
                rt.https_bind = Some(https_bind.to_string());
                rt.active_domains = domains.clone();
            });

            let (initial_acceptor, initial_acme_guard) = spawn_acme_instance(
                domains.clone(),
                email.clone(),
                staging,
                cache_dir.clone(),
                svc.clone(),
            );
            let hot_acceptor = HotSwapAcmeAcceptor::new(initial_acceptor);
            let mut reload_rx = svc.subscribe_tls_reload();
            let supervisor_svc = svc.clone();
            let supervisor_acceptor = hot_acceptor.clone();
            let supervisor_cache = cache_dir.clone();
            let supervisor_task = tokio::spawn(async move {
                let mut cur_domains = domains;
                let mut cur_email = email;
                let mut cur_staging = staging;
                let mut _cur_acme_guard = initial_acme_guard;
                while reload_rx.changed().await.is_ok() {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let _ = reload_rx.borrow_and_update();
                    let eff = supervisor_svc.effective_settings();
                    let new_domains = supervisor_svc.effective_tls_domains().await;
                    let new_email = eff
                        .acme_email
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string);
                    let new_staging = eff.acme_staging;
                    if new_domains.is_empty() {
                        continue;
                    }
                    if new_domains == cur_domains
                        && new_email == cur_email
                        && new_staging == cur_staging
                    {
                        continue;
                    }
                    info!(
                        domains = ?new_domains,
                        staging = new_staging,
                        "reloading ACME certificate manager with updated domain set"
                    );
                    let (next_acceptor, next_guard) = spawn_acme_instance(
                        new_domains.clone(),
                        new_email.clone(),
                        new_staging,
                        supervisor_cache.clone(),
                        supervisor_svc.clone(),
                    );
                    supervisor_acceptor.swap(next_acceptor);
                    _cur_acme_guard = next_guard;
                    cur_domains = new_domains.clone();
                    cur_email = new_email;
                    cur_staging = new_staging;
                    supervisor_svc.update_tls_runtime(|rt| {
                        rt.active_domains = new_domains;
                        rt.reload_generation = rt.reload_generation.saturating_add(1);
                    });
                }
            });
            let _supervisor_guard = AbortOnDrop(supervisor_task);

            if https_bind.port() != 443 {
                warn!(
                    https_bind = %https_bind,
                    "Let's Encrypt TLS-ALPN-01 requires external port 443 to reach this listener"
                );
            }

            let http_handle = Handle::new();
            let https_handle = Handle::new();
            spawn_shutdown_bridge(shutdown, http_handle.clone(), https_handle.clone());

            let http_app = build_dynamic_http_router_with_default(
                svc.clone(),
                app.clone(),
                https_bind.port(),
                cfg.tls.http_redirect_https,
            );

            info!(
                http_bind = %cfg.bind_addr,
                https_bind = %https_bind,
                domains = ?svc.tls_runtime_snapshot().active_domains,
                staging,
                redirect = svc
                    .settings_overrides()
                    .http_redirect_https
                    .unwrap_or(cfg.tls.http_redirect_https),
                profile = ?cfg.profile,
                "ferropod listening (HTTP + ACME HTTPS)"
            );

            let http_fut = axum_server::bind(cfg.bind_addr)
                .handle(http_handle)
                .serve(http_app.into_make_service_with_connect_info::<SocketAddr>());
            let https_fut = axum_server::bind(https_bind)
                .acceptor(hot_acceptor)
                .handle(https_handle)
                .serve(app.into_make_service_with_connect_info::<SocketAddr>());

            let res = tokio::try_join!(http_fut, https_fut);
            res?;
            Ok(())
        }
    }
}

fn spawn_shutdown_bridge<F>(
    shutdown: F,
    http_handle: Handle<SocketAddr>,
    https_handle: Handle<SocketAddr>,
) where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        shutdown.await;
        http_handle.graceful_shutdown(Some(Duration::from_secs(10)));
        https_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });
}
