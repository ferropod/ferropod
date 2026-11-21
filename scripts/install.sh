#!/usr/bin/env bash
set -euo pipefail

# ferropod one-click installer / updater / uninstaller
# Usage:
#   Install:   curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash
#   Update:    curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash -s -- --update
#   Uninstall: curl -fsSL https://raw.githubusercontent.com/ferropod/ferropod/main/scripts/install.sh | sudo bash -s -- --uninstall

REPO="${FERROPOD_REPO:-ferropod/ferropod}"
BIN_DIR="${FERROPOD_BIN_DIR:-/usr/local/bin}"
DATA_DIR="${FERROPOD_DATA_DIR:-/var/lib/ferropod}"
ENV_DIR="/etc/ferropod"
ENV_FILE="${ENV_DIR}/ferropod.env"
SERVICE_FILE="/etc/systemd/system/ferropod.service"

PROFILE="${FERROPOD_PROFILE:-low-mem}"
BIND_ADDR="${FERROPOD_BIND:-0.0.0.0:8080}"
HTTPS_BIND="${FERROPOD_HTTPS_BIND:-}"
ACME_DOMAINS="${FERROPOD_ACME_DOMAINS:-}"
ACME_EMAIL="${FERROPOD_ACME_EMAIL:-}"
REDIRECT_HTTPS="${FERROPOD_HTTP_REDIRECT_HTTPS:-false}"

VERSION=""
MODE="install"
FORCE="false"
NO_SYSTEMD="false"
PURGE="false"

info()  { printf '\033[1;34m[ferropod]\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m[ferropod]\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m[ferropod]\033[0m %s\n' "$*" >&2; }
die()   { printf '\033[1;31m[ferropod]\033[0m ERROR: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
ferropod One-Click Installer & Updater

Usage:
  install.sh [OPTIONS]

Modes:
  (default)                 Install ferropod (and configure systemd on Linux when run as root)
  --update                  Check and upgrade existing ferropod installation
  --uninstall               Remove ferropod binary and systemd service (keeps data dir)

Options:
  --version <TAG>           Install a specific release tag (e.g. v0.1.0); defaults to latest
  --force                   Reinstall even if already on the target version
  --purge                   With --uninstall, also remove data dir and /etc/ferropod
  --repo <OWNER/REPO>       GitHub repository (default: ferropod/ferropod)
  --bin-dir <DIR>           Binary install directory (default: /usr/local/bin)
  --data-dir <DIR>          Data directory (default: /var/lib/ferropod)
  --profile <PROFILE>       Runtime profile: low-mem | standard (default: low-mem)
  --bind <ADDR>             HTTP listen address (default: 0.0.0.0:8080)
  --https-bind <ADDR>       HTTPS listen address (e.g. 0.0.0.0:443)
  --domain <DOMAIN>         ACME Let's Encrypt domain (repeatable or comma-separated)
  --email <EMAIL>           ACME Let's Encrypt contact email
  --redirect-https          Redirect HTTP to HTTPS (308)
  --no-systemd              Install binary only; do not configure systemd service
  -h, --help                Show this help message
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --update)         MODE="update"; shift ;;
    --uninstall)      MODE="uninstall"; shift ;;
    --version)        VERSION="${2:-}"; shift 2 ;;
    --force)          FORCE="true"; shift ;;
    --purge)          PURGE="true"; shift ;;
    --repo)           REPO="${2:-}"; shift 2 ;;
    --bin-dir)        BIN_DIR="${2:-}"; shift 2 ;;
    --data-dir)       DATA_DIR="${2:-}"; shift 2 ;;
    --profile)        PROFILE="${2:-}"; shift 2 ;;
    --bind)           BIND_ADDR="${2:-}"; shift 2 ;;
    --https-bind)     HTTPS_BIND="${2:-}"; shift 2 ;;
    --domain)
      if [[ -n "$ACME_DOMAINS" ]]; then
        ACME_DOMAINS="${ACME_DOMAINS},${2:-}"
      else
        ACME_DOMAINS="${2:-}"
      fi
      shift 2
      ;;
    --email)          ACME_EMAIL="${2:-}"; shift 2 ;;
    --redirect-https) REDIRECT_HTTPS="true"; shift ;;
    --no-systemd)     NO_SYSTEMD="true"; shift ;;
    -h|--help)        usage; exit 0 ;;
    *)                die "Unknown option: $1 (use --help for usage)" ;;
  esac
done

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"
}

detect_target() {
  local os arch
  os="$(uname -s | tr '[:upper:]' '[:lower:]')"
  arch="$(uname -m)"

  case "$os" in
    linux)
      case "$arch" in
        x86_64|amd64)   echo "x86_64-unknown-linux-gnu" ;;
        aarch64|arm64)  echo "aarch64-unknown-linux-gnu" ;;
        *)              die "Unsupported Linux architecture: $arch" ;;
      esac
      ;;
    darwin)
      case "$arch" in
        arm64|aarch64)  echo "aarch64-apple-darwin" ;;
        x86_64|amd64)   echo "x86_64-apple-darwin" ;;
        *)              die "Unsupported macOS architecture: $arch" ;;
      esac
      ;;
    *)
      die "Unsupported operating system: $os"
      ;;
  esac
}

github_curl() {
  local url="$1"
  if [[ -n "${GITHUB_TOKEN:-}" ]]; then
    curl -fsSL -H "Accept: application/vnd.github+json" -H "Authorization: Bearer ${GITHUB_TOKEN}" "$url"
  else
    curl -fsSL -H "Accept: application/vnd.github+json" "$url"
  fi
}

resolve_latest_tag() {
  local api_url="https://api.github.com/repos/${REPO}/releases/latest"
  local json tag
  json="$(github_curl "$api_url")" || die "Failed to query GitHub Releases (${api_url}). Has a release been published yet?"
  tag="$(printf '%s' "$json" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  [[ -n "$tag" ]] || die "Could not parse tag_name from GitHub Releases response"
  printf '%s' "$tag"
}

installed_version() {
  local bin="${BIN_DIR}/ferropod"
  if [[ -x "$bin" ]]; then
    "$bin" --version 2>/dev/null | awk '{print $2}' | head -n 1 || true
  fi
}

verify_sha256() {
  local file="$1"
  local expected="$2"
  local actual=""
  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print tolower($1)}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file" | awk '{print tolower($1)}')"
  else
    warn "sha256sum/shasum not found; skipping checksum verification"
    return 0
  fi
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  if [[ "$actual" != "$expected" ]]; then
    die "SHA-256 mismatch for $(basename "$file"): expected ${expected}, got ${actual}"
  fi
  ok "SHA-256 checksum verified."
}

generate_master_key() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  elif [[ -r /dev/urandom ]]; then
    od -An -tx1 -N32 /dev/urandom | tr -d ' \n'
  else
    date +%s%N | sha256sum | awk '{print $1}'
  fi
}

has_systemd() {
  [[ "$(uname -s)" == "Linux" ]] \
    && [[ "$NO_SYSTEMD" != "true" ]] \
    && [[ "$(id -u)" -eq 0 ]] \
    && command -v systemctl >/dev/null 2>&1 \
    && [[ -d /run/systemd/system ]]
}

do_uninstall() {
  info "Uninstalling ferropod..."
  if command -v systemctl >/dev/null 2>&1 && [[ -f "$SERVICE_FILE" ]]; then
    systemctl stop ferropod 2>/dev/null || true
    systemctl disable ferropod 2>/dev/null || true
    rm -f "$SERVICE_FILE"
    systemctl daemon-reload 2>/dev/null || true
    ok "Removed systemd service."
  fi

  rm -f "${BIN_DIR}/ferropod" "${BIN_DIR}/ferropod.bak"
  ok "Removed binary from ${BIN_DIR}/ferropod."

  if [[ "$PURGE" == "true" ]]; then
    rm -rf "$DATA_DIR" "$ENV_DIR"
    ok "Purged ${DATA_DIR} and ${ENV_DIR}."
  else
    info "Data preserved at ${DATA_DIR} and config at ${ENV_DIR} (pass --purge to remove)."
  fi
}

setup_systemd_service() {
  mkdir -p "$DATA_DIR" "$ENV_DIR"
  chmod 750 "$DATA_DIR"

  local fresh_env="false"
  if [[ ! -f "$ENV_FILE" ]]; then
    fresh_env="true"
    local master_key
    master_key="$(generate_master_key)"
    cat >"$ENV_FILE" <<EOF
FERROPOD_MASTER_KEY=${master_key}
FERROPOD_PROFILE=${PROFILE}
FERROPOD_BIND=${BIND_ADDR}
FERROPOD_DATA_DIR=${DATA_DIR}
EOF
    if [[ -n "$HTTPS_BIND" ]]; then
      echo "FERROPOD_HTTPS_BIND=${HTTPS_BIND}" >>"$ENV_FILE"
    fi
    if [[ -n "$ACME_DOMAINS" ]]; then
      echo "FERROPOD_ACME_DOMAINS=${ACME_DOMAINS}" >>"$ENV_FILE"
    fi
    if [[ -n "$ACME_EMAIL" ]]; then
      echo "FERROPOD_ACME_EMAIL=${ACME_EMAIL}" >>"$ENV_FILE"
    fi
    if [[ "$REDIRECT_HTTPS" == "true" ]]; then
      echo "FERROPOD_HTTP_REDIRECT_HTTPS=true" >>"$ENV_FILE"
    fi
    chmod 600 "$ENV_FILE"
    ok "Generated ${ENV_FILE} with random FERROPOD_MASTER_KEY."
  else
    info "Keeping existing ${ENV_FILE}."
  fi

  local cap_lines=""
  local bind_port="${BIND_ADDR##*:}"
  if [[ "$bind_port" =~ ^[0-9]+$ ]] && (( bind_port < 1024 )); then
    cap_lines=$'AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE'
  elif [[ -n "$HTTPS_BIND" || -n "$ACME_DOMAINS" ]]; then
    cap_lines=$'AmbientCapabilities=CAP_NET_BIND_SERVICE\nCapabilityBoundingSet=CAP_NET_BIND_SERVICE'
  fi

  cat >"$SERVICE_FILE" <<EOF
[Unit]
Description=ferropod AI-Native Self-Hosted PaaS
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=${ENV_FILE}
ExecStart=${BIN_DIR}/ferropod serve
Restart=always
RestartSec=3
LimitNOFILE=65535
${cap_lines}

[Install]
WantedBy=multi-user.target
EOF

  systemctl daemon-reload
  systemctl enable ferropod >/dev/null 2>&1 || true
  systemctl restart ferropod
  ok "systemd service ferropod started."

  # Mint initial admin token on fresh install
  if [[ "$fresh_env" == "true" ]]; then
    sleep 1
    local master_key_val
    master_key_val="$(grep '^FERROPOD_MASTER_KEY=' "$ENV_FILE" | cut -d'=' -f2-)"
    local token_out
    if token_out="$(FERROPOD_DATA_DIR="$DATA_DIR" FERROPOD_MASTER_KEY="$master_key_val" "${BIN_DIR}/ferropod" token create --name admin --scope admin 2>/dev/null)"; then
      echo ""
      ok "Initial Admin Token created (save this securely):"
      printf '%s\n' "$token_out"
    fi
  fi
}

check_health() {
  local bind_val="$BIND_ADDR"
  if [[ -f "$ENV_FILE" ]]; then
    local env_bind
    env_bind="$(grep '^FERROPOD_BIND=' "$ENV_FILE" | cut -d'=' -f2- || true)"
    [[ -n "$env_bind" ]] && bind_val="$env_bind"
  fi
  local port="${bind_val##*:}"
  local url="http://127.0.0.1:${port}/_healthz"
  local i
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if curl -fsS "$url" >/dev/null 2>&1; then
      ok "Health check passed: ${url}"
      ok "Admin UI: http://<your-server-ip>:${port}/_admin"
      return 0
    fi
    sleep 1
  done
  warn "Health check did not respond yet on ${url}; check 'journalctl -u ferropod -n 50'"
}

main() {
  if [[ "$MODE" == "uninstall" ]]; then
    do_uninstall
    exit 0
  fi

  need_cmd curl
  need_cmd tar

  local target tag clean_tag current_ver
  target="$(detect_target)"
  info "Detected platform: ${target}"

  if [[ -n "$VERSION" ]]; then
    tag="$VERSION"
    [[ "$tag" == v* ]] || tag="v${tag}"
  else
    info "Resolving latest release from github.com/${REPO}..."
    tag="$(resolve_latest_tag)"
  fi
  clean_tag="${tag#v}"
  current_ver="$(installed_version)"

  if [[ "$MODE" == "update" && "$FORCE" != "true" && -n "$current_ver" && "$current_ver" == "$clean_tag" ]]; then
    ok "ferropod v${current_ver} is already up to date."
    exit 0
  fi

  if [[ -n "$current_ver" ]]; then
    info "Upgrading ferropod: v${current_ver} -> ${tag}"
  else
    info "Installing ferropod ${tag}"
  fi

  local pkg="ferropod-${tag}-${target}"
  local tarball="${pkg}.tar.gz"
  local base_url="https://github.com/${REPO}/releases/download/${tag}"
  local tmp_dir
  tmp_dir="$(mktemp -d)"
  trap 'rm -rf "$tmp_dir"' EXIT

  info "Downloading ${tarball}..."
  curl -fsSL -o "${tmp_dir}/${tarball}" "${base_url}/${tarball}" \
    || die "Failed to download ${base_url}/${tarball}"

  if curl -fsSL -o "${tmp_dir}/SHA256SUMS.txt" "${base_url}/SHA256SUMS.txt" 2>/dev/null; then
    local expected
    expected="$(awk -v f="$tarball" '{sub(/^\*/, "", $2); if ($2 == f) { print $1; exit }}' "${tmp_dir}/SHA256SUMS.txt")"
    if [[ -n "$expected" ]]; then
      verify_sha256 "${tmp_dir}/${tarball}" "$expected"
    else
      warn "Checksum entry for ${tarball} not found in SHA256SUMS.txt"
    fi
  else
    warn "SHA256SUMS.txt not found for ${tag}; skipping checksum verification"
  fi

  tar -xzf "${tmp_dir}/${tarball}" -C "$tmp_dir"
  local extracted_bin="${tmp_dir}/${pkg}/ferropod"
  [[ -f "$extracted_bin" ]] || extracted_bin="$(find "$tmp_dir" -maxdepth 3 -type f -name ferropod | head -n 1)"
  [[ -f "$extracted_bin" ]] || die "ferropod binary not found inside ${tarball}"
  chmod 755 "$extracted_bin"

  mkdir -p "$BIN_DIR"
  if [[ -f "${BIN_DIR}/ferropod" ]]; then
    cp -f "${BIN_DIR}/ferropod" "${BIN_DIR}/ferropod.bak" 2>/dev/null || true
  fi
  mv -f "$extracted_bin" "${BIN_DIR}/ferropod"
  ok "Installed $("${BIN_DIR}/ferropod" --version) to ${BIN_DIR}/ferropod"

  if [[ "$MODE" == "update" ]]; then
    if has_systemd && systemctl is-active --quiet ferropod 2>/dev/null; then
      info "Restarting systemd service ferropod..."
      systemctl restart ferropod
      check_health
    fi
  else
    if has_systemd; then
      setup_systemd_service
      check_health
    else
      info "Run 'ferropod dev' for local development or 'ferropod serve --help' for server options."
    fi
  fi
}

main "$@"
