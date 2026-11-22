use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::model::{Bindings, TargetEnv};

/// Top-level declarative application manifest (`ferropod.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub app: AppSpec,
    #[serde(default)]
    pub worker: Option<WorkerSpec>,
    #[serde(default)]
    pub assets: Option<AssetsSpec>,
    #[serde(default)]
    pub bindings: Bindings,
    #[serde(default)]
    pub routes: Vec<RouteSpec>,
    #[serde(default)]
    pub migrations: Vec<MigrationSpec>,
    #[serde(default)]
    pub crons: Vec<CronSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSpec {
    pub name: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub health_check_path: Option<String>,
    #[serde(default)]
    pub max_concurrency: Option<i64>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub wasm_max_memory_mb: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerSpec {
    pub artifact: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetsSpec {
    pub bucket: String,
    #[serde(default)]
    pub dir: Option<PathBuf>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub spa_fallback: bool,
    #[serde(default = "default_true")]
    pub public_read: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSpec {
    pub host: String,
    #[serde(default = "default_prod")]
    pub env: TargetEnv,
}

fn default_prod() -> TargetEnv {
    TargetEnv::Prod
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationSpec {
    pub db: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSpec {
    pub name: String,
    pub schedule: String,
    #[serde(default = "default_cron_target")]
    pub target: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_cron_target() -> String {
    "cron".to_string()
}

/// Options controlling `ferropod deploy`.
#[derive(Debug, Clone)]
pub struct DeployOptions {
    pub config_path: PathBuf,
    pub server_url: String,
    pub token: String,
    pub target_env: TargetEnv,
    pub dry_run: bool,
}

/// Summary returned after executing `ferropod deploy`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeploySummary {
    pub dry_run: bool,
    pub project_id: String,
    pub project_name: String,
    pub deployment_id: Option<String>,
    pub target_env: String,
    pub assets_uploaded: usize,
    pub assets_skipped: usize,
    pub migrations_applied: usize,
    pub routes_bound: usize,
    pub crons_synced: usize,
    pub planned_actions: Vec<String>,
}

/// Scaffold a starter FerroPod project (`static`, `rust`, or `ts`), refusing to overwrite existing files.
pub fn init_project(dir: &Path, name: &str, template: &str) -> Result<Vec<PathBuf>> {
    init_project_with_force(dir, name, template, false)
}

/// Scaffold a starter FerroPod project (`static`, `rust`, or `ts`).
/// When `force` is `false`, aborts before writing anything if any target file already exists.
pub fn init_project_with_force(
    dir: &Path,
    name: &str,
    template: &str,
    force: bool,
) -> Result<Vec<PathBuf>> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        anyhow::bail!("project name cannot be empty");
    }

    let bucket_name = format!("{clean_name}-assets");
    let db_name = format!("{clean_name}-db");
    let kv_name = format!("{clean_name}-kv");

    let mut planned: Vec<(PathBuf, String)> = vec![
        (
            dir.join("public/index.html"),
            format!(
                "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{clean_name}</title></head>\n<body><h1>Hello from {clean_name}</h1><p>Deployed declaratively with ferropod.toml.</p></body>\n</html>\n"
            ),
        ),
        (
            dir.join("migrations/0001_init.sql"),
            "CREATE TABLE IF NOT EXISTS notes (\n  id INTEGER PRIMARY KEY AUTOINCREMENT,\n  title TEXT NOT NULL,\n  created_at INTEGER NOT NULL\n);\n"
                .to_string(),
        ),
    ];

    let worker_block = match template {
        "static" => String::new(),
        "rust" => {
            planned.push((
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname = \"{clean_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n"
                ),
            ));
            planned.push((
                dir.join("src/lib.rs"),
                "// Build with: cargo build --target wasm32-wasip2 --release\n// Export wasi:http/incoming-handler using ferropod:platform@0.2.0 bindings.\n"
                    .to_string(),
            ));
            format!(
                "\n[worker]\nartifact = \"target/wasm32-wasip2/release/{}.wasm\"\n",
                clean_name.replace('-', "_")
            )
        }
        "ts" => {
            planned.push((
                dir.join("worker.ts"),
                "// Bundle to wasm32-wasip2 component using jco / ComponentizeJS.\nexport const handler = {\n  fetch(req: Request): Response {\n    return new Response(JSON.stringify({ ok: true, url: req.url }), {\n      headers: { \"content-type\": \"application/json\" },\n    });\n  },\n};\n"
                    .to_string(),
            ));
            "\n[worker]\nartifact = \"dist/worker.wasm\"\n".to_string()
        }
        other => anyhow::bail!("unknown template '{other}', expected 'static', 'rust', or 'ts'"),
    };

    let manifest_path = dir.join("ferropod.toml");
    let toml_body = format!(
        "[app]\nname = \"{clean_name}\"\ndescription = \"FerroPod {template} application\"\nhealth_check_path = \"/_healthz\"\n{worker_block}\n[assets]\nbucket = \"{bucket_name}\"\ndir = \"public\"\nspa_fallback = true\npublic_read = true\n\n[bindings.kv]\nCACHE = \"{kv_name}\"\n\n[bindings.db]\nDB = \"{db_name}\"\n\n[[migrations]]\ndb = \"{db_name}\"\ndir = \"migrations\"\n\n[[routes]]\nhost = \"{clean_name}.local\"\nenv = \"prod\"\n"
    );
    planned.push((manifest_path, toml_body));

    if !force {
        let existing: Vec<String> = planned
            .iter()
            .filter(|(p, _)| p.exists())
            .map(|(p, _)| p.display().to_string())
            .collect();
        if !existing.is_empty() {
            anyhow::bail!(
                "refusing to overwrite existing file(s): {} (pass --force to overwrite)",
                existing.join(", ")
            );
        }
    }

    std::fs::create_dir_all(dir)?;
    let mut created = Vec::with_capacity(planned.len());
    for (path, content) in planned {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        created.push(path);
    }

    Ok(created)
}

fn collect_files_recursive(
    root: &Path,
    cur: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<()> {
    let entries = std::fs::read_dir(cur)
        .with_context(|| format!("failed to read assets directory {}", cur.display()))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            collect_files_recursive(root, &path, out)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Build an immutable content-addressed remote filename for a worker artifact
/// (e.g. `app.wasm` -> `app-a1b2c3d4e5f6.wasm`) so new deployments never overwrite old releases.
fn versioned_artifact_name(local_path: &Path, bytes: &[u8]) -> String {
    let digest = sha256_hex(bytes);
    let short = &digest[..12];
    let stem = local_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("worker");
    match local_path.extension().and_then(|e| e.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem}-{short}.{ext}"),
        _ => format!("{stem}-{short}"),
    }
}

/// Compute a deterministic release prefix from the sorted `(rel_key, sha256)` file set
/// so static deployments are isolated per content version while identical deploys stay deduplicated.
fn compute_release_prefix(base_prefix: Option<&str>, file_digests: &[(String, String)]) -> String {
    let mut hasher = Sha256::new();
    for (key, digest) in file_digests {
        hasher.update(key.as_bytes());
        hasher.update(b":");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    let rel_id = &hex::encode(hasher.finalize())[..12];
    match base_prefix
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
    {
        Some(pfx) => format!("{pfx}/_releases/rel_{rel_id}"),
        None => format!("_releases/rel_{rel_id}"),
    }
}

/// Split a SQL migration script into individual executable statements.
/// Handles single/double/backtick/bracket quotes, `--` and `/* */` comments,
/// and `BEGIN ... END` blocks (such as SQLite/Turso triggers) without splitting inside them.
pub fn split_sql_statements(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    let mut start = 0usize;
    let mut out = Vec::new();
    let mut begin_depth: usize = 0;

    while i < len {
        let b = bytes[i];
        // Line comment: -- ... \n
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment: /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }
        // Quoted strings / identifiers
        if b == b'\'' || b == b'"' || b == b'`' {
            let quote = b;
            i += 1;
            while i < len {
                if bytes[i] == quote {
                    if i + 1 < len && bytes[i + 1] == quote {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if b == b'[' {
            i += 1;
            while i < len && bytes[i] != b']' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            continue;
        }
        // Track BEGIN / END keywords for trigger blocks
        if b.is_ascii_alphabetic() || b == b'_' {
            let word_start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &sql[word_start..i];
            if word.eq_ignore_ascii_case("BEGIN") {
                begin_depth += 1;
            } else if word.eq_ignore_ascii_case("END") && begin_depth > 0 {
                begin_depth -= 1;
            }
            continue;
        }
        if b == b';' && begin_depth == 0 {
            let chunk = sql[start..i].trim();
            if has_executable_sql(chunk) {
                out.push(chunk.to_string());
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }

    let tail = sql[start..].trim();
    if has_executable_sql(tail) {
        out.push(tail.to_string());
    }
    out
}

fn has_executable_sql(chunk: &str) -> bool {
    let bytes = chunk.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }
        return true;
    }
    false
}

fn encode_blob_key(key: &str) -> String {
    key.split('/')
        .map(|seg| {
            seg.bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                        (b as char).to_string()
                    } else {
                        format!("%{b:02X}")
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/")
}

async fn check_status(resp: reqwest::Response, ctx: &str) -> Result<serde_json::Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{ctx} failed with HTTP {status}: {text}");
    }
    if text.trim().is_empty() {
        return Ok(serde_json::Value::Null);
    }
    serde_json::from_str(&text).with_context(|| format!("{ctx} returned invalid JSON: {text}"))
}

/// Parse `ferropod.toml` and idempotently deploy the stack to a FerroPod server.
pub async fn deploy_manifest(opts: DeployOptions) -> Result<DeploySummary> {
    let raw_toml = std::fs::read_to_string(&opts.config_path)
        .with_context(|| format!("failed to read {}", opts.config_path.display()))?;
    let manifest: Manifest = toml::from_str(&raw_toml)
        .with_context(|| format!("failed to parse {}", opts.config_path.display()))?;

    let base_dir = opts
        .config_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let server = opts.server_url.trim_end_matches('/').to_string();

    let mut summary = DeploySummary {
        dry_run: opts.dry_run,
        project_name: manifest.app.name.clone(),
        target_env: opts.target_env.as_str().to_string(),
        ..Default::default()
    };

    if opts.dry_run {
        summary
            .planned_actions
            .push(format!("ensure app '{}'", manifest.app.name));
        for kv in manifest.bindings.kv.values() {
            summary
                .planned_actions
                .push(format!("ensure kv namespace '{kv}'"));
        }
        for db in manifest.bindings.db.values() {
            summary
                .planned_actions
                .push(format!("ensure user database '{db}'"));
        }
        for mig in &manifest.migrations {
            summary.planned_actions.push(format!(
                "apply SQL migrations from '{}' to '{}'",
                mig.dir.display(),
                mig.db
            ));
        }
        if let Some(assets) = &manifest.assets {
            summary
                .planned_actions
                .push(format!("sync static assets to bucket '{}'", assets.bucket));
        }
        if let Some(worker) = &manifest.worker {
            summary
                .planned_actions
                .push(format!("upload worker '{}'", worker.artifact.display()));
        }
        for r in &manifest.routes {
            summary
                .planned_actions
                .push(format!("bind route '{}' -> {}", r.host, r.env.as_str()));
        }
        for c in &manifest.crons {
            summary
                .planned_actions
                .push(format!("sync cron '{}' ({})", c.name, c.schedule));
        }
        return Ok(summary);
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let auth_header = format!("Bearer {}", opts.token.trim());

    // 1. Ensure app exists
    let get_proj = client
        .get(format!("{server}/_api/v1/projects/{}", manifest.app.name))
        .header("Authorization", &auth_header)
        .send()
        .await?;
    let project_json = if get_proj.status() == StatusCode::NOT_FOUND {
        let resp = client
            .post(format!("{server}/_api/v1/projects"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "name": manifest.app.name,
                "group": manifest.app.group,
                "description": manifest.app.description,
                "health_check_path": manifest.app.health_check_path,
                "max_concurrency": manifest.app.max_concurrency,
                "timeout_ms": manifest.app.timeout_ms,
                "wasm_max_memory_mb": manifest.app.wasm_max_memory_mb,
            }))
            .send()
            .await?;
        check_status(resp, "create app").await?
    } else {
        let existing = check_status(get_proj, "fetch app").await?;
        let pid = existing["id"]
            .as_str()
            .ok_or_else(|| anyhow!("missing project id in response"))?;
        let resp = client
            .patch(format!("{server}/_api/v1/projects/{pid}"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "group": manifest.app.group,
                "description": manifest.app.description,
                "health_check_path": manifest.app.health_check_path,
                "max_concurrency": manifest.app.max_concurrency,
                "timeout_ms": manifest.app.timeout_ms,
                "wasm_max_memory_mb": manifest.app.wasm_max_memory_mb,
            }))
            .send()
            .await?;
        check_status(resp, "update app").await?
    };

    let project_id = project_json["id"]
        .as_str()
        .ok_or_else(|| anyhow!("missing project id"))?
        .to_string();
    summary.project_id = project_id.clone();

    // 2. Ensure KV namespaces, Queues, Buckets, Search Indexes, and User DBs exist
    for kv_name in manifest.bindings.kv.values() {
        let resp = client
            .post(format!("{server}/_api/v1/kv"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "name": kv_name,
                "project_id": project_id,
                "group": manifest.app.group,
            }))
            .send()
            .await?;
        if resp.status() != StatusCode::CONFLICT {
            check_status(resp, &format!("create kv '{kv_name}'")).await?;
        }
    }

    for q_name in manifest.bindings.queue.values() {
        let resp = client
            .post(format!("{server}/_api/v1/queues"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "name": q_name,
                "project_id": project_id,
                "group": manifest.app.group,
            }))
            .send()
            .await?;
        if resp.status() != StatusCode::CONFLICT {
            check_status(resp, &format!("create queue '{q_name}'")).await?;
        }
    }

    // Provision & reconcile buckets (deduplicated; update public_read if existing bucket differs)
    let mut desired_buckets: HashMap<&str, bool> = HashMap::new();
    for b_name in manifest.bindings.bucket.values() {
        desired_buckets.entry(b_name.as_str()).or_insert(false);
    }
    if let Some(a) = &manifest.assets {
        desired_buckets.insert(a.bucket.as_str(), a.public_read);
    }
    if !desired_buckets.is_empty() {
        let existing_buckets_val = check_status(
            client
                .get(format!("{server}/_api/v1/buckets"))
                .header("Authorization", &auth_header)
                .send()
                .await?,
            "list buckets",
        )
        .await?;
        let mut existing_buckets: HashMap<String, (Option<String>, bool)> = HashMap::new();
        if let Some(arr) = existing_buckets_val.as_array() {
            for item in arr {
                if let Some(name) = item["name"].as_str() {
                    let owner = item["project_id"].as_str().map(str::to_string);
                    let pub_read = item["public_read"].as_bool().unwrap_or(false);
                    existing_buckets.insert(name.to_string(), (owner, pub_read));
                }
            }
        }
        for (b_name, public_read) in desired_buckets {
            if let Some((owner_pid, cur_pub)) = existing_buckets.get(b_name) {
                if let Some(other) = owner_pid.as_deref() {
                    if other != project_id {
                        anyhow::bail!(
                            "bucket '{b_name}' belongs to another project ('{other}'), refusing to use it"
                        );
                    }
                }
                if *cur_pub != public_read {
                    let resp = client
                        .patch(format!("{server}/_api/v1/buckets/{b_name}"))
                        .header("Authorization", &auth_header)
                        .json(&serde_json::json!({ "public_read": public_read }))
                        .send()
                        .await?;
                    check_status(resp, &format!("update bucket '{b_name}' public_read")).await?;
                }
            } else {
                let resp = client
                    .post(format!("{server}/_api/v1/buckets"))
                    .header("Authorization", &auth_header)
                    .json(&serde_json::json!({
                        "name": b_name,
                        "project_id": project_id,
                        "public_read": public_read,
                        "group": manifest.app.group,
                    }))
                    .send()
                    .await?;
                check_status(resp, &format!("create bucket '{b_name}'")).await?;
            }
        }
    }

    let existing_dbs_val = check_status(
        client
            .get(format!("{server}/_api/v1/dbs"))
            .header("Authorization", &auth_header)
            .send()
            .await?,
        "list dbs",
    )
    .await?;
    let mut existing_dbs: HashMap<String, (String, Option<String>)> = HashMap::new();
    if let Some(arr) = existing_dbs_val.as_array() {
        for item in arr {
            if let (Some(name), Some(id)) = (item["name"].as_str(), item["id"].as_str()) {
                let owner = item["project_id"].as_str().map(str::to_string);
                existing_dbs.insert(name.to_string(), (id.to_string(), owner));
            }
        }
    }
    let mut db_ids: HashMap<String, String> = HashMap::new();
    for db_name in manifest
        .bindings
        .db
        .values()
        .map(String::as_str)
        .chain(manifest.migrations.iter().map(|m| m.db.as_str()))
    {
        if db_ids.contains_key(db_name) {
            continue;
        }
        if let Some((id, owner_pid)) = existing_dbs.get(db_name) {
            if let Some(other) = owner_pid.as_deref() {
                if other != project_id {
                    anyhow::bail!(
                        "database '{db_name}' belongs to another project ('{other}'), refusing to bind or run migrations"
                    );
                }
            }
            db_ids.insert(db_name.to_string(), id.clone());
        } else {
            let resp = client
                .post(format!("{server}/_api/v1/dbs"))
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({
                    "name": db_name,
                    "project_id": project_id,
                    "group": manifest.app.group,
                }))
                .send()
                .await?;
            let created = check_status(resp, &format!("create db '{db_name}'")).await?;
            if let Some(id) = created["id"].as_str() {
                db_ids.insert(db_name.to_string(), id.to_string());
                existing_dbs.insert(
                    db_name.to_string(),
                    (id.to_string(), Some(project_id.clone())),
                );
            }
        }
    }

    let existing_search_val = check_status(
        client
            .get(format!("{server}/_api/v1/search"))
            .header("Authorization", &auth_header)
            .send()
            .await?,
        "list search indexes",
    )
    .await?;
    let existing_search_names: std::collections::HashSet<String> = existing_search_val
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v["name"].as_str().map(str::to_string))
        .collect();
    for s_name in manifest.bindings.search.values() {
        if !existing_search_names.contains(s_name) {
            let resp = client
                .post(format!("{server}/_api/v1/search"))
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({
                    "name": s_name,
                    "project_id": project_id,
                    "group": manifest.app.group,
                }))
                .send()
                .await?;
            check_status(resp, &format!("create search index '{s_name}'")).await?;
        }
    }

    // 3. Apply sorted SQL migrations idempotently
    for mig in &manifest.migrations {
        let db_id = db_ids
            .get(&mig.db)
            .ok_or_else(|| anyhow!("migration db '{}' could not be resolved", mig.db))?;
        let mig_dir = base_dir.join(&mig.dir);
        if !mig_dir.exists() {
            anyhow::bail!("migration directory '{}' does not exist", mig_dir.display());
        }

        let resp = client
            .post(format!("{server}/_api/v1/dbs/{db_id}/execute"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "statements": [{
                    "sql": "CREATE TABLE IF NOT EXISTS _ferropod_migrations (version TEXT PRIMARY KEY, applied_at INTEGER NOT NULL)",
                    "params": []
                }]
            }))
            .send()
            .await?;
        check_status(resp, "init _ferropod_migrations").await?;

        let applied_qr = check_status(
            client
                .post(format!("{server}/_api/v1/dbs/{db_id}/query"))
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({
                    "sql": "SELECT version FROM _ferropod_migrations ORDER BY version ASC",
                    "params": []
                }))
                .send()
                .await?,
            "query applied migrations",
        )
        .await?;
        let applied: std::collections::HashSet<String> = applied_qr["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| r.get(0).and_then(|v| v.as_str()).map(str::to_string))
            .collect();

        let mut sql_files: Vec<(String, PathBuf)> = Vec::new();
        for entry in std::fs::read_dir(&mig_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("sql") {
                if let Some(fname) = path.file_name().and_then(|s| s.to_str()) {
                    sql_files.push((fname.to_string(), path));
                }
            }
        }
        sql_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (version, path) in sql_files {
            if applied.contains(&version) {
                continue;
            }
            let sql_body = std::fs::read_to_string(&path)?;
            let stmts = split_sql_statements(&sql_body);
            if stmts.is_empty() {
                anyhow::bail!(
                    "migration file '{}' contains no executable SQL statements",
                    version
                );
            }
            let now = Utc::now().timestamp();
            let mut batch: Vec<serde_json::Value> = stmts
                .into_iter()
                .map(|s| serde_json::json!({ "sql": s, "params": [] }))
                .collect();
            batch.push(serde_json::json!({
                "sql": "INSERT INTO _ferropod_migrations (version, applied_at) VALUES (?1, ?2)",
                "params": [version, now]
            }));
            let resp = client
                .post(format!("{server}/_api/v1/dbs/{db_id}/execute"))
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({ "statements": batch }))
                .send()
                .await?;
            check_status(resp, &format!("apply migration '{version}'")).await?;
            summary.migrations_applied += 1;
        }
    }

    // 4. Incremental static assets upload isolated per content-addressed release prefix
    let mut assets_bucket_out = None;
    let mut assets_prefix_out = None;
    let mut spa_fallback_out = false;
    if let Some(assets) = &manifest.assets {
        assets_bucket_out = Some(assets.bucket.clone());
        assets_prefix_out = assets.prefix.clone();
        spa_fallback_out = assets.spa_fallback;
        if let Some(dir_rel) = &assets.dir {
            let local_dir = base_dir.join(dir_rel);
            let mut local_files = Vec::new();
            collect_files_recursive(&local_dir, &local_dir, &mut local_files)?;
            local_files.sort_by(|a, b| a.0.cmp(&b.0));

            let mut prepared: Vec<(String, PathBuf, Vec<u8>, String)> =
                Vec::with_capacity(local_files.len());
            let mut file_digests: Vec<(String, String)> = Vec::with_capacity(local_files.len());
            for (rel_key, full_path) in local_files {
                let data = std::fs::read(&full_path)?;
                let digest = sha256_hex(&data);
                file_digests.push((rel_key.clone(), digest.clone()));
                prepared.push((rel_key, full_path, data, digest));
            }

            let release_prefix = compute_release_prefix(assets.prefix.as_deref(), &file_digests);
            assets_prefix_out = Some(release_prefix.clone());

            let existing_resp = client
                .get(format!(
                    "{server}/_blob/{}?prefix={}/&limit=1000",
                    assets.bucket,
                    encode_blob_key(&release_prefix)
                ))
                .header("Authorization", &auth_header)
                .send()
                .await?;
            let existing_list = check_status(existing_resp, "list existing blob objects").await?;
            let mut remote_hashes: HashMap<String, String> = HashMap::new();
            if let Some(arr) = existing_list.as_array() {
                for obj in arr {
                    if let (Some(k), Some(sha)) = (obj["key"].as_str(), obj["sha256"].as_str()) {
                        remote_hashes.insert(k.to_string(), sha.to_string());
                    }
                }
            }

            for (rel_key, full_path, data, digest) in prepared {
                let remote_key = format!("{release_prefix}/{rel_key}");
                if remote_hashes.get(&remote_key).is_some_and(|s| s == &digest) {
                    summary.assets_skipped += 1;
                    continue;
                }
                let mime = mime_guess::from_path(&full_path)
                    .first_or_octet_stream()
                    .essence_str()
                    .to_string();
                let encoded_key = encode_blob_key(&remote_key);
                let resp = client
                    .put(format!("{server}/_blob/{}/{}", assets.bucket, encoded_key))
                    .header("Authorization", &auth_header)
                    .header("Content-Type", mime)
                    .body(data)
                    .send()
                    .await?;
                check_status(resp, &format!("upload asset '{remote_key}'")).await?;
                summary.assets_uploaded += 1;
            }
        }
    }

    // 5. Upload worker artifact with immutable content-addressed filename
    let mut artifact_path_out = None;
    if let Some(worker) = &manifest.worker {
        let local_artifact = base_dir.join(&worker.artifact);
        let bytes = std::fs::read(&local_artifact).with_context(|| {
            format!(
                "failed to read worker artifact {}",
                local_artifact.display()
            )
        })?;
        let versioned_name = versioned_artifact_name(&local_artifact, &bytes);
        let resp = client
            .put(format!(
                "{server}/_api/v1/projects/{project_id}/artifacts/{versioned_name}"
            ))
            .header("Authorization", &auth_header)
            .header("Content-Type", "application/octet-stream")
            .body(bytes)
            .send()
            .await?;
        let uploaded = check_status(resp, "upload worker artifact").await?;
        artifact_path_out = uploaded["artifact_path"].as_str().map(str::to_string);
    }

    // 6. Create deployment
    let cron_names: Vec<String> = manifest.crons.iter().map(|c| c.name.clone()).collect();
    let dep_resp = client
        .post(format!(
            "{server}/_api/v1/projects/{project_id}/deployments"
        ))
        .header("Authorization", &auth_header)
        .json(&serde_json::json!({
            "artifact_path": artifact_path_out,
            "assets_bucket": assets_bucket_out,
            "assets_prefix": assets_prefix_out,
            "spa_fallback": spa_fallback_out,
            "bindings": manifest.bindings,
            "crons": cron_names,
            "target_env": opts.target_env,
        }))
        .send()
        .await?;
    let dep = check_status(dep_resp, "create deployment").await?;
    summary.deployment_id = dep["id"].as_str().map(str::to_string);

    // 7. Upsert routes
    for r in &manifest.routes {
        let resp = client
            .post(format!("{server}/_api/v1/routes"))
            .header("Authorization", &auth_header)
            .json(&serde_json::json!({
                "host": r.host,
                "project": project_id,
                "target_env": r.env,
            }))
            .send()
            .await?;
        check_status(resp, &format!("upsert route '{}'", r.host)).await?;
        summary.routes_bound += 1;
    }

    // 8. Sync crons
    if !manifest.crons.is_empty() {
        let existing_crons = check_status(
            client
                .get(format!("{server}/_api/v1/crons"))
                .header("Authorization", &auth_header)
                .send()
                .await?,
            "list crons",
        )
        .await?;
        let mut by_name: HashMap<String, String> = HashMap::new();
        if let Some(arr) = existing_crons.as_array() {
            for c in arr {
                if c["project_id"].as_str() == Some(project_id.as_str()) {
                    if let (Some(n), Some(id)) = (c["name"].as_str(), c["id"].as_str()) {
                        by_name.insert(n.to_string(), id.to_string());
                    }
                }
            }
        }
        for c in &manifest.crons {
            if let Some(cid) = by_name.get(&c.name) {
                let resp = client
                    .patch(format!("{server}/_api/v1/crons/{cid}"))
                    .header("Authorization", &auth_header)
                    .json(&serde_json::json!({
                        "schedule": c.schedule,
                        "target": c.target,
                        "enabled": c.enabled,
                    }))
                    .send()
                    .await?;
                check_status(resp, &format!("update cron '{}'", c.name)).await?;
            } else {
                let resp = client
                    .post(format!("{server}/_api/v1/crons"))
                    .header("Authorization", &auth_header)
                    .json(&serde_json::json!({
                        "name": c.name,
                        "project_id": project_id,
                        "schedule": c.schedule,
                        "target": c.target,
                        "enabled": c.enabled,
                    }))
                    .send()
                    .await?;
                check_status(resp, &format!("create cron '{}'", c.name)).await?;
            }
            summary.crons_synced += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_refuses_to_overwrite_existing_files_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let cargo = dir.path().join("Cargo.toml");
        std::fs::write(&cargo, "# keep me\n").unwrap();

        let err = init_project(dir.path(), "my-app", "rust").unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        // Verify original file untouched and no partial files created
        assert_eq!(std::fs::read_to_string(&cargo).unwrap(), "# keep me\n");
        assert!(!dir.path().join("ferropod.toml").exists());

        // With force = true, scaffolding succeeds
        let files = init_project_with_force(dir.path(), "my-app", "rust", true).unwrap();
        assert!(files.iter().any(|p| p.ends_with("ferropod.toml")));
        assert!(std::fs::read_to_string(&cargo)
            .unwrap()
            .contains("name = \"my-app\""));
    }

    #[test]
    fn split_sql_statements_handles_multi_comments_quotes_and_triggers() {
        let script = r#"
            -- leading comment; not a statement
            CREATE TABLE users (id INTEGER PRIMARY KEY, bio TEXT DEFAULT 'a;b''c');
            /* block ; comment */
            CREATE INDEX idx_users_id ON users(id);
            CREATE TRIGGER trg_users AFTER INSERT ON users
            BEGIN
                INSERT INTO audit(msg) VALUES ('inserted;ok');
            END;
            -- trailing comment only
        "#;
        let stmts = split_sql_statements(script);
        assert_eq!(stmts.len(), 3);
        assert!(stmts[0].contains("CREATE TABLE users"));
        assert!(stmts[0].contains("'a;b''c'"));
        assert!(stmts[1].contains("CREATE INDEX idx_users_id"));
        assert!(stmts[2].contains("CREATE TRIGGER trg_users"));
        assert!(stmts[2].contains("END"));
    }

    #[test]
    fn versioned_artifact_and_release_prefix_are_content_addressed() {
        let a1 = versioned_artifact_name(Path::new("target/app.wasm"), b"v1-bytes");
        let a2 = versioned_artifact_name(Path::new("target/app.wasm"), b"v2-bytes");
        assert_ne!(a1, a2);
        assert!(a1.starts_with("app-") && a1.ends_with(".wasm"));
        assert!(a2.starts_with("app-") && a2.ends_with(".wasm"));

        let d1 = vec![("index.html".to_string(), sha256_hex(b"prod"))];
        let d2 = vec![("index.html".to_string(), sha256_hex(b"preview"))];
        let p1 = compute_release_prefix(Some("web"), &d1);
        let p1_again = compute_release_prefix(Some("web"), &d1);
        let p2 = compute_release_prefix(Some("web"), &d2);
        assert_eq!(p1, p1_again);
        assert_ne!(p1, p2);
        assert!(p1.starts_with("web/_releases/rel_"));
    }
}
