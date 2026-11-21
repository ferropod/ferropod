use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use futures_util::StreamExt;
use rustls_acme::{caches::DirCache, AcmeConfig};
use serde::Serialize;
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
        .route("/_healthz", get(|| async { Json(HealthzResp { status: "ok" }) }))
        .fallback(move |req: Request<Body>| async move { redirect_to_https(req, https_port) })
}

fn redirect_to_https(req: Request<Body>, https_port: u16) -> Response {
    let Some(raw_host) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return (StatusCode::BAD_REQUEST, "missing Host header").into_response();
    };
    let host = strip_host_port(raw_host);
    if host.is_empty() {
        return (StatusCode::BAD_REQUEST, "invalid Host header").into_response();
    }
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let target = if https_port == 443 {
        format!("https://{host}{path_and_query}")
    } else {
        format!("https://{host}:{https_port}{path_and_query}")
    };
    Redirect::permanent(&target).into_response()
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
    let route_domains = svc.allowed_domains().await;
    let resolved = resolve_tls(cfg, route_domains)?;

    match resolved {
        ResolvedTls::Disabled => {
            let listener = tokio::net::TcpListener::bind(cfg.bind_addr)
                .await
                .with_context(|| format!("failed to bind HTTP address {}", cfg.bind_addr))?;
            info!(bind = %cfg.bind_addr, profile = ?cfg.profile, "ferropod listening (HTTP)");
            axum::serve(listener, app)
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

            let http_handle = Handle::new();
            let https_handle = Handle::new();
            spawn_shutdown_bridge(shutdown, http_handle.clone(), https_handle.clone());

            let http_app = if cfg.tls.http_redirect_https {
                build_https_redirect_router(https_bind.port())
            } else {
                app.clone()
            };

            info!(
                http_bind = %cfg.bind_addr,
                https_bind = %https_bind,
                redirect = cfg.tls.http_redirect_https,
                profile = ?cfg.profile,
                "ferropod listening (HTTP + Manual HTTPS)"
            );

            let http_fut = axum_server::bind(cfg.bind_addr)
                .handle(http_handle)
                .serve(http_app.into_make_service());
            let https_fut = axum_server::bind_rustls(https_bind, rustls_cfg)
                .handle(https_handle)
                .serve(app.into_make_service());

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
                .with_context(|| format!("failed to create ACME cache dir {}", cache_dir.display()))?;

            let mut acme_cfg = AcmeConfig::new(domains.clone())
                .cache(DirCache::new(cache_dir.clone()))
                .directory_lets_encrypt(!staging);
            if let Some(addr) = email.as_deref() {
                acme_cfg = acme_cfg.contact_push(format!("mailto:{addr}"));
            }

            let mut state = acme_cfg.state();
            let acceptor = state.axum_acceptor(state.default_rustls_config());

            let acme_task = tokio::spawn(async move {
                while let Some(ev) = state.next().await {
                    match ev {
                        Ok(ok) => info!(event = ?ok, "acme event"),
                        Err(err) => error!(error = ?err, "acme error"),
                    }
                }
            });

            if https_bind.port() != 443 {
                warn!(
                    https_bind = %https_bind,
                    "Let's Encrypt TLS-ALPN-01 requires external port 443 to reach this listener"
                );
            }

            let http_handle = Handle::new();
            let https_handle = Handle::new();
            spawn_shutdown_bridge(shutdown, http_handle.clone(), https_handle.clone());

            let http_app = if cfg.tls.http_redirect_https {
                build_https_redirect_router(https_bind.port())
            } else {
                app.clone()
            };

            info!(
                http_bind = %cfg.bind_addr,
                https_bind = %https_bind,
                domains = ?domains,
                staging,
                redirect = cfg.tls.http_redirect_https,
                profile = ?cfg.profile,
                "ferropod listening (HTTP + ACME HTTPS)"
            );

            let http_fut = axum_server::bind(cfg.bind_addr)
                .handle(http_handle)
                .serve(http_app.into_make_service());
            let https_fut = axum_server::bind(https_bind)
                .acceptor(acceptor)
                .handle(https_handle)
                .serve(app.into_make_service());

            let res = tokio::try_join!(http_fut, https_fut);
            acme_task.abort();
            res?;
            Ok(())
        }
    }
}

fn spawn_shutdown_bridge<F>(
    shutdown: F,
    http_handle: Handle<SocketAddr>,
    https_handle: Handle<SocketAddr>,
)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        shutdown.await;
        http_handle.graceful_shutdown(Some(Duration::from_secs(10)));
        https_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });
}
