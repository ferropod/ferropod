// Native HTML Web Component: <fp-system-updates>
// System Version, GitHub Releases Browser, One-Click Self-Update & Install Scripts.

const updateSheet = new CSSStyleSheet();
updateSheet.replaceSync(`
  *, *::before, *::after { box-sizing: border-box; }
  :host {
    display: block;
    color: var(--text);
    font-family: var(--font-sans);
    font-size: 14px;
    line-height: 1.5;
    touch-action: manipulation;
    max-width: 100%;
  }
  button, input { font: inherit; color: inherit; }
  code, pre, .mono { font-family: var(--font-mono); }
  a { color: var(--accent); text-decoration: none; }
  a:hover { text-decoration: underline; }
  .muted { color: var(--muted); }
  .text-xs { font-size: 12px; }
  .text-strong { font-weight: 600; color: var(--text); }
  .row { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
  .row-between { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 12px; }
  .stack { display: flex; flex-direction: column; }
  .gap-xs { gap: 6px; }
  .gap-sm { gap: 10px; }
  .gap-md { gap: 16px; }

  .page-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
    margin-bottom: 16px;
  }
  .page-title { margin: 0; font-size: 20px; font-weight: 700; letter-spacing: -0.02em; }
  .page-subtitle { margin: 4px 0 0; color: var(--muted); font-size: 13px; }

  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-sm);
    overflow: hidden;
    margin-bottom: 16px;
  }
  .panel-header {
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .panel-title { margin: 0; font-size: 14px; font-weight: 600; display: flex; align-items: center; gap: 8px; }
  .panel-body { padding: 16px; }

  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(175px, 1fr));
    gap: 12px;
  }
  .info-card {
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
  }
  .info-label { font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .info-val { font-size: 14px; font-weight: 600; margin-top: 3px; word-break: break-all; }

  .banner {
    padding: 14px 16px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
    margin-top: 14px;
  }
  .banner-update {
    background: color-mix(in srgb, var(--accent) 8%, var(--surface));
    border-color: color-mix(in srgb, var(--accent) 40%, var(--border));
  }
  .banner-ok {
    background: color-mix(in srgb, var(--success, #16a34a) 8%, var(--surface));
    border-color: color-mix(in srgb, var(--success, #16a34a) 35%, var(--border));
  }
  .banner-warn {
    background: color-mix(in srgb, var(--warn, #d97706) 8%, var(--surface));
    border-color: color-mix(in srgb, var(--warn, #d97706) 35%, var(--border));
  }
  .banner-error {
    background: var(--danger-soft, rgba(220, 38, 38, 0.08));
    border-color: color-mix(in srgb, var(--danger, #dc2626) 40%, var(--border));
    color: var(--danger, #dc2626);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 7px 12px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .btn:hover:not(:disabled) { background: var(--surface-subtle); border-color: var(--border-strong); }
  .btn:disabled { opacity: 0.55; cursor: not-allowed; }
  .btn-sm { padding: 5px 10px; font-size: 12px; }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .btn-primary:hover:not(:disabled) { background: var(--accent-hover); border-color: var(--accent-hover); }
  .btn-ghost { background: transparent; border-color: transparent; color: var(--muted); }
  .btn-ghost:hover:not(:disabled) { color: var(--text); background: var(--surface-subtle); }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    border: 1px solid var(--border);
    background: var(--surface-subtle);
  }
  .badge-accent {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
    color: var(--accent);
  }
  .badge-success {
    background: color-mix(in srgb, var(--success, #16a34a) 12%, transparent);
    border-color: color-mix(in srgb, var(--success, #16a34a) 35%, transparent);
    color: var(--success, #16a34a);
  }
  .badge-warn {
    background: color-mix(in srgb, var(--warn, #d97706) 14%, transparent);
    border-color: color-mix(in srgb, var(--warn, #d97706) 35%, transparent);
    color: var(--warn, #d97706);
  }

  .cmd-list { display: flex; flex-direction: column; gap: 10px; }
  .cmd-item {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-subtle);
    padding: 10px 12px;
  }
  .cmd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 6px;
  }
  .cmd-code {
    display: block;
    font-size: 12px;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
    overflow-x: auto;
    white-space: nowrap;
    color: var(--text);
  }

  .release-list { display: flex; flex-direction: column; }
  .release-row {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
  }
  .release-row:last-child { border-bottom: none; }
  .release-notes {
    margin: 10px 0 0;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 220px;
    overflow-y: auto;
    color: var(--text);
  }
  .empty-state {
    padding: 28px 16px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
  .spinner {
    display: inline-block;
    width: 13px;
    height: 13px;
    border: 2px solid currentColor;
    border-right-color: transparent;
    border-radius: 50%;
    animation: fp-spin 0.7s linear infinite;
  }
  @keyframes fp-spin { to { transform: rotate(360deg); } }
  @media (max-width: 639px) {
    .panel-header, .panel-body { padding: 12px 14px; }
    .info-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .btn { min-height: 38px; }
    .banner > div { min-width: 0 !important; width: 100%; }
  }
`);

function esc(val) {
  if (val === null || val === undefined) return '';
  return String(val)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

class FpSystemUpdates extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [updateSheet];
    this._token = '';
    this._scope = '';
    this._info = null;
    this._check = null;
    this._releases = [];
    this._loadingInfo = false;
    this._checking = false;
    this._loadingReleases = false;
    this._applyingTag = null;
    this._restarting = false;
    this._error = '';
    this._expandedTags = new Set();
    this._pollTimer = null;
    this._onLangChange = () => { if (this.isConnected) this.render(); };
  }

  set token(v) { this._token = v || ''; }
  set scope(v) { this._scope = v || ''; }

  t(key, vars) {
    return window.FerroI18n ? window.FerroI18n.t(key, vars) : key;
  }

  formatBytes(n) {
    return window.FerroI18n ? window.FerroI18n.formatBytes(n) : String(n || 0);
  }

  isAdmin() {
    return this._scope === 'admin';
  }

  toast(message, kind = 'info') {
    this.dispatchEvent(new CustomEvent('fp-toast', {
      bubbles: true,
      composed: true,
      detail: { message, kind }
    }));
  }

  requestConfirm(message, onConfirm, opts = {}) {
    const ev = new CustomEvent('fp-confirm', {
      bubbles: true,
      composed: true,
      cancelable: true,
      detail: { message, onConfirm, ...opts }
    });
    this.dispatchEvent(ev);
    if (!ev.defaultPrevented && window.confirm(message)) {
      onConfirm();
    }
  }

  async api(path, opts = {}) {
    const headers = Object.assign({ Accept: 'application/json' }, opts.headers || {});
    if (this._token) headers.Authorization = 'Bearer ' + this._token;
    if (opts.body && !headers['Content-Type']) headers['Content-Type'] = 'application/json';
    const resp = await fetch(path, Object.assign({}, opts, { headers }));
    const text = await resp.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch (_) { data = text; }
    if (!resp.ok) {
      const msg = (data && (data.error || data.message)) || text || ('HTTP ' + resp.status);
      throw new Error(msg);
    }
    return data;
  }

  connectedCallback() {
    window.addEventListener('fp-lang-change', this._onLangChange);
    this.render();
    this.loadInitial();
  }

  disconnectedCallback() {
    window.removeEventListener('fp-lang-change', this._onLangChange);
    if (this._pollTimer) {
      clearInterval(this._pollTimer);
      this._pollTimer = null;
    }
  }

  async loadInitial() {
    this._loadingInfo = true;
    this._error = '';
    this.render();
    try {
      this._info = await this.api('/_api/v1/system/info');
    } catch (e) {
      this._error = e.message;
    } finally {
      this._loadingInfo = false;
      this.render();
    }
    if (this.isAdmin()) {
      this.checkUpdates(false);
      this.loadReleases();
    }
  }

  async checkUpdates(showToast = true) {
    if (!this.isAdmin() || this._checking) return;
    this._checking = true;
    this._error = '';
    this.render();
    try {
      this._check = await this.api('/_api/v1/system/update/check');
      if (showToast) {
        if (this._check.update_available && this._check.latest) {
          this.toast(this.t('updates.foundNew', { version: this._check.latest.tag }), 'info');
        } else {
          this.toast(this.t('updates.upToDate'), 'success');
        }
      }
    } catch (e) {
      this._error = e.message;
      if (showToast) this.toast(e.message, 'error');
    } finally {
      this._checking = false;
      this.render();
    }
  }

  async loadReleases() {
    if (!this.isAdmin() || this._loadingReleases) return;
    this._loadingReleases = true;
    this.render();
    try {
      this._releases = (await this.api('/_api/v1/system/releases?limit=12')) || [];
    } catch (e) {
      if (!this._error) this._error = e.message;
    } finally {
      this._loadingReleases = false;
      this.render();
    }
  }

  applyUpdate(tag, force = false) {
    if (!this.isAdmin() || this._applyingTag || this._restarting) return;
    const label = tag || (this._check && this._check.latest ? this._check.latest.tag : 'latest');
    const confirmMsg = this.t('updates.confirmApply', { version: label });
    this.requestConfirm(confirmMsg, () => {
      this._doApplyUpdate(tag, label, force);
    }, {
      title: this.t('updates.title'),
      confirmLabel: this.t('common.confirm'),
      danger: false
    });
  }

  async _doApplyUpdate(tag, label, force) {
    this._applyingTag = label;
    this._error = '';
    this.render();
    try {
      const res = await this.api('/_api/v1/system/update/apply', {
        method: 'POST',
        body: JSON.stringify({
          version: tag || null,
          force: Boolean(force),
          restart: true
        })
      });
      if (res && res.restarting) {
        this._restarting = true;
        this.toast(this.t('updates.restartingToast', { version: res.target_tag }), 'success');
        this.render();
        this.waitForRestartAndReload();
      } else {
        this.toast((res && res.message) || this.t('updates.upToDate'), 'info');
      }
    } catch (e) {
      this._error = e.message;
      this.toast(e.message, 'error');
    } finally {
      this._applyingTag = null;
      this.render();
    }
  }

  waitForRestartAndReload() {
    if (this._pollTimer) clearInterval(this._pollTimer);
    let attempts = 0;
    const maxAttempts = 40;
    setTimeout(() => {
      this._pollTimer = setInterval(async () => {
        attempts += 1;
        try {
          const r = await fetch('/_healthz', { cache: 'no-store' });
          if (r.ok) {
            clearInterval(this._pollTimer);
            this._pollTimer = null;
            this.toast(this.t('updates.restartDone'), 'success');
            setTimeout(() => window.location.reload(), 500);
            return;
          }
        } catch (_) {}
        if (attempts >= maxAttempts) {
          clearInterval(this._pollTimer);
          this._pollTimer = null;
          this._restarting = false;
          this._error = this.t('updates.restartTimeout');
          this.render();
        }
      }, 1500);
    }, 1200);
  }

  async copyText(text) {
    try {
      await navigator.clipboard.writeText(text);
      this.toast(this.t('common.copied'), 'success');
    } catch (_) {
      this.toast(text, 'info');
    }
  }

  renderStatusBanner() {
    if (this._restarting) {
      return `
        <div class="banner banner-update">
          <div class="row">
            <span class="spinner"></span>
            <div>
              <div class="text-strong">${esc(this.t('updates.restartingTitle'))}</div>
              <div class="muted text-xs">${esc(this.t('updates.restartingDesc'))}</div>
            </div>
          </div>
        </div>`;
    }

    if (this._error) {
      return `<div class="banner banner-error"><div>${esc(this._error)}</div></div>`;
    }

    if (!this.isAdmin()) {
      return `<div class="banner banner-warn"><div class="text-xs">${esc(this.t('updates.adminRequired'))}</div></div>`;
    }

    const info = this._info || {};
    const chk = this._check;
    if (!chk) {
      return this._checking
        ? `<div class="banner"><div class="row"><span class="spinner"></span><span>${esc(this.t('updates.checking'))}</span></div></div>`
        : '';
    }

    if (!chk.latest) {
      return `
        <div class="banner banner-warn">
          <div>
            <div class="text-strong">${esc(this.t('updates.noReleasesTitle'))}</div>
            <div class="muted text-xs">${esc(this.t('updates.noReleasesDesc', { repo: chk.repo }))}</div>
          </div>
          <a class="btn btn-sm" href="https://github.com/${esc(chk.repo)}/releases" target="_blank" rel="noopener">${esc(this.t('updates.openGithub'))}</a>
        </div>`;
    }

    const rel = chk.latest;
    const isDocker = info.install_mode === 'docker';
    const busy = Boolean(this._applyingTag);

    if (chk.update_available) {
      let actionHtml = '';
      if (isDocker) {
        actionHtml = `<button type="button" class="btn btn-primary btn-sm" data-copy="docker compose pull && docker compose up -d">${esc(this.t('updates.copyDockerCmd'))}</button>`;
      } else if (info.can_self_update && rel.asset_url) {
        actionHtml = `
          <button type="button" class="btn btn-primary btn-sm" data-apply="${esc(rel.tag)}" ${busy ? 'disabled' : ''}>
            ${busy ? '<span class="spinner"></span>' : ''}
            ${esc(busy ? this.t('updates.updating') : this.t('updates.updateNow', { version: rel.tag }))}
          </button>`;
      } else {
        actionHtml = `<a class="btn btn-primary btn-sm" href="${esc(rel.html_url)}" target="_blank" rel="noopener">${esc(this.t('updates.viewRelease'))}</a>`;
      }

      return `
        <div class="banner banner-update">
          <div style="flex:1;min-width:220px;">
            <div class="row gap-xs">
              <span class="badge badge-accent">${esc(this.t('updates.newAvailable'))}</span>
              <span class="text-strong mono">v${esc(chk.current_version)} → ${esc(rel.tag)}</span>
              ${rel.asset_size ? `<span class="muted text-xs">(${esc(this.formatBytes(rel.asset_size))})</span>` : ''}
            </div>
            ${rel.body ? `<pre class="release-notes">${esc(rel.body)}</pre>` : ''}
            ${!info.can_self_update && !isDocker ? `<div class="muted text-xs" style="margin-top:6px;">${esc(this.t('updates.notWritableHint'))}</div>` : ''}
          </div>
          <div class="row">
            <a class="btn btn-sm" href="${esc(rel.html_url)}" target="_blank" rel="noopener">${esc(this.t('updates.releaseNotes'))}</a>
            ${actionHtml}
          </div>
        </div>`;
    }

    return `
      <div class="banner banner-ok">
        <div class="row">
          <span class="badge badge-success">${esc(this.t('updates.upToDate'))}</span>
          <span class="text-xs muted">${esc(this.t('updates.latestLabel'))}: <strong class="mono">${esc(rel.tag)}</strong></span>
        </div>
        <a class="btn btn-sm" href="${esc(rel.html_url)}" target="_blank" rel="noopener">${esc(this.t('updates.openGithub'))}</a>
      </div>`;
  }

  renderScriptsPanel() {
    const repo = (this._info && this._info.repo) || 'ferropod/ferropod';
    const installCmd = `curl -fsSL https://raw.githubusercontent.com/${repo}/main/scripts/install.sh | sudo bash`;
    const updateScriptCmd = `curl -fsSL https://raw.githubusercontent.com/${repo}/main/scripts/install.sh | sudo bash -s -- --update`;
    const cliUpdateCmd = `ferropod update`;
    const dockerUpdateCmd = `docker compose pull && docker compose up -d`;

    const cmds = [
      { label: this.t('updates.scriptInstall'), cmd: installCmd },
      { label: this.t('updates.scriptUpdate'), cmd: updateScriptCmd },
      { label: this.t('updates.cliUpdate'), cmd: cliUpdateCmd },
      { label: this.t('updates.dockerUpdate'), cmd: dockerUpdateCmd }
    ];

    return `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('updates.scriptsTitle'))}</h3>
          <span class="muted text-xs">${esc(this.t('updates.scriptsSubtitle'))}</span>
        </div>
        <div class="panel-body">
          <div class="cmd-list">
            ${cmds.map((c) => `
              <div class="cmd-item">
                <div class="cmd-head">
                  <span class="text-xs text-strong">${esc(c.label)}</span>
                  <button type="button" class="btn btn-ghost btn-sm" data-copy="${esc(c.cmd)}">${esc(this.t('common.copy'))}</button>
                </div>
                <code class="cmd-code">${esc(c.cmd)}</code>
              </div>
            `).join('')}
          </div>
        </div>
      </div>`;
  }

  renderReleasesPanel() {
    if (!this.isAdmin()) return '';
    const info = this._info || {};
    const curVer = info.current_version || '';
    const busy = Boolean(this._applyingTag) || this._restarting;

    let bodyHtml = '';
    if (this._loadingReleases && !this._releases.length) {
      bodyHtml = `<div class="empty-state">${esc(this.t('common.loading'))}</div>`;
    } else if (!this._releases.length) {
      bodyHtml = `<div class="empty-state">${esc(this.t('updates.noReleasesList'))}</div>`;
    } else {
      bodyHtml = `<div class="release-list">${this._releases.map((rel) => {
        const isCurrent = rel.version === curVer;
        const expanded = this._expandedTags.has(rel.tag);
        const locale = window.FerroI18n ? window.FerroI18n.getLocale() : undefined;
        const dateStr = rel.published_at ? new Date(rel.published_at).toLocaleString(locale) : '-';
        const canInstall = info.can_self_update && rel.asset_url && info.install_mode !== 'docker';
        return `
          <div class="release-row">
            <div class="row-between">
              <div class="row gap-xs">
                <span class="text-strong mono">${esc(rel.tag)}</span>
                ${isCurrent ? `<span class="badge badge-success">${esc(this.t('updates.currentBadge'))}</span>` : ''}
                ${rel.prerelease ? `<span class="badge badge-warn">Pre-release</span>` : ''}
                <span class="muted text-xs">${esc(dateStr)}</span>
                ${rel.asset_size ? `<span class="muted text-xs mono">${esc(this.formatBytes(rel.asset_size))}</span>` : ''}
              </div>
              <div class="row gap-xs">
                ${rel.body ? `<button type="button" class="btn btn-ghost btn-sm" data-toggle-notes="${esc(rel.tag)}">${esc(expanded ? this.t('updates.hideNotes') : this.t('updates.showNotes'))}</button>` : ''}
                <a class="btn btn-ghost btn-sm" href="${esc(rel.html_url)}" target="_blank" rel="noopener">GitHub</a>
                ${canInstall ? `
                  <button type="button" class="btn btn-sm ${isCurrent ? '' : 'btn-primary'}" data-apply="${esc(rel.tag)}" data-force="${isCurrent ? '1' : '0'}" ${busy ? 'disabled' : ''}>
                    ${esc(isCurrent ? this.t('updates.reinstall') : this.t('updates.installVersion'))}
                  </button>` : ''}
              </div>
            </div>
            ${expanded && rel.body ? `<pre class="release-notes">${esc(rel.body)}</pre>` : ''}
          </div>`;
      }).join('')}</div>`;
    }

    return `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('updates.releasesTitle'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-action="reload-releases" ${this._loadingReleases ? 'disabled' : ''}>
            ${esc(this.t('common.refresh'))}
          </button>
        </div>
        ${bodyHtml}
      </div>`;
  }

  render() {
    const info = this._info || {};
    const repo = info.repo || 'ferropod/ferropod';
    const modeLabel = info.install_mode === 'systemd'
      ? 'systemd'
      : info.install_mode === 'docker'
        ? 'Docker'
        : (info.install_mode || '-');

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">${esc(this.t('updates.title'))}</h2>
          <p class="page-subtitle">${esc(this.t('updates.subtitle'))}</p>
        </div>
        <div class="row">
          <a class="btn btn-sm" href="https://github.com/${esc(repo)}/releases" target="_blank" rel="noopener">${esc(this.t('updates.openGithub'))}</a>
          ${this.isAdmin() ? `
            <button type="button" class="btn btn-primary btn-sm" data-action="check" ${this._checking || this._restarting ? 'disabled' : ''}>
              ${this._checking ? '<span class="spinner"></span>' : ''}
              ${esc(this._checking ? this.t('updates.checking') : this.t('updates.checkNow'))}
            </button>` : ''}
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('updates.instanceTitle'))}</h3>
          <span class="badge badge-accent mono">v${esc(info.current_version || '-')}</span>
        </div>
        <div class="panel-body">
          <div class="info-grid">
            <div class="info-card">
              <div class="info-label">${esc(this.t('updates.currentVersion'))}</div>
              <div class="info-val mono">v${esc(info.current_version || '-')}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('updates.targetPlatform'))}</div>
              <div class="info-val mono">${esc(info.target || '-')}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('updates.installMode'))}</div>
              <div class="info-val">${esc(modeLabel)}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('updates.binaryPath'))}</div>
              <div class="info-val mono text-xs">${esc(info.binary_path || '-')}</div>
            </div>
          </div>
          ${this.renderStatusBanner()}
        </div>
      </div>

      ${this.renderScriptsPanel()}
      ${this.renderReleasesPanel()}
    `;

    this.bindEvents();
  }

  bindEvents() {
    const sr = this.shadowRoot;
    const checkBtn = sr.querySelector('[data-action="check"]');
    if (checkBtn) checkBtn.addEventListener('click', () => this.checkUpdates(true));

    const reloadRelBtn = sr.querySelector('[data-action="reload-releases"]');
    if (reloadRelBtn) reloadRelBtn.addEventListener('click', () => this.loadReleases());

    sr.querySelectorAll('[data-copy]').forEach((btn) => {
      btn.addEventListener('click', () => this.copyText(btn.getAttribute('data-copy')));
    });

    sr.querySelectorAll('[data-toggle-notes]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const tag = btn.getAttribute('data-toggle-notes');
        if (this._expandedTags.has(tag)) this._expandedTags.delete(tag);
        else this._expandedTags.add(tag);
        this.render();
      });
    });

    sr.querySelectorAll('[data-apply]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const tag = btn.getAttribute('data-apply');
        const force = btn.getAttribute('data-force') === '1';
        this.applyUpdate(tag, force);
      });
    });
  }
}

if (!customElements.get('fp-system-updates')) {
  customElements.define('fp-system-updates', FpSystemUpdates);
}
