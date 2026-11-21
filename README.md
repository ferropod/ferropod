<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/banner-dark.svg" />
    <source media="(prefers-color-scheme: light)" srcset="assets/banner-light.svg" />
    <img alt="ferropod — AI-Native PaaS in Pure Rust" src="assets/banner-dark.svg" width="100%" />
  </picture>
</p>

# ferropod

[![CI](https://github.com/ferropod/ferropod/actions/workflows/ci.yml/badge.svg)](https://github.com/ferropod/ferropod/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ferropod/ferropod?include_prereleases&label=Release)](https://github.com/ferropod/ferropod/releases)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache--2.0-orange.svg)](LICENSE)
[![Homepage](https://img.shields.io/badge/Homepage-ferropod.github.io-1f2937.svg)](https://ferropod.github.io)

**AI-Native lightweight self-hosted PaaS in Pure Rust.**  
Single binary + single `/data` directory. Designed to run comfortably even on a **512 MiB RAM** VPS.

- **Apps**. Deploy Wasm Workers on Wasmtime, serve static assets with SPA fallback, or combine both in one app. The built-in gateway can forward a path prefix to an HTTP backend on the same origin.
- **Turso Database (MVCC + `io_uring`)**: Dedicated isolated databases (`core.db`, `kv.db`, `queue.db`, per-project `dbs/<id>.db`, daily `logs/*.db`).
- **Native Streaming Blob Storage**: 5-endpoint HTTP Blob API (`PUT`/`GET`/`HEAD`/`DELETE`/`LIST`) with `HMAC-SHA256` presigned URLs, content-addressed files in `/data/blobs`, and async GC.
- **Persistent Queue & KV**: Disk-backed durability with bounded micro-batch ingestion, backpressure (`429`), lease recovery, exponential backoff, DLQ replay, TTL batch cleanup, and ephemeral in-memory KV mode.
- **Zero-Build Admin UI & Daily Logs**. Manage apps, deployments, routes, and resources at `/_panel`. Follow logs live via SSE or query the daily Turso log files.

## One-Click Install & Update

Prebuilt binaries for Linux (`x86_64`, `aarch64`) and macOS (`arm64`, `x86_64`) are published on [GitHub Releases](https://github.com/ferropod/ferropod/releases) with `SHA256SUMS.txt` verification.

```bash
# One-click install (Linux systemd + auto master key + initial admin token, or macOS binary)
curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash

# Install with built-in Let's Encrypt HTTPS on 80/443
curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash -s -- \
  --bind 0.0.0.0:80 \
  --https-bind 0.0.0.0:443 \
  --domain panel.example.com \
  --email ops@example.com \
  --redirect-https

# One-click update via script
curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash -s -- --update

# Or self-update directly via CLI (supports --check, --version vX.Y.Z, --force)
ferropod update --check
sudo ferropod update

# Uninstall (keeps /var/lib/ferropod unless --purge is passed)
curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash -s -- --uninstall
```

You can also check for updates, browse GitHub Releases, and trigger **one-click self-update & graceful restart** directly from the Admin Console at `/_panel` → **Updates** (`/_panel#/updates`).

## Quickstart (Build from Source)

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

# Local dev environment (.ferropod-dev; enables --wasm-allow-private-outbound)
ferropod dev

# Mint an admin API token
ferropod token create --name admin --scope admin

# Consistent database snapshot for off-site backup (rclone/restic)
ferropod snapshot --out ./data/backups/manual

# Print a systemd unit file (automatically adds CAP_NET_BIND_SERVICE for ports < 1024)
ferropod install-systemd --data-dir /var/lib/ferropod --bind 0.0.0.0:80 --https-bind 0.0.0.0:443 --acme-domain panel.example.com --http-redirect-https
```

## Deploy an App

1. Open `/_panel` and sign in with an admin token. Choose **Apps → New App**.
2. Open the app and choose **New Deployment**. For a static site, upload the
   built files to a bucket in **Object Storage** and select it under **Assets
   Bucket**. Set **Assets Prefix** if the files are under a folder such as
   `dist/`. For a Worker, use **Upload Worker file** to upload a Wasm component
   (`.wasm` or `.wat`, up to 64 MiB). A deployment can include both.
3. Choose **Production environment** to activate the deployment, or select the
   preview environment to test it first. Under **Routes → Add Route**, bind a
   hostname to the app and its target environment.

### Serve a Static Site with a Local HTTP API

Ferropod can serve the site and forward API requests from the same origin.
For example, start your HTTP backend on `127.0.0.1:3000`, then start Ferropod
with the site's hostname and the path prefix you want to forward.

```bash
ferropod serve --bind 127.0.0.1:8080 \
  --local-api-host 127.0.0.1 \
  --local-api-prefix /backend \
  --local-api-upstream 127.0.0.1:3000
```

Bind hostname `127.0.0.1` to the app's active environment under **Routes**.
Open `http://127.0.0.1:8080` and call the backend with a relative URL such
as `fetch('/backend/users')`. The gateway forwards `/backend` and its
subpaths before SPA fallback, preserving the full path and query string.

Configure one forwarding rule per Ferropod process with startup flags or
the equivalent environment variables.

| Flag | Environment variable | Value |
| --- | --- | --- |
| `--local-api-host` | `FERROPOD_LOCAL_API_HOST` | Hostname from **Routes**, without a port |
| `--local-api-prefix` | `FERROPOD_LOCAL_API_PREFIX` | Path prefix, defaults to `/api` |
| `--local-api-upstream` | `FERROPOD_LOCAL_API_UPSTREAM` | HTTP backend IP address and port |

The upstream must be reachable from Ferropod. In a container, `127.0.0.1`
refers to the container's own network namespace. For Worker isolation, outbound
`wasi:http` requests from Wasm Workers block private/loopback/local-network IPs
(and the `--local-api-upstream` address) by default; pass
`--wasm-allow-private-outbound` (or `FERROPOD_WASM_ALLOW_PRIVATE_OUTBOUND=true`)
on `ferropod serve` if Workers intentionally call private-network hosts.

## Docker Deployment

Single-container deployment with native HTTP/HTTPS ports:

```bash
export FERROPOD_MASTER_KEY="$(openssl rand -hex 32)"
docker compose up -d --build
```

- Exposes HTTP (`80`, configurable via `FERROPOD_HTTP_PORT` / `FERROPOD_BIND`) and HTTPS (`443`, configurable via `FERROPOD_HTTPS_PORT` / `FERROPOD_HTTPS_BIND`) with persistent state in `./data:/data` and `--profile low-mem`.
- **Built-in HTTPS**: configure `FERROPOD_ACME_DOMAINS` + `FERROPOD_ACME_EMAIL` for automatic Let's Encrypt (TLS-ALPN-01, cached under `/data/certs`, automatically including active project route domains), or set `FERROPOD_TLS_CERT` + `FERROPOD_TLS_KEY` for manual PEM certificates. You can also place `ferropod` behind Cloudflare Tunnel / Proxy or an existing reverse proxy if preferred.
- **Domain Binding**: bind a hostname to an app's production or preview environment via **Routes** or `POST /_api/v1/routes`.
- Note: Docker's default seccomp profile blocks `io_uring` syscalls on some hosts; Turso automatically falls back to standard async file I/O when `io_uring` is restricted, or you can relax seccomp in trusted environments.

## Documentation & Contracts

- Architecture Specification: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- WIT Host Binding Contract (`ferropod:platform@0.2.0`): [wit/platform.wit](wit/platform.wit)

## Roadmap

- **M1 (Done)**: Storage foundation (`turso` MVCC + Blob FS), native Blob API, static site routing, daily logs, maintenance & graceful shutdown, zero-build `/_panel` shell, REST API (`/_api/v1`), CLI.
- **M2 (Implemented)**: Wasmtime AOT `.cwasm` runtime, server-side Wasm compilation, strongly typed WIT host bindings (`ferropod:platform@0.2.0`), `wasi:http/outgoing-handler`, Queue/Cron worker dispatch, and built-in HTTP API forwarding for apps.
- **M3**: Native `rmcp` MCP server (`deploy_stack`, `worker_invoke`, `logs_query`, resource tools, audit trail).
- **M4**: Hardening, automated health checks & rollback, alerting webhooks, release packaging, and optional `--features search` full-text indexing.

## Deploy from a Git CI Pipeline

The deployment dialog accepts a built `.wasm` or `.wat` component (up to 64 MiB).
For Git-triggered deployments, let the Git provider run the build and then use
`scripts/deploy-wasm.py`. The script uses Python 3 and `FERROPOD_TOKEN`; it uploads
the component with a unique name and creates a deployment, retaining the current
app's bindings, environment, static assets, and cron declarations. It deploys to
preview by default. Use `--target prod` to activate the deployment in production
or `--target ready` to create a version for later activation.

Create a token scoped to the app in the management console and store it as a
masked CI secret. Never put it in the repository or a command-line argument. The
Ferropod URL must be reachable by the CI runner over HTTPS (HTTP is accepted only
for localhost testing). Failed uploads or deployment validation cause a nonzero
exit status. Uploaded artifacts can remain on disk if a later deployment fails.

For example, copy `scripts/deploy-wasm.py` into the application's repository and
adapt the build command and artifact path to its Wasm component:

```yaml
name: Build and deploy
on:
  push:
    branches: [main]
  workflow_dispatch:
permissions:
  contents: read
concurrency:
  group: ferropod-preview
  cancel-in-progress: false
jobs:
  deploy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: wasm32-wasip2
      - run: cargo build --release --target wasm32-wasip2
      - name: Deploy preview
        env:
          FERROPOD_TOKEN: ${{ secrets.FERROPOD_TOKEN }}
        run: >-
          python3 scripts/deploy-wasm.py
          --url '${{ vars.FERROPOD_URL }}'
          --app '${{ vars.FERROPOD_APP }}'
          --file target/wasm32-wasip2/release/worker.wasm
          --target preview
```

A private GitHub repository can use `actions/checkout` with the workflow's
read-only `GITHUB_TOKEN` for that repository. If the build needs another private
repository, configure a read-only deploy key or appropriately scoped CI secret
for that checkout. Other Git providers can run the same script from their push
pipeline. Git credentials stay with the provider. Use your project's build steps
to produce a Wasm component compatible with Ferropod's Worker interfaces.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
