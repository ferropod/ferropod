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

/// Scaffold a starter FerroPod project (`static`, `rust`, or `ts`).
pub fn init_project(dir: &Path, name: &str, template: &str) -> Result<Vec<PathBuf>> {
    let clean_name = name.trim();
    if clean_name.is_empty() {
        anyhow::bail!("project name cannot be empty");
    }
    std::fs::create_dir_all(dir)?;
    let mut created = Vec::new();

    let bucket_name = format!("{clean_name}-assets");
    let db_name = format!("{clean_name}-db");
    let kv_name = format!("{clean_name}-kv");

    std::fs::create_dir_all(dir.join("public"))?;
    let index_path = dir.join("public/index.html");
    std::fs::write(
        &index_path,
        format!(
            "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>{clean_name}</title></head>\n<body><h1>Hello from {clean_name}</h1><p>Deployed declaratively with ferropod.toml.</p></body>\n</html>\n"
        ),
    )?;
    created.push(index_path);

    std::fs::create_dir_all(dir.join("migrations"))?;
    let mig_path = dir.join("migrations/0001_init.sql");
    std::fs::write(
        &mig_path,
        "CREATE TABLE IF NOT EXISTS notes (\n  id INTEGER PRIMARY KEY AUTOINCREMENT,\n  title TEXT NOT NULL,\n  created_at INTEGER NOT NULL\n);\n",
    )?;
    created.push(mig_path);

    let worker_block = match template {
        "static" => String::new(),
        "rust" => {
            std::fs::create_dir_all(dir.join("src"))?;
            let cargo_path = dir.join("Cargo.toml");
            std::fs::write(
                &cargo_path,
                format!(
                    "[package]\nname = \"{clean_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n"
                ),
            )?;
            created.push(cargo_path);
            let lib_path = dir.join("src/lib.rs");
            std::fs::write(
                &lib_path,
                "// Build with: cargo build --target wasm32-wasip2 --release\n// Export wasi:http/incoming-handler using ferropod:platform@0.2.0 bindings.\n",
            )?;
            created.push(lib_path);
            format!(
                "\n[worker]\nartifact = \"target/wasm32-wasip2/release/{}.wasm\"\n",
                clean_name.replace('-', "_")
            )
        }
        "ts" => {
            let ts_path = dir.join("worker.ts");
            std::fs::write(
                &ts_path,
                "// Bundle to wasm32-wasip2 component using jco / ComponentizeJS.\nexport const handler = {\n  fetch(req: Request): Response {\n    return new Response(JSON.stringify({ ok: true, url: req.url }), {\n      headers: { \"content-type\": \"application/json\" },\n    });\n  },\n};\n",
            )?;
            created.push(ts_path);
            "\n[worker]\nartifact = \"dist/worker.wasm\"\n".to_string()
        }
        other => anyhow::bail!("unknown template '{other}', expected 'static', 'rust', or 'ts'"),
    };

    let manifest_path = dir.join("ferropod.toml");
    let toml_body = format!(
        "[app]\nname = \"{clean_name}\"\ndescription = \"FerroPod {template} application\"\nhealth_check_path = \"/_healthz\"\n{worker_block}\n[assets]\nbucket = \"{bucket_name}\"\ndir = \"public\"\nspa_fallback = true\npublic_read = true\n\n[bindings.kv]\nCACHE = \"{kv_name}\"\n\n[bindings.db]\nDB = \"{db_name}\"\n\n[[migrations]]\ndb = \"{db_name}\"\ndir = \"migrations\"\n\n[[routes]]\nhost = \"{clean_name}.local\"\nenv = \"prod\"\n"
    );
    std::fs::write(&manifest_path, toml_body)?;
    created.push(manifest_path);

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

    for (b_name, public_read) in manifest
        .bindings
        .bucket
        .values()
        .map(|n| (n.as_str(), false))
        .chain(
            manifest
                .assets
                .as_ref()
                .map(|a| (a.bucket.as_str(), a.public_read)),
        )
    {
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
        if resp.status() != StatusCode::CONFLICT {
            check_status(resp, &format!("create bucket '{b_name}'")).await?;
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
    let mut db_ids: HashMap<String, String> = HashMap::new();
    if let Some(arr) = existing_dbs_val.as_array() {
        for item in arr {
            if let (Some(name), Some(id)) = (item["name"].as_str(), item["id"].as_str()) {
                db_ids.insert(name.to_string(), id.to_string());
            }
        }
    }
    for db_name in manifest
        .bindings
        .db
        .values()
        .map(String::as_str)
        .chain(manifest.migrations.iter().map(|m| m.db.as_str()))
    {
        if !db_ids.contains_key(db_name) {
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
            let now = Utc::now().timestamp();
            let resp = client
                .post(format!("{server}/_api/v1/dbs/{db_id}/execute"))
                .header("Authorization", &auth_header)
                .json(&serde_json::json!({
                    "statements": [
                        { "sql": sql_body, "params": [] },
                        { "sql": "INSERT INTO _ferropod_migrations (version, applied_at) VALUES (?1, ?2)", "params": [version, now] }
                    ]
                }))
                .send()
                .await?;
            check_status(resp, &format!("apply migration '{version}'")).await?;
            summary.migrations_applied += 1;
        }
    }

    // 4. Incremental static assets upload (SHA-256 diffing)
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

            let existing_resp = client
                .get(format!("{server}/_blob/{}?limit=1000", assets.bucket))
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

            let prefix_clean = assets
                .prefix
                .as_deref()
                .map(|p| p.trim_matches('/'))
                .filter(|p| !p.is_empty());

            for (rel_key, full_path) in local_files {
                let remote_key = match prefix_clean {
                    Some(pfx) => format!("{pfx}/{rel_key}"),
                    None => rel_key,
                };
                let data = std::fs::read(&full_path)?;
                let digest = sha256_hex(&data);
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

    // 5. Upload worker artifact if declared
    let mut artifact_path_out = None;
    if let Some(worker) = &manifest.worker {
        let local_artifact = base_dir.join(&worker.artifact);
        let bytes = std::fs::read(&local_artifact).with_context(|| {
            format!(
                "failed to read worker artifact {}",
                local_artifact.display()
            )
        })?;
        let fname = local_artifact
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("worker.wasm");
        let resp = client
            .put(format!(
                "{server}/_api/v1/projects/{project_id}/artifacts/{fname}"
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
