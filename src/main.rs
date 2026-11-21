use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use ferropod::config::{Config, Profile, TlsConfig};
use ferropod::core::model::{Caller, TokenScope};
use ferropod::core::service::{PlatformDeps, PlatformService};
use ferropod::dispatcher::{maintenance::run_maintenance_loop, shutdown::shutdown_signal};
use ferropod::gateway::{router::build_router, tls::serve_http_and_https};
use ferropod::storage::{
    blob_fs::FsBlobStore, core_store::TursoCoreStore, kv_store::TursoKvStore, log_store::DailyLogStore,
    queue_store::TursoQueueStore, user_db::TursoUserDbManager,
};
use ferropod::update;

#[derive(Debug, Parser)]
#[command(name = "ferropod", version, about = "AI-Native lightweight self-hosted PaaS")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start the ferropod server.
    Serve {
        #[arg(long, env = "FERROPOD_PROFILE", default_value = "standard")]
        profile: Profile,
        /// HTTP listen address (e.g. `0.0.0.0:80` or `127.0.0.1:8080`).
        #[arg(long, env = "FERROPOD_BIND", default_value = "127.0.0.1:8080")]
        bind: SocketAddr,
        /// Optional HTTPS listen address (e.g. `0.0.0.0:443`). Defaults to `0.0.0.0:443` when TLS/ACME flags are set.
        #[arg(long, env = "FERROPOD_HTTPS_BIND")]
        https_bind: Option<SocketAddr>,
        /// PEM certificate chain path for manual HTTPS mode.
        #[arg(long, env = "FERROPOD_TLS_CERT")]
        tls_cert: Option<PathBuf>,
        /// PEM private key path for manual HTTPS mode.
        #[arg(long, env = "FERROPOD_TLS_KEY")]
        tls_key: Option<PathBuf>,
        /// Domain(s) for automatic Let's Encrypt HTTPS (repeatable or comma-separated).
        #[arg(long, env = "FERROPOD_ACME_DOMAINS", value_delimiter = ',')]
        acme_domain: Vec<String>,
        /// Contact email for Let's Encrypt ACME registration.
        #[arg(long, env = "FERROPOD_ACME_EMAIL")]
        acme_email: Option<String>,
        /// Use Let's Encrypt staging directory (for testing without hitting rate limits).
        #[arg(long, env = "FERROPOD_ACME_STAGING", default_value_t = false)]
        acme_staging: bool,
        /// Redirect HTTP requests on `--bind` to HTTPS (`308 Permanent Redirect`), keeping `/_healthz` on HTTP.
        #[arg(long, env = "FERROPOD_HTTP_REDIRECT_HTTPS", default_value_t = false)]
        http_redirect_https: bool,
        #[arg(long, env = "FERROPOD_DATA_DIR", default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, env = "FERROPOD_MASTER_KEY", default_value = "dev-insecure-master-key-change-me")]
        master_key: String,
    },
    /// Start a local development instance using ./.ferropod-dev and auto-create a dev admin token.
    Dev {
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: SocketAddr,
        #[arg(long, default_value = "./.ferropod-dev")]
        data_dir: PathBuf,
    },
    /// Manage API / MCP tokens.
    Token {
        #[command(subcommand)]
        action: TokenCommands,
    },
    /// Export consistent database snapshots for off-site backup.
    Snapshot {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, env = "FERROPOD_DATA_DIR", default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, env = "FERROPOD_MASTER_KEY", default_value = "dev-insecure-master-key-change-me")]
        master_key: String,
    },
    /// Print a ready-to-use systemd unit configuration.
    InstallSystemd {
        #[arg(long, default_value = "/var/lib/ferropod")]
        data_dir: PathBuf,
        #[arg(long, default_value = "0.0.0.0:80")]
        bind: SocketAddr,
        #[arg(long)]
        https_bind: Option<SocketAddr>,
        #[arg(long, value_delimiter = ',')]
        acme_domain: Vec<String>,
        #[arg(long)]
        acme_email: Option<String>,
        #[arg(long, default_value_t = false)]
        http_redirect_https: bool,
        #[arg(long, default_value = "low-mem")]
        profile: Profile,
        #[arg(long, default_value = "/usr/local/bin/ferropod")]
        bin_path: PathBuf,
    },
    /// Check for updates or self-update ferropod from GitHub Releases.
    #[command(visible_alias = "self-update")]
    Update {
        /// Only check if a newer release is available without installing.
        #[arg(long, default_value_t = false)]
        check: bool,
        /// Install a specific release tag (e.g. v0.2.0) instead of latest.
        #[arg(long)]
        version: Option<String>,
        /// Reinstall even if already running the target version.
        #[arg(long, default_value_t = false)]
        force: bool,
        /// Do not automatically restart the systemd ferropod service after updating.
        #[arg(long, default_value_t = false)]
        no_restart: bool,
    },
}

#[derive(Debug, Subcommand)]
enum TokenCommands {
    /// Create and print a new API / MCP token once.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "admin")]
        scope: String,
        #[arg(long)]
        expires_in_days: Option<i64>,
        #[arg(long, env = "FERROPOD_DATA_DIR", default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long, env = "FERROPOD_MASTER_KEY", default_value = "dev-insecure-master-key-change-me")]
        master_key: String,
    },
}

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ferropod=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let worker_threads = match &cli.command {
        Commands::Serve { profile, .. } => match profile {
            Profile::LowMem => 2,
            Profile::Standard => std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
        },
        _ => 2,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()?;

    rt.block_on(async_main(cli))
}

async fn init_service(cfg: &Config) -> Result<PlatformService> {
    tokio::fs::create_dir_all(&cfg.data_dir).await?;
    tokio::fs::create_dir_all(cfg.artifacts_dir()).await?;

    let core_store = Arc::new(TursoCoreStore::open(&cfg.core_db_path()).await?);
    let kv_store = Arc::new(
        TursoKvStore::open(&cfg.kv_db_path(), cfg.max_ephemeral_kv_entries).await?,
    );
    kv_store.migrate_from_legacy_core(core_store.database()).await?;
    let queue_store = Arc::new(TursoQueueStore::open(&cfg.queue_db_path()).await?);
    let user_dbs = Arc::new(TursoUserDbManager::new(
        cfg.user_dbs_dir(),
        cfg.max_open_user_dbs,
    )?);
    let blob_store = Arc::new(FsBlobStore::new(
        cfg.blobs_dir(),
        core_store.clone(),
        cfg.blob_gc_grace_secs,
    )?);
    let log_store = Arc::new(
        DailyLogStore::open(cfg.logs_dir(), cfg.log_channel_capacity).await?,
    );

    let svc = PlatformService::new(
        cfg.clone(),
        PlatformDeps {
            projects: core_store.clone(),
            secrets: core_store.clone(),
            tokens: core_store.clone(),
            registry: core_store.clone(),
            kv: kv_store,
            queue: queue_store,
            objects: blob_store,
            db: user_dbs,
            logs: log_store,
        },
    )
    .await?;

    Ok(svc)
}

async fn async_main(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Serve {
            profile,
            bind,
            https_bind,
            tls_cert,
            tls_key,
            acme_domain,
            acme_email,
            acme_staging,
            http_redirect_https,
            data_dir,
            master_key,
        } => {
            let mut cfg = Config::for_profile(profile, bind, data_dir, master_key);
            cfg.tls = TlsConfig {
                https_bind,
                cert_path: tls_cert,
                key_path: tls_key,
                acme_domains: acme_domain,
                acme_email,
                acme_staging,
                http_redirect_https,
            };
            run_server(cfg, false).await?;
        }
        Commands::Dev { bind, data_dir } => {
            let cfg = Config::for_profile(
                Profile::Standard,
                bind,
                data_dir,
                "dev-insecure-master-key-change-me".to_string(),
            );
            run_server(cfg, true).await?;
        }
        Commands::Token { action } => match action {
            TokenCommands::Create {
                name,
                scope,
                expires_in_days,
                data_dir,
                master_key,
            } => {
                let cfg = Config::for_profile(
                    Profile::Standard,
                    "127.0.0.1:8080".parse().unwrap(),
                    data_dir,
                    master_key,
                );
                let svc = init_service(&cfg).await?;
                let parsed_scope = TokenScope::parse(&scope)?;
                let expires_at =
                    expires_in_days.map(|d| chrono::Utc::now().timestamp() + d * 86_400);
                let caller = Caller::system("cli");
                let (meta, raw) = svc
                    .create_token(&caller, &name, parsed_scope, expires_at)
                    .await?;
                println!("Created token {} (scope={}):", meta.id, meta.scope.as_str());
                println!("{raw}");
                let _ = svc.flush_and_checkpoint().await;
            }
        },
        Commands::Snapshot {
            out,
            data_dir,
            master_key,
        } => {
            let cfg = Config::for_profile(
                Profile::Standard,
                "127.0.0.1:8080".parse().unwrap(),
                data_dir,
                master_key,
            );
            let svc = init_service(&cfg).await?;
            let caller = Caller::system("cli");
            let count = svc.create_snapshot(&caller, &out).await?;
            println!("Snapshot complete at {} ({count} user databases)", out.display());
        }
        Commands::InstallSystemd {
            data_dir,
            bind,
            https_bind,
            acme_domain,
            acme_email,
            http_redirect_https,
            profile,
            bin_path,
        } => {
            let prof_str = match profile {
                Profile::LowMem => "low-mem",
                Profile::Standard => "standard",
            };
            let mut exec = format!(
                "{} serve --profile {} --bind {} --data-dir {}",
                bin_path.display(),
                prof_str,
                bind,
                data_dir.display()
            );
            if let Some(hb) = https_bind {
                exec.push_str(&format!(" --https-bind {hb}"));
            }
            for d in &acme_domain {
                exec.push_str(&format!(" --acme-domain {d}"));
            }
            if let Some(email) = &acme_email {
                exec.push_str(&format!(" --acme-email {email}"));
            }
            if http_redirect_https {
                exec.push_str(" --http-redirect-https");
            }
            let needs_cap_bind = bind.port() < 1024
                || https_bind.map(|b| b.port() < 1024).unwrap_or(false)
                || !acme_domain.is_empty();
            let cap_line = if needs_cap_bind {
                "\nAmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE"
            } else {
                ""
            };
            println!(
                "[Unit]
Description=ferropod AI-Native Self-Hosted PaaS
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart={exec}
Environment=FERROPOD_MASTER_KEY=CHANGE_ME_TO_A_STRONG_RANDOM_SECRET
Restart=always
RestartSec=3
LimitNOFILE=65535{cap_line}

[Install]
WantedBy=multi-user.target"
            );
        }
        Commands::Update {
            check,
            version,
            force,
            no_restart,
        } => {
            if check {
                let res = update::check_for_update().await?;
                println!("Current version : v{}", res.current_version);
                println!("Target platform : {}", res.target);
                println!("Install mode    : {}", res.install_mode);
                match res.latest {
                    Some(rel) => {
                        println!("Latest release  : {} ({})", rel.tag, rel.html_url);
                        if res.update_available {
                            println!("Status          : Update available! Run `ferropod update` to upgrade.");
                        } else {
                            println!("Status          : Up to date.");
                        }
                    }
                    None => {
                        println!("Latest release  : none published yet in {}", res.repo);
                    }
                }
            } else {
                let _lock = update::UpdateLock::try_acquire()
                    .ok_or_else(|| anyhow::anyhow!("another update is already in progress"))?;
                println!(
                    "Checking GitHub Releases ({}) for {}...",
                    update::update_repo(),
                    version.as_deref().unwrap_or("latest")
                );
                let res = update::apply_update(version.as_deref(), force, false).await?;
                println!("{}", res.message);
                println!("Binary path     : {}", res.binary_path);
                if let Some(bak) = &res.backup_path {
                    println!("Backup saved to : {bak}");
                }
                if res.checksum_verified {
                    println!("SHA-256         : verified");
                }
                if !no_restart && res.new_version != res.previous_version {
                    #[cfg(target_os = "linux")]
                    {
                        let active = tokio::process::Command::new("systemctl")
                            .args(["is-active", "--quiet", "ferropod"])
                            .status()
                            .await
                            .map(|s| s.success())
                            .unwrap_or(false);
                        if active {
                            println!("Restarting systemd service `ferropod`...");
                            let st = tokio::process::Command::new("systemctl")
                                .args(["restart", "ferropod"])
                                .status()
                                .await;
                            match st {
                                Ok(s) if s.success() => println!("Service `ferropod` restarted."),
                                Ok(s) => eprintln!("Warning: `systemctl restart ferropod` exited with {s}"),
                                Err(e) => eprintln!("Warning: failed to restart systemd service: {e}"),
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn run_server(cfg: Config, dev_mode: bool) -> Result<()> {
    let svc = init_service(&cfg).await?;
    let dev_caller = Caller::system("dev");

    if dev_mode && svc.list_tokens(&dev_caller).await?.is_empty() {
        let (_, raw) = svc
            .create_token(&dev_caller, "dev-admin", TokenScope::Admin, None)
            .await?;
        info!(token = %raw, "dev mode initialized admin token");
        println!("DEV ADMIN TOKEN: {raw}");
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let maint_handle = tokio::spawn(run_maintenance_loop(svc.clone(), shutdown_rx));
    let mut restart_rx = update::init_restart_channel();

    let app = build_router(svc.clone());
    let stop_fut = async move {
        tokio::select! {
            _ = shutdown_signal() => {}
            _ = async {
                while !*restart_rx.borrow() {
                    if restart_rx.changed().await.is_err() {
                        std::future::pending::<()>().await;
                    }
                }
            } => {
                info!("received self-update restart signal, initiating graceful shutdown");
            }
        }
    };
    serve_http_and_https(&cfg, &svc, app, stop_fut).await?;

    let _ = shutdown_tx.send(true);
    let _ = maint_handle.await;
    let _ = svc.flush_and_checkpoint().await;
    info!("ferropod shutdown complete");

    if update::is_restart_requested() {
        info!("re-executing updated ferropod binary");
        update::exec_self_restart()?;
    }

    Ok(())
}
