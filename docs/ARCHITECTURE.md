# ferropod Architecture Specification (v1.0)

> **Positioning**: AI-Native lightweight self-hosted PaaS in Pure Rust.  
> **Form Factor**: Single Rust binary (`ferropod`) + single `/data` directory. Zero external DB/MQ/S3/ELK/Node.js dependencies. Ready for **512 MiB RAM** VPS deployments.

---

## 1. Design Goals & Constraints

| Dimension | Design Principle |
| --- | --- |
| **Deployment** | One binary (`ferropod`: `serve`, `dev`, `token`, `snapshot`, `install-systemd`) + one `/data` folder. |
| **Memory Budget** | Idle RSS ~30–60 MiB; `--profile low-mem` caps concurrency, user DB LRU count, and channel sizes to run steadily within **512 MiB RAM**. |
| **Disk-First Storage** | Disk (`turso` + content-addressed `/data/blobs`) is the source of truth; memory holds only bounded buffers, hot route table, and OS page cache. |
| **1 Core Service + 3 Thin Adapters** | All business logic, token scope validation, and audit logging live once in `PlatformService`, exposed via `/_api/v1/*` (+ `/_panel`), `/mcp` (M3), and WIT Host Bindings (M2). |
| **Evolvability** | Every subsystem operates behind `core::traits` (`MetaStore`, `BlobMetaStore`, `KvStore`, `QueueStore`, `ObjectStore`, `DbManager`, `LogStore`, `WorkerRuntime`) so single-node engines can later swap to distributed backends without breaking Workers or APIs. |

---

## 2. System Layers

```text
  Browsers / APIs           Blob Clients / Uploads       Admin / CLI / CI / Scripts     AI Agents (M3)
  *.app.domain              /_blob/* (HMAC / Bearer)     /_api/v1/* (Bearer)            /mcp / stdio
         │                            │                            │                          │
         └────────────────────────────┼────────────────────────────┼──────────────────────────┘
                                      ▼
                    ┌───────────────────────────────────────────┐
                    │  Native Rust TLS (rustls / rustls-acme)   │
                    │  HTTP :80 (+308 redirect) & HTTPS :443    │
                    └─────────────────────┬─────────────────────┘
                                          ▼
┌─────────────────────────────────────────────────────────────────────────────────────────────────┐
│                             ferro/ferropod (Single Pure Rust Binary)                             │
│                                                                                                 │
│  Gateway (axum + tower): Host router, static site streaming (SPA fallback), 5-endpoint Blob,    │
│                          native TLS/ACME, request_id & access logs, Bearer scope auth           │
│                                         │                                                       │
│              ┌──────────────────────────┼──────────────────────────┐                            │
│              ▼                          ▼                          ▼                            │
│     Web / REST / Admin           MCP Server (M3)           Dispatcher & Maintenance             │
│     /_api/v1/* + /_panel         rmcp HTTP & stdio         Queue batch ingest + worker,         │
│     (HTMX/HTML zero-build)                                 Cron, TTL/DLQ/Blob/Log cleanup       │
│              │                          │                          │                            │
│              └──────────────────────────┼──────────────────────────┘                            │
│                                         ▼                                                       │
│                          PlatformService (Domain Orchestration)                                 │
│                                         │                                                       │
│                          Wasmtime Runtime (M2 · WIT Host Bindings)                              │
│                                         │                                                       │
│     core::traits (MetaStore | BlobMetaStore | KvStore | QueueStore | ObjectStore | DbManager)  │
│                                         │                                                       │
│  Storage Engine (Turso MVCC + Local Streaming FS):                                              │
│   • core.db (MVCC)    : projects, deployments, routes, secrets (AES-GCM), tokens, kv/ns, blob   │
│   • kv.db (MVCC)      : isolated user KV entries (persistent + TTL + ephemeral in-memory mode)  │
│   • queue.db (MVCC)   : queues, jobs (isolated checkpointing & 4-layer peak shaving)            │
│   • dbs/<id>.db       : per-project isolated databases (LRU connection cache + per-DB DDL lock) │
│   • logs/logs_YYYY_MM_DD.db : daily rotating log DBs (drop expired files via fs::remove_file)   │
│   • blobs/<h0>/<h1>/<sha256>: streaming content-addressed files (64 KiB chunks, Range, GC)      │
└─────────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Single-Crate Modular Structure

```text
ferropod/
├── Cargo.toml
├── wit/platform.wit
├── docs/ARCHITECTURE.md
└── src/
    ├── lib.rs / main.rs       # CLI entry & runtime bootstrap
    ├── config.rs              # Profile presets (low-mem / standard) & TLS settings
    ├── core/                  # model, traits, crypto (AES-256-GCM / SHA256 / HMAC), PlatformService
    ├── runtime/               # Wasmtime AOT .cwasm runtime, WIT v0.2 host bindings, outbound HTTP
    ├── storage/               # Turso pool & tx retry, core_store, kv_store, queue_store, user_db, blob_fs
    ├── logs/                  # DailyLogStore (bounded mpsc -> SSE broadcast + micro-batch daily DBs)
    ├── dispatcher/            # Background maintenance & graceful shutdown
    ├── gateway/               # Axum router, static hosting, native Blob API, native TLS/ACME, guard
    └── web/                   # /_api/v1 REST/SSE & zero-build /_panel dashboard
```

---

## 4. Storage Isolation & Retention Policies

| Store | Contents | Concurrency Strategy | Retention & Cleanup |
| --- | --- | --- | --- |
| `core.db` | Projects, deployments, routes, encrypted secrets, API tokens, KV namespace ownership, Blob metadata & GC queue, user DB registry, cron jobs | Turso MVCC (`BEGIN CONCURRENT` + backoff retry); hot routing table cached in `Arc<RwLock<HashMap>>` | Retained permanently; consistent backups via `ferropod snapshot` |
| `kv.db` | User KV entries (`kv_entries`) | Turso MVCC isolated from `core.db` + bounded in-memory ephemeral cache | KV without TTL kept indefinitely; KV with TTL logically filtered on read + deleted in `LIMIT 500` batches |
| `queue.db` | Queues & jobs | Bounded `mpsc(4096)` micro-batch ingestion (`10ms` or `200` items per tx) + `Notify` wakeup + ordered per-queue `claim_batch` | ACK deletes immediately; pending max retention (`7d`); lease reaper (`60s`); DLQ retained (`7d`) then batch deleted |
| `dbs/<id>.db` | Per-project isolated Turso databases | MVCC concurrent DML; per-DB `RwLock` write lock for DDL; LRU open-DB cap (`10` on `low-mem`) | Retained permanently; consistent backups via `ferropod snapshot` |
| `logs/logs_YYYY_MM_DD.db` | Access, runtime, queue, cron, audit, system logs | Non-blocking `try_send` (drops `debug` under backpressure) + 500ms/200-entry batch commit | Default `7` days; expired day files deleted directly via `fs::remove_file` |
| `/data/blobs/` | Static assets & Blob object payloads | Streaming 64 KiB chunks; atomic temp-file write + rename | Unreferenced `sha256` entries queued to `blob_gc` and deleted asynchronously |

---

## 5. Four-Layer Single-Node Queue Peak Shaving

1. **Micro-Batch Ingestion + Backpressure**: Producers push into a bounded `mpsc(4096)`. A dedicated `BatchIngestor` flushes up to `200` items or every `10ms` in a single transaction, collapsing thousands of writes/sec into ~100 commits/sec while returning `429` if saturated.
2. **Disk Reservoir**: Pending messages wait safely in `queue.db`, keeping RAM flat.
3. **Controlled Dispatch**: `Notify` wakes per-queue workers; a `Semaphore` caps concurrent Wasmtime executions so downstream DBs/APIs aren't overwhelmed.
4. **Foreground Priority**: High foreground load or memory pressure pauses background queue pulling while ingestion remains durable.

---

## 6. Security & Access Control

- **Secrets**: Encrypted at rest with `AES-256-GCM` using a key derived from `MASTER_KEY` and random 12-byte nonces.
- **API & MCP Tokens**: Only `SHA-256` hashes are stored in `core.db`; verified with scope checks (`admin`, `project:<id>`, `readonly`).
- **Blob Presigned URLs**: Stateless `HMAC-SHA256` over `op:bucket:key:exp`, verified with constant-time comparison.
- **Native TLS / ACME**: Built-in `rustls` / `rustls-acme` (TLS-ALPN-01) handles HTTPS directly on configurable ports (`80`/`443` or custom), caching certificates under `/data/certs` and merging configured `--acme-domain` entries with bound project route domains.

---

## 7. M2–M4 Evolution

- **M2 (Implemented)**: `runtime/wasmtime.rs` implementing `WorkerRuntime` with AOT `.cwasm` mmap loading, server-side Wasm compilation, `StoreLimits`, `epoch_interruption`, strongly-typed WIT host bindings (`ferropod:platform@0.2.0` in `wit/platform.wit`), `wasi:http/outgoing-handler`, and direct Queue/Cron execution (`queue-consumer` / `scheduled` with HTTP fallback).
- **M3**: Mount `rmcp` `StreamableHttpService` at `/mcp` and `ferropod mcp` stdio mode exposing `deploy_stack`, `deployment_promote`, `deployment_rollback`, `worker_invoke`, `logs_query`, and full resource tools.
- **M4**: Harden health-check auto-rollback, webhook alerting, release binaries, and optional v2 multi-tenant / distributed trait implementations (Turso Embedded Replicas, NATS/Valkey, RustFS/Garage).
- **Post-M4 (Optional `--features search`)**: Embedded Tantivy / `turso/fts` full-text search kept opt-in to protect the 512 MiB RAM budget by default.
