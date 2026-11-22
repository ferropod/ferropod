use serde_json::{json, Value};

/// OpenAPI 3.1.0 specification describing FerroPod HTTP & AI endpoints.
pub fn openapi_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "FerroPod Control & Data Plane API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Single-binary self-hosted PaaS combining Wasmtime Workers, Turso MVCC SQL, KV, Queues, Blob Storage, Search, Realtime Pub-Sub, AI Gateway, and MCP."
        },
        "components": {
            "securitySchemes": {
                "bearerAuth": {
                    "type": "http",
                    "scheme": "bearer",
                    "bearerFormat": "fp_token"
                }
            }
        },
        "security": [{ "bearerAuth": [] }],
        "paths": {
            "/_healthz": {
                "get": {
                    "summary": "Unauthenticated liveness probe",
                    "security": [],
                    "responses": { "200": { "description": "Service is healthy" } }
                }
            },
            "/_api/v1/openapi.json": {
                "get": {
                    "summary": "OpenAPI 3.1.0 specification",
                    "security": [],
                    "responses": { "200": { "description": "OpenAPI document" } }
                }
            },
            "/_api/v1/llms.txt": {
                "get": {
                    "summary": "Concise AI agent handbook for FerroPod",
                    "security": [],
                    "responses": { "200": { "description": "Markdown handbook" } }
                }
            },
            "/_api/v1/status": {
                "get": {
                    "summary": "Platform runtime status & resource counts",
                    "responses": { "200": { "description": "Status summary" } }
                }
            },
            "/_api/v1/metrics": {
                "get": {
                    "summary": "Process & host telemetry (JSON)",
                    "responses": { "200": { "description": "Metrics snapshot" } }
                }
            },
            "/_api/v1/metrics/prometheus": {
                "get": {
                    "summary": "Prometheus text exposition telemetry",
                    "responses": { "200": { "description": "Prometheus text format" } }
                }
            },
            "/_api/v1/projects": {
                "get": { "summary": "List apps", "responses": { "200": { "description": "App list" } } },
                "post": { "summary": "Create app", "responses": { "201": { "description": "Created app" } } }
            },
            "/_api/v1/projects/{id}": {
                "get": { "summary": "Get app details", "responses": { "200": { "description": "App metadata" } } },
                "patch": { "summary": "Update app settings", "responses": { "200": { "description": "Updated app" } } },
                "delete": { "summary": "Delete app", "responses": { "204": { "description": "Deleted" } } }
            },
            "/_api/v1/projects/{id}/deployments": {
                "get": { "summary": "List app deployments", "responses": { "200": { "description": "Deployment history" } } },
                "post": { "summary": "Create immutable deployment", "responses": { "201": { "description": "Deployment created" } } }
            },
            "/_api/v1/projects/{id}/promote": {
                "post": { "summary": "Promote preview or deployment to production", "responses": { "200": { "description": "Active deployment updated" } } }
            },
            "/_api/v1/projects/{id}/rollback": {
                "post": { "summary": "Rollback production deployment", "responses": { "200": { "description": "Rolled back" } } }
            },
            "/_api/v1/projects/{id}/invoke": {
                "post": { "summary": "Invoke worker or static route directly", "responses": { "200": { "description": "Worker response" } } }
            },
            "/_api/v1/routes": {
                "get": { "summary": "List hostname routes", "responses": { "200": { "description": "Routes" } } },
                "post": { "summary": "Create or update hostname route", "responses": { "200": { "description": "Route saved" } } }
            },
            "/_api/v1/kv": {
                "get": { "summary": "List KV namespaces", "responses": { "200": { "description": "KV namespaces" } } },
                "post": { "summary": "Create KV namespace", "responses": { "201": { "description": "Created namespace" } } }
            },
            "/_api/v1/queues": {
                "get": { "summary": "List queues", "responses": { "200": { "description": "Queues" } } },
                "post": { "summary": "Create queue", "responses": { "201": { "description": "Created queue" } } }
            },
            "/_api/v1/buckets": {
                "get": { "summary": "List blob buckets", "responses": { "200": { "description": "Buckets" } } },
                "post": { "summary": "Create blob bucket", "responses": { "201": { "description": "Created bucket" } } }
            },
            "/_api/v1/dbs": {
                "get": { "summary": "List Turso user databases", "responses": { "200": { "description": "Databases" } } },
                "post": { "summary": "Create isolated user database", "responses": { "201": { "description": "Created database" } } }
            },
            "/_api/v1/search": {
                "get": { "summary": "List search indexes", "responses": { "200": { "description": "Search indexes" } } },
                "post": { "summary": "Create search index", "responses": { "201": { "description": "Created search index" } } }
            },
            "/_api/v1/crons": {
                "get": { "summary": "List scheduled cron jobs", "responses": { "200": { "description": "Cron jobs" } } },
                "post": { "summary": "Create scheduled cron job", "responses": { "201": { "description": "Created cron job" } } }
            },
            "/_api/v1/logs": {
                "get": { "summary": "Query daily logs", "responses": { "200": { "description": "Matching log entries" } } }
            },
            "/_api/v1/backups": {
                "get": { "summary": "List snapshot backups", "responses": { "200": { "description": "Backups" } } },
                "post": { "summary": "Create consistent snapshot backup", "responses": { "201": { "description": "Snapshot created" } } }
            },
            "/_api/v1/tokens": {
                "get": { "summary": "List API tokens", "responses": { "200": { "description": "Token metadata list" } } },
                "post": { "summary": "Create API token", "responses": { "201": { "description": "Raw token (shown once)" } } }
            },
            "/_api/v1/alerts/webhooks": {
                "get": { "summary": "List alert webhooks", "responses": { "200": { "description": "Alert webhooks" } } },
                "post": { "summary": "Create alert webhook", "responses": { "201": { "description": "Webhook created" } } }
            },
            "/_api/v1/ai/providers": {
                "get": { "summary": "List configured AI Gateway providers", "responses": { "200": { "description": "AI providers" } } },
                "post": { "summary": "Configure AI Gateway provider", "responses": { "201": { "description": "Provider saved" } } }
            },
            "/_ai/v1/chat/completions": {
                "post": { "summary": "OpenAI-compatible AI Gateway proxy with failover and cache", "responses": { "200": { "description": "Model completion response" } } }
            },
            "/_realtime/{channel}/sse": {
                "get": { "summary": "Subscribe to Realtime Pub-Sub channel over SSE", "responses": { "200": { "description": "Event stream" } } }
            },
            "/_mcp": {
                "post": { "summary": "Model Context Protocol (JSON-RPC 2.0) server endpoint", "responses": { "200": { "description": "JSON-RPC response" } } }
            },
            "/_blob/{bucket}/{key}": {
                "get": { "summary": "Download object (supports Range & ETag)", "responses": { "200": { "description": "Object bytes" } } },
                "put": { "summary": "Upload object", "responses": { "201": { "description": "Object metadata" } } },
                "delete": { "summary": "Delete object", "responses": { "200": { "description": "Deletion result" } } }
            }
        }
    })
}

/// Concise AI agent handbook served at `/_api/v1/llms.txt`.
pub fn llms_txt() -> &'static str {
    r#"# FerroPod — AI-Native Lightweight Self-Hosted PaaS

## Authentication
All control-plane APIs (`/_api/v1/*`), `/_mcp`, and `/_ai/v1/*` accept Bearer authentication:
  Authorization: Bearer fp_<token>

Generate an admin token on the server:
  ferropod token create --name agent --scope admin

## Core Workflows
- Health & Status: GET /_healthz (public), GET /_api/v1/status, GET /_api/v1/metrics/prometheus
- Apps & Deployments: POST /_api/v1/projects -> PUT /_api/v1/projects/{id}/artifacts/{file} -> POST /_api/v1/projects/{id}/deployments -> POST /_api/v1/routes
- Declarative CLI:
    ferropod init --template static --name my-app
    ferropod deploy --url http://127.0.0.1:8080 --token $FERROPOD_TOKEN --env prod
- Storage & Compute Primitives:
    - Turso User DBs: POST /_api/v1/dbs, POST /_api/v1/dbs/{id}/query, POST /_api/v1/dbs/{id}/execute
    - KV Store: GET/PUT /_api/v1/kv/{ns}/{key} (default TTL 7d; supports ephemeral in-memory mode)
    - Queues: POST /_api/v1/queues/{name}/enqueue, GET /_api/v1/queues/{name}/stats
    - Blob Storage: PUT/GET/HEAD/DELETE /_blob/{bucket}/{key}
    - Full-Text & Hybrid Search: POST /_api/v1/search/{id}/docs, POST /_api/v1/search/{id}/query

## MCP Server (`/_mcp` & `ferropod mcp`)
Streamable HTTP JSON-RPC endpoint at `/_mcp` (alias `/mcp`) and local CLI stdio via `ferropod mcp`.
Exposed high-semantic tools (10):
  1. platform_overview    - Inspect cluster status, apps, routes, queue depths/DLQ, and recent errors
  2. deploy_stack         - Idempotently provision app + resources + static/Wasm deployment + route
  3. promote_or_rollback  - Promote preview/version to prod or rollback to previous prod
  4. invoke_worker        - Execute an HTTP request against a deployed Worker/static app
  5. inspect_schema       - List user databases or inspect table DDL schema
  6. query_db             - Run parameterized SQL query or batch statements on a user DB
  7. diagnose_logs        - Search access/runtime/queue/cron/audit logs
  8. manage_kv            - Read, write, delete, or list keys in a KV namespace
  9. manage_queue         - Inspect queue stats, enqueue messages, view jobs, or replay DLQ
 10. search_query         - Query or upsert documents in a search index
"#
}
