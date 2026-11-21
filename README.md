<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/banner-dark.svg" />
    <source media="(prefers-color-scheme: light)" srcset="assets/banner-light.svg" />
    <img alt="ferropod — AI-Native PaaS in Pure Rust" src="assets/banner-dark.svg" width="100%" />
  </picture>
</p>

# ferropod

[![CI](https://github.com/ferropod/ferropod/actions/workflows/ci.yml/badge.svg)](https://github.com/ferropod/ferropod/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-orange.svg)](LICENSE)
[![Homepage](https://img.shields.io/badge/Homepage-ferropod.github.io-1f2937.svg)](https://ferropod.github.io)

**AI-Native lightweight self-hosted PaaS in Pure Rust.**  
Single binary + single `/data` directory. Designed to run comfortably even on a **512 MiB RAM** VPS.

- **Workers & Static Sites**: Wasmtime (`wasi:http/incoming-handler@0.2.0`) with WIT Host Bindings + zero-overhead static asset hosting (SPA fallback, Range, ETag/304).
- **Turso Database (MVCC + `io_uring`)**: Dedicated isolated databases (`core.db`, `queue.db`, per-project `dbs/<id>.db`, daily `logs/*.db`).
- **Native Streaming Blob Storage**: 5-endpoint HTTP Blob API (`PUT`/`GET`/`HEAD`/`DELETE`/`LIST`) with `HMAC-SHA256` presigned URLs, content-addressed files in `/data/blobs`, and async GC.
- **Persistent Queue & KV**: Disk-backed durability with bounded micro-batch ingestion, backpressure (`429`), lease recovery, exponential backoff, DLQ replay, TTL batch cleanup, and ephemeral in-memory KV mode.
- **Zero-Build Admin UI & Daily Logs**: Embedded HTML/HTMX dashboard at `/_admin`, SSE live tail, and daily rotating Turso log files.
- **Native MCP Server (M3)**: Built-in `rmcp` tools/resources so AI agents can deploy, test, inspect logs, and promote/rollback safely.

## Quickstart

```bash
# Build
cargo build --release

# Start server (standard profile)
ferropod serve --data-dir ./data --bind 127.0.0.1:8080

# Start on a 512 MiB VPS
ferropod serve --profile low-mem --data-dir ./data --bind 127.0.0.1:8080

# Start on 80/443 with automatic Let's Encrypt HTTPS (no Caddy/Nginx required)
ferropod serve --profile low-mem \
  --bind 0.0.0.0:80 \
  --https-bind 0.0.0.0:443 \
  --acme-domain panel.example.com \
  --acme-domain app.example.com \
  --acme-email ops@example.com \
  --http-redirect-https \
  --data-dir ./data

# Or use custom ports + manual PEM certificate/key
ferropod serve --profile low-mem \
  --bind 0.0.0.0:8080 \
  --https-bind 0.0.0.0:8443 \
  --tls-cert /etc/ssl/fullchain.pem \
  --tls-key /etc/ssl/privkey.pem \
  --http-redirect-https \
  --data-dir ./data

# Local dev environment (.ferropod-dev)
ferropod dev

# Mint an admin API token
ferropod token create --name admin --scope admin

# Consistent database snapshot for off-site backup (rclone/restic)
ferropod snapshot --out ./data/backups/manual

# Print a systemd unit file (automatically adds CAP_NET_BIND_SERVICE for ports < 1024)
ferropod install-systemd --data-dir /var/lib/ferropod --bind 0.0.0.0:80 --https-bind 0.0.0.0:443 --acme-domain panel.example.com --http-redirect-https
```

## Docker Deployment

Single-container deployment with native HTTP/HTTPS ports:

```bash
export FERROPOD_MASTER_KEY="$(openssl rand -hex 32)"
docker compose up -d --build
```

- Exposes HTTP (`80`, configurable via `FERROPOD_HTTP_PORT` / `FERROPOD_BIND`) and HTTPS (`443`, configurable via `FERROPOD_HTTPS_PORT` / `FERROPOD_HTTPS_BIND`) with persistent state in `./data:/data` and `--profile low-mem`.
- **Built-in HTTPS**: configure `FERROPOD_ACME_DOMAINS` + `FERROPOD_ACME_EMAIL` for automatic Let's Encrypt (TLS-ALPN-01, cached under `/data/certs`, automatically including active project route domains), or set `FERROPOD_TLS_CERT` + `FERROPOD_TLS_KEY` for manual PEM certificates. You can also place `ferropod` behind Cloudflare Tunnel / Proxy or an existing reverse proxy if preferred.
- **Domain Binding**: bind any domain to a project/environment via `/_admin` (Routes) or `PUT /_api/v1/routes`.
- Note: Docker's default seccomp profile blocks `io_uring` syscalls on some hosts; Turso automatically falls back to standard async file I/O when `io_uring` is restricted, or you can relax seccomp in trusted environments.

## Documentation & Contracts

- Architecture Specification: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- WIT Host Binding Contract: [wit/platform.wit](wit/platform.wit)

## Roadmap

- **M1 (Current)**: Storage foundation (`turso` MVCC + Blob FS), native Blob API, static site routing, daily logs, maintenance & graceful shutdown, zero-build `/_admin` shell, REST API (`/_api/v1`), CLI.
- **M2**: Wasmtime AOT `.cwasm` runtime + WIT host bindings + Queue/Cron HTTP dispatcher.
- **M3**: Native `rmcp` MCP server (`deploy_stack`, `worker_invoke`, `logs_query`, resource tools, audit trail).
- **M4**: Hardening, automated health checks & rollback, alerting webhooks, release packaging.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
