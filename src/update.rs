use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::Utc;
use flate2::read::GzDecoder;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::watch;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_REPO: &str = "ferropod/ferropod";
pub const DEFAULT_API_BASE: &str = "https://api.github.com";

static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);
static RESTART_TX: OnceLock<watch::Sender<bool>> = OnceLock::new();

/// Register the server shutdown/restart channel so API-triggered updates can
/// cleanly drain connections, flush Turso checkpoints, and re-exec the new binary.
pub fn init_restart_channel() -> watch::Receiver<bool> {
    let tx = RESTART_TX.get_or_init(|| {
        let (tx, _rx) = watch::channel(false);
        tx
    });
    tx.subscribe()
}

/// Request a graceful server shutdown followed by process restart.
pub fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::SeqCst);
    if let Some(tx) = RESTART_TX.get() {
        let _ = tx.send(true);
    }
}

/// Returns true if the server exited because a self-update requested a restart.
pub fn is_restart_requested() -> bool {
    RESTART_REQUESTED.load(Ordering::SeqCst)
}

/// Replace the current process image with the newly installed binary (Unix).
#[cfg(unix)]
pub fn exec_self_restart() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().context("resolve current executable path")?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let err = std::process::Command::new(&exe).args(&args).exec();
    Err(anyhow!("failed to exec {}: {err}", exe.display()))
}

#[cfg(not(unix))]
pub fn exec_self_restart() -> Result<()> {
    let exe = std::env::current_exe().context("resolve current executable path")?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    std::process::Command::new(&exe)
        .args(&args)
        .spawn()
        .with_context(|| format!("spawn restarted process {}", exe.display()))?;
    Ok(())
}

/// RAII guard preventing concurrent self-update executions.
pub struct UpdateLock;

impl UpdateLock {
    pub fn try_acquire() -> Option<Self> {
        if UPDATE_IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            Some(Self)
        } else {
            None
        }
    }
}

impl Drop for UpdateLock {
    fn drop(&mut self) {
        UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

/// Static runtime information about the current ferropod installation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemUpdateInfo {
    pub current_version: String,
    pub target: String,
    pub repo: String,
    pub install_mode: String,
    pub binary_path: String,
    pub can_self_update: bool,
    pub update_in_progress: bool,
}

/// Summary of a GitHub Release relevant to the current platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseSummary {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub body: String,
    pub html_url: String,
    pub published_at: Option<String>,
    pub prerelease: bool,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub asset_size: Option<u64>,
    pub checksum_url: Option<String>,
}

/// Result returned when checking for updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCheckResult {
    pub current_version: String,
    pub target: String,
    pub repo: String,
    pub install_mode: String,
    pub binary_path: String,
    pub can_self_update: bool,
    pub update_available: bool,
    pub latest: Option<ReleaseSummary>,
    pub checked_at: i64,
}

/// Result returned after applying a self-update.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateApplyResult {
    pub previous_version: String,
    pub new_version: String,
    pub target_tag: String,
    pub binary_path: String,
    pub backup_path: Option<String>,
    pub checksum_verified: bool,
    pub restarting: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

pub fn update_repo() -> String {
    std::env::var("FERROPOD_UPDATE_REPO")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

pub fn api_base() -> String {
    std::env::var("FERROPOD_UPDATE_API_BASE")
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// Map compile-time OS/architecture to the GitHub Release target triple.
pub fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        _ => "unsupported",
    }
}

/// Detect whether ferropod is running inside Docker, systemd, or as a standalone binary.
pub fn detect_install_mode() -> &'static str {
    if Path::new("/.dockerenv").exists()
        || std::env::var("FERROPOD_DOCKER")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    {
        return "docker";
    }
    if std::env::var_os("INVOCATION_ID").is_some() {
        return "systemd";
    }
    "binary"
}

fn is_path_writable(exe: &Path) -> bool {
    let Some(parent) = exe.parent() else {
        return false;
    };
    let probe = parent.join(format!(".ferropod-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Return static runtime info about the running instance and update capability.
pub fn system_info() -> SystemUpdateInfo {
    let install_mode = detect_install_mode();
    let exe_path = std::env::current_exe().ok();
    let binary_path = exe_path
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let target = current_target();
    let can_self_update = install_mode != "docker"
        && target != "unsupported"
        && exe_path.as_deref().map(is_path_writable).unwrap_or(false);

    SystemUpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        target: target.to_string(),
        repo: update_repo(),
        install_mode: install_mode.to_string(),
        binary_path,
        can_self_update,
        update_in_progress: UPDATE_IN_PROGRESS.load(Ordering::SeqCst),
    }
}

fn build_http_client() -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(&format!("ferropod/{CURRENT_VERSION}"))
            .unwrap_or_else(|_| HeaderValue::from_static("ferropod")),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    if let Ok(token) =
        std::env::var("FERROPOD_GITHUB_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN"))
    {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {trimmed}")) {
                headers.insert(AUTHORIZATION, val);
            }
        }
    }

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(120))
        .build()
        .context("build HTTP client")
}

fn to_release_summary(rel: GhRelease, target: &str) -> ReleaseSummary {
    let expected_asset = format!("ferropod-{}-{target}.tar.gz", rel.tag_name);
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == expected_asset)
        .or_else(|| {
            rel.assets
                .iter()
                .find(|a| a.name.contains(target) && a.name.ends_with(".tar.gz"))
        });
    let checksum = rel
        .assets
        .iter()
        .find(|a| a.name == "SHA256SUMS.txt" || a.name.ends_with(".sha256"));

    let version = rel.tag_name.trim_start_matches('v').to_string();
    let name = rel
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| rel.tag_name.clone());

    ReleaseSummary {
        tag: rel.tag_name,
        version,
        name,
        body: rel.body.unwrap_or_default(),
        html_url: rel.html_url,
        published_at: rel.published_at,
        prerelease: rel.prerelease,
        asset_name: asset.map(|a| a.name.clone()),
        asset_url: asset.map(|a| a.browser_download_url.clone()),
        asset_size: asset.map(|a| a.size),
        checksum_url: checksum.map(|c| c.browser_download_url.clone()),
    }
}

/// Compare two semver strings (optionally prefixed with `v`).
/// Returns `true` if `candidate` is strictly newer than `current`.
pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    let parse = |s: &str| -> Option<((u64, u64, u64), Option<String>)> {
        let trimmed = s.trim().trim_start_matches('v');
        let (core, pre) = match trimmed.split_once('-') {
            Some((c, p)) => (c, Some(p.to_string())),
            None => (trimmed, None),
        };
        let mut parts = core.split('.');
        let major = parts.next()?.parse::<u64>().ok()?;
        let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
        Some(((major, minor, patch), pre))
    };

    let (Some((cur_core, cur_pre)), Some((cand_core, cand_pre))) =
        (parse(current), parse(candidate))
    else {
        return current.trim().trim_start_matches('v') != candidate.trim().trim_start_matches('v');
    };

    if cand_core != cur_core {
        return cand_core > cur_core;
    }
    match (cur_pre, cand_pre) {
        (Some(_), None) => true,  // stable > prerelease
        (None, Some(_)) => false, // prerelease < stable
        (Some(a), Some(b)) => b > a,
        (None, None) => false,
    }
}

/// Query GitHub Releases for the latest release and compare with the running version.
pub async fn check_for_update() -> Result<UpdateCheckResult> {
    let info = system_info();
    let client = build_http_client()?;
    let url = format!("{}/repos/{}/releases/latest", api_base(), info.repo);

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("fetch latest release from {url}"))?;

    let latest = if resp.status() == reqwest::StatusCode::NOT_FOUND {
        None
    } else if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("GitHub API returned {status}: {body}");
    } else {
        let gh: GhRelease = resp.json().await.context("parse GitHub release JSON")?;
        Some(to_release_summary(gh, &info.target))
    };

    let update_available = latest
        .as_ref()
        .map(|rel| is_newer_version(&info.current_version, &rel.version))
        .unwrap_or(false);

    Ok(UpdateCheckResult {
        current_version: info.current_version,
        target: info.target,
        repo: info.repo,
        install_mode: info.install_mode,
        binary_path: info.binary_path,
        can_self_update: info.can_self_update,
        update_available,
        latest,
        checked_at: Utc::now().timestamp(),
    })
}

/// List recent published GitHub Releases (excluding drafts).
pub async fn list_releases(limit: usize) -> Result<Vec<ReleaseSummary>> {
    let info = system_info();
    let client = build_http_client()?;
    let per_page = limit.clamp(1, 30);
    let url = format!(
        "{}/repos/{}/releases?per_page={per_page}",
        api_base(),
        info.repo
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("fetch releases list from {url}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("GitHub API returned {status}: {body}");
    }

    let items: Vec<GhRelease> = resp.json().await.context("parse GitHub releases JSON")?;
    Ok(items
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| to_release_summary(r, &info.target))
        .collect())
}

async fn fetch_release_by_tag(
    client: &reqwest::Client,
    repo: &str,
    target: &str,
    tag: Option<&str>,
) -> Result<ReleaseSummary> {
    let url = match tag.map(str::trim).filter(|s| !s.is_empty()) {
        Some(t) => {
            let normalized = if t.starts_with('v') {
                t.to_string()
            } else {
                format!("v{t}")
            };
            format!("{}/repos/{repo}/releases/tags/{normalized}", api_base())
        }
        None => format!("{}/repos/{repo}/releases/latest", api_base()),
    };

    let resp = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("fetch release metadata from {url}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("release {} not found in {repo}", tag.unwrap_or("latest"));
    }
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("GitHub API returned {status}: {body}");
    }

    let gh: GhRelease = resp.json().await.context("parse GitHub release JSON")?;
    Ok(to_release_summary(gh, target))
}

/// Parse a `SHA256SUMS.txt` file and return the expected hex digest for `asset_name`.
pub fn parse_checksum_for_asset(sums_txt: &str, asset_name: &str) -> Option<String> {
    for line in sums_txt.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        if file == asset_name || file.ends_with(&format!("/{asset_name}")) {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

/// Extract the `ferropod` binary bytes from a `.tar.gz` archive.
pub fn extract_binary_from_tar_gz(tar_gz_bytes: &[u8]) -> Result<Vec<u8>> {
    let gz = GzDecoder::new(tar_gz_bytes);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().context("iterate tar entries")? {
        let mut entry = entry.context("read tar entry")?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().context("read tar entry path")?;
        let file_name = path.file_name().and_then(|n| n.to_str());
        if file_name == Some("ferropod") || file_name == Some("ferropod.exe") {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .context("read ferropod binary from archive")?;
            return Ok(buf);
        }
    }
    bail!("archive does not contain a 'ferropod' binary")
}

/// Download, verify, and atomically install a ferropod release binary.
pub async fn apply_update(
    target_tag: Option<&str>,
    force: bool,
    will_restart: bool,
) -> Result<UpdateApplyResult> {
    let info = system_info();
    if info.install_mode == "docker" {
        bail!(
            "running inside Docker container; update with: docker compose pull && docker compose up -d"
        );
    }
    if info.target == "unsupported" {
        bail!(
            "self-update is not supported on {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }

    let exe_path = std::env::current_exe().context("resolve current executable path")?;
    let parent_dir = exe_path
        .parent()
        .ok_or_else(|| anyhow!("executable has no parent directory"))?;
    if !is_path_writable(&exe_path) {
        bail!(
            "cannot write to {}; run `ferropod update` with appropriate permissions (e.g. sudo)",
            parent_dir.display()
        );
    }

    let client = build_http_client()?;
    let rel = fetch_release_by_tag(&client, &info.repo, &info.target, target_tag).await?;

    if !force && target_tag.is_none() && !is_newer_version(&info.current_version, &rel.version) {
        return Ok(UpdateApplyResult {
            previous_version: info.current_version.clone(),
            new_version: info.current_version,
            target_tag: rel.tag,
            binary_path: exe_path.display().to_string(),
            backup_path: None,
            checksum_verified: false,
            restarting: false,
            message: "already on the latest version".to_string(),
        });
    }

    let asset_url = rel.asset_url.as_deref().ok_or_else(|| {
        anyhow!(
            "release {} does not contain a binary asset for target {}",
            rel.tag,
            info.target
        )
    })?;
    let asset_name = rel
        .asset_name
        .clone()
        .unwrap_or_else(|| format!("ferropod-{}-{}.tar.gz", rel.tag, info.target));

    let archive_bytes = client
        .get(asset_url)
        .send()
        .await
        .with_context(|| format!("download {asset_url}"))?
        .error_for_status()
        .with_context(|| format!("download failed for {asset_url}"))?
        .bytes()
        .await
        .context("read downloaded release archive")?;

    let mut checksum_verified = false;
    if let Some(cs_url) = rel.checksum_url.as_deref() {
        let cs_resp = client
            .get(cs_url)
            .send()
            .await
            .with_context(|| format!("download checksums from {cs_url}"))?;
        if cs_resp.status().is_success() {
            let sums_txt = cs_resp.text().await.unwrap_or_default();
            if let Some(expected) = parse_checksum_for_asset(&sums_txt, &asset_name) {
                let mut hasher = Sha256::new();
                hasher.update(&archive_bytes);
                let actual = hex::encode(hasher.finalize());
                if actual != expected {
                    bail!("SHA-256 mismatch for {asset_name}: expected {expected}, got {actual}");
                }
                checksum_verified = true;
            }
        }
    }

    let bin_bytes = tokio::task::spawn_blocking(move || extract_binary_from_tar_gz(&archive_bytes))
        .await
        .context("extract task join failed")??;

    let tmp_bin: PathBuf = parent_dir.join(format!(".ferropod-update-{}.tmp", std::process::id()));
    tokio::fs::write(&tmp_bin, &bin_bytes)
        .await
        .with_context(|| format!("write staged binary {}", tmp_bin.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&tmp_bin, std::fs::Permissions::from_mode(0o755))
            .await
            .with_context(|| format!("chmod +x {}", tmp_bin.display()))?;
    }

    // Smoke-test the extracted binary before replacing the live executable.
    let verify_out = tokio::process::Command::new(&tmp_bin)
        .arg("--version")
        .output()
        .await;
    match verify_out {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let _ = tokio::fs::remove_file(&tmp_bin).await;
            let stderr = String::from_utf8_lossy(&out.stderr);
            bail!("staged binary failed --version check: {stderr}");
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp_bin).await;
            bail!("failed to execute staged binary for verification: {e}");
        }
    }

    // Best-effort backup of previous binary.
    let backup_path = parent_dir.join("ferropod.bak");
    let backup_created = tokio::fs::copy(&exe_path, &backup_path).await.is_ok();

    // Atomic replacement on the same filesystem.
    if let Err(e) = tokio::fs::rename(&tmp_bin, &exe_path).await {
        let _ = tokio::fs::remove_file(&tmp_bin).await;
        return Err(anyhow!("failed to replace {}: {e}", exe_path.display()));
    }

    Ok(UpdateApplyResult {
        previous_version: info.current_version,
        new_version: rel.version,
        target_tag: rel.tag.clone(),
        binary_path: exe_path.display().to_string(),
        backup_path: backup_created.then(|| backup_path.display().to_string()),
        checksum_verified,
        restarting: will_restart,
        message: if will_restart {
            format!("updated to {}; restarting server...", rel.tag)
        } else {
            format!("updated to {}", rel.tag)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};

    #[test]
    fn semver_comparison_works() {
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.1.0", "v0.1.1"));
        assert!(is_newer_version("0.1.0", "1.0.0"));
        assert!(is_newer_version("0.2.0-rc.1", "0.2.0"));
        assert!(is_newer_version("0.2.0-rc.1", "0.2.0-rc.2"));
        assert!(!is_newer_version("0.2.0", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "v0.2.0"));
        assert!(!is_newer_version("0.2.0", "0.2.0-rc.1"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
    }

    #[test]
    fn parses_sha256sums_correctly() {
        let sums = "\
aaaa1111  ferropod-v0.2.0-x86_64-unknown-linux-gnu.tar.gz
bbbb2222 *ferropod-v0.2.0-aarch64-unknown-linux-gnu.tar.gz
";
        assert_eq!(
            parse_checksum_for_asset(sums, "ferropod-v0.2.0-x86_64-unknown-linux-gnu.tar.gz")
                .as_deref(),
            Some("aaaa1111")
        );
        assert_eq!(
            parse_checksum_for_asset(sums, "ferropod-v0.2.0-aarch64-unknown-linux-gnu.tar.gz")
                .as_deref(),
            Some("bbbb2222")
        );
        assert_eq!(parse_checksum_for_asset(sums, "missing.tar.gz"), None);
    }

    #[test]
    fn extracts_ferropod_binary_from_tar_gz() {
        let payload = b"#!/bin/sh\necho ferropod 0.2.0\n";
        let mut tar_gz = Vec::new();
        {
            let enc = GzEncoder::new(&mut tar_gz, Compression::default());
            let mut builder = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    "ferropod-v0.2.0-x86_64-unknown-linux-gnu/ferropod",
                    &payload[..],
                )
                .unwrap();
            builder.finish().unwrap();
        }
        let extracted = extract_binary_from_tar_gz(&tar_gz).unwrap();
        assert_eq!(extracted, payload);
    }
}
