// Native HTML Web Component: <fp-ops-center>
// Backups & Consistent Snapshots (real /_api/v1/backups API) + Scheduled Tasks / Cron Jobs (project-isolated preview).

const opsSheet = new CSSStyleSheet();
opsSheet.replaceSync(`
  *, *::before, *::after { box-sizing: border-box; }
  :host {
    display: block;
    color: var(--text);
    font-family: var(--font-sans);
    font-size: 14px;
    line-height: 1.5;
  }
  button, input, select { font: inherit; color: inherit; }
  code, pre, .mono { font-family: var(--font-mono); }
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

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 16px;
    overflow-x: auto;
  }
  .tab-btn {
    padding: 9px 14px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--muted);
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
  }
  .tab-btn:hover { color: var(--text); }
  .tab-btn.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  .panel {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    box-shadow: var(--shadow-sm);
    overflow: hidden;
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
  }
  .btn:hover { background: var(--surface-subtle); border-color: var(--border-strong); }
  .btn:disabled { opacity: 0.55; cursor: not-allowed; }
  .btn-sm { padding: 5px 10px; font-size: 12px; }
  .btn-primary { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .btn-primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); }
  .btn-danger { color: var(--danger); border-color: color-mix(in srgb, var(--danger) 35%, var(--border)); background: var(--danger-soft); }
  .btn-ghost { background: transparent; border-color: transparent; color: var(--text-secondary); }
  .btn-ghost:hover { background: var(--surface-subtle); }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }
  .badge-neutral { background: var(--surface-subtle); color: var(--text-secondary); border: 1px solid var(--border); }
  .badge-accent { background: var(--accent-soft); color: var(--accent); }
  .badge-success { background: var(--success-soft); color: var(--success); }
  .badge-warn { background: var(--warn-soft); color: var(--warn); }
  .badge-info { background: var(--info-soft); color: var(--info); }

  .table-wrap { width: 100%; overflow-x: auto; }
  table.data-table { width: 100%; border-collapse: collapse; font-size: 13px; }
  table.data-table th {
    text-align: left;
    padding: 9px 12px;
    font-size: 12px;
    color: var(--muted);
    border-bottom: 1px solid var(--border);
    background: var(--surface-subtle);
    font-weight: 600;
  }
  table.data-table td {
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
  }
  table.data-table tr:last-child td { border-bottom: none; }

  .code-block {
    margin: 0;
    padding: 10px 12px;
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    font-size: 12px;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .empty-state { padding: 28px 16px; text-align: center; color: var(--muted); font-size: 13px; }

  input[type="text"], select {
    width: 100%;
    padding: 8px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    outline: none;
  }
  input:focus, select:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }

  .preset-bar { display: flex; flex-wrap: wrap; gap: 6px; }
  .preset-chip {
    padding: 3px 9px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-subtle);
    font-size: 11.5px;
    cursor: pointer;
  }
  .preset-chip:hover { border-color: var(--accent); color: var(--accent); }

  dialog.fp-dialog {
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-md);
    width: min(500px, calc(100vw - 24px));
    overflow: hidden;
  }
  dialog.fp-dialog::backdrop { background: rgba(15, 23, 42, 0.55); backdrop-filter: blur(2px); }
  .dialog-head {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .dialog-title { margin: 0; font-size: 15px; font-weight: 600; }
  .dialog-body { padding: 16px; }
  .dialog-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    background: var(--surface-subtle);
  }
`);

function esc(v) {
  return String(v === null || v === undefined ? '' : v)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function getI18n() {
  return window.FerroI18n || {
    t: (k) => k,
    formatTime: (v) => String(v || ''),
    formatBytes: (b) => `${b || 0} B`
  };
}

function emitToast(el, message, kind = 'info') {
  el.dispatchEvent(new CustomEvent('fp-toast', {
    detail: { message, kind },
    bubbles: true,
    composed: true
  }));
}

// Built-in platform maintenance tasks (driven by tokio intervals in src/dispatcher/maintenance.rs)
const nowMs = Date.now();
const systemTasks = [
  { id: 'sys_kv_ttl', name: 'KV TTL Batch Cleanup', schedule: '*/1 * * * *', target: 'internal:kv_ttl_gc', enabled: true, last_run: nowMs - 42000, last_status: 'ok' },
  { id: 'sys_blob_gc', name: 'Orphan Blob Async GC', schedule: '*/5 * * * *', target: 'internal:blob_gc', enabled: true, last_run: nowMs - 180000, last_status: 'ok' },
  { id: 'sys_queue_lease', name: 'Queue Lease Reaper & DLQ', schedule: '*/1 * * * *', target: 'internal:queue_reaper', enabled: true, last_run: nowMs - 30000, last_status: 'ok' },
  { id: 'sys_daily_backup', name: 'Daily Auto Snapshot', schedule: '0 3 * * *', target: 'internal:auto_snapshot', enabled: true, last_run: nowMs - 3600000 * 6, last_status: 'ok' }
];

class FpOpsCenter extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [opsSheet];
    this._token = '';
    this._scope = 'admin';
    this._projects = [];
    this._tab = 'backups'; // backups | cron
    this._backups = [];
    this._loadingBackups = false;
    this._creatingBackup = false;
    this._backupErr = '';
    this._crons = [];
    this._loadingCrons = false;
    this._cronErr = '';
  }

  set token(val) {
    this._token = String(val || '');
    if (this.isConnected) {
      if (this._tab === 'backups' && this._isAdmin()) this.loadBackups();
      if (this._tab === 'cron') this.loadCrons();
    }
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (!this._isAdmin()) this._tab = 'cron';
    if (this.isConnected) this.render();
  }

  set projects(list) {
    this._projects = Array.isArray(list) ? list : [];
    if (this.isConnected) this.render();
  }

  _isAdmin() {
    return this._scope === 'admin';
  }

  _scopeProjectId() {
    return this._scope.startsWith('project:') ? this._scope.slice('project:'.length) : '';
  }

  _projectName(pid) {
    if (!pid) return '';
    const p = this._projects.find((item) => item.id === pid);
    return p ? p.name : pid;
  }

  _authHeaders(json = false) {
    const tok = this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
    const h = {};
    if (tok) h.Authorization = 'Bearer ' + tok;
    if (json) h['Content-Type'] = 'application/json';
    return h;
  }

  connectedCallback() {
    if (!this._isAdmin()) this._tab = 'cron';
    this.render();
    if (this._isAdmin()) {
      this.loadBackups();
    }
    this.loadCrons();
  }

  async loadCrons() {
    this._loadingCrons = true;
    this._cronErr = '';
    this.render();
    try {
      const res = await fetch('/_api/v1/crons', { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._crons = (await res.json()) || [];
    } catch (err) {
      this._cronErr = err.message || 'Failed to load cron jobs';
    } finally {
      this._loadingCrons = false;
      this.render();
    }
  }

  async createCron(payload) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/crons', {
        method: 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify(payload)
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      emitToast(this, t('ops.cronCreated'), 'success');
      await this.loadCrons();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async toggleCron(job) {
    try {
      const res = await fetch('/_api/v1/crons/' + encodeURIComponent(job.id), {
        method: 'PATCH',
        headers: this._authHeaders(true),
        body: JSON.stringify({ enabled: !job.enabled })
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      await this.loadCrons();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async runCron(job) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/crons/' + encodeURIComponent(job.id) + '/run', {
        method: 'POST',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const result = await res.json();
      if (result.last_status && result.last_status.startsWith('error:')) {
        throw new Error(result.last_status);
      }
      emitToast(this, t('ops.cronTriggered', { name: job.name }), 'success');
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
    await this.loadCrons();
  }

  async deleteCron(id) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/crons/' + encodeURIComponent(id), {
        method: 'DELETE',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      emitToast(this, t('ops.cronDeleted'), 'success');
      await this.loadCrons();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async loadBackups() {
    if (!this._isAdmin()) return;
    this._loadingBackups = true;
    this._backupErr = '';
    this.render();
    try {
      const res = await fetch('/_api/v1/backups', { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._backups = (await res.json()) || [];
    } catch (err) {
      this._backupErr = err.message || 'Failed to load backups';
    } finally {
      this._loadingBackups = false;
      this.render();
    }
  }

  async createBackup(label) {
    const { t } = getI18n();
    this._creatingBackup = true;
    this.render();
    try {
      const res = await fetch('/_api/v1/backups', {
        method: 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify({ label: label || null })
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      emitToast(this, t('ops.backupCreated'), 'success');
      await this.loadBackups();
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._creatingBackup = false;
      this.render();
    }
  }

  async deleteBackup(name) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/backups/' + encodeURIComponent(name), {
        method: 'DELETE',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      emitToast(this, t('ops.backupDeleted'), 'success');
      await this.loadBackups();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  render() {
    const { t, formatTime, formatBytes } = getI18n();
    const isAdmin = this._isAdmin();
    const scopePid = this._scopeProjectId();
    const canWriteCron = this._scope !== 'readonly';

    const tabsHtml = `
      <div class="tabs">
        ${isAdmin ? `<button type="button" class="tab-btn ${this._tab === 'backups' ? 'active' : ''}" data-ops-tab="backups">${esc(t('ops.tabBackups'))}</button>` : ''}
        <button type="button" class="tab-btn ${this._tab === 'cron' ? 'active' : ''}" data-ops-tab="cron">${esc(t('ops.tabCron'))}</button>
      </div>
    `;

    let bodyHtml = '';
    if (this._tab === 'backups' && isAdmin) {
      const rows = this._backups.map((b) => `
        <tr>
          <td>
            <div class="mono text-strong">${esc(b.name)}</div>
            <details style="margin-top:4px;">
              <summary class="muted text-xs" style="cursor:pointer;">${esc(t('ops.filesCount', { count: b.file_count }))}</summary>
              <pre class="code-block" style="margin-top:6px;">${esc((b.files || []).join('\n'))}</pre>
            </details>
          </td>
          <td class="mono text-xs">${esc(formatBytes(b.size_bytes))}</td>
          <td class="muted text-xs">${esc(formatTime(b.created_at_ms))}</td>
          <td>
            <button type="button" class="btn btn-danger btn-sm" data-del-backup="${esc(b.name)}">${esc(t('common.delete'))}</button>
          </td>
        </tr>
      `).join('');

      bodyHtml = `
        <div class="stack gap-md">
          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">
                <span>${esc(t('ops.backupsList'))}</span>
                <span class="badge badge-success">Real API</span>
              </h3>
              <div class="row gap-xs">
                <button type="button" class="btn btn-sm" id="reload-backups-btn">${esc(t('common.refresh'))}</button>
                <button type="button" class="btn btn-primary btn-sm" id="new-backup-btn" ${this._creatingBackup ? 'disabled' : ''}>${esc(this._creatingBackup ? t('ops.creating') : t('ops.createBackup'))}</button>
              </div>
            </div>
            ${this._backupErr ? `<div class="panel-body"><div class="badge badge-warn">${esc(this._backupErr)}</div></div>` : ''}
            ${this._loadingBackups
              ? `<div class="empty-state">${esc(t('common.loading'))}</div>`
              : (rows
                ? `<div class="table-wrap"><table class="data-table"><thead><tr><th>${esc(t('ops.snapshotName'))}</th><th>${esc(t('ops.size'))}</th><th>${esc(t('common.createdAt'))}</th><th>${esc(t('common.actions'))}</th></tr></thead><tbody>${rows}</tbody></table></div>`
                : `<div class="empty-state">${esc(t('ops.noBackups'))}</div>`)
            }
          </div>

          <div class="panel">
            <div class="panel-header"><h3 class="panel-title">${esc(t('ops.offsiteHintTitle'))}</h3></div>
            <div class="panel-body">
              <pre class="code-block"># CLI manual snapshot & off-site sync (rclone / restic)
ferropod snapshot --out ./data/backups/manual
rclone sync ./data/backups remote:ferropod-backups</pre>
            </div>
          </div>
        </div>
      `;
    } else {
      const visibleProjectTasks = this._crons;
      bodyHtml = `
        <div class="stack gap-md">
          ${isAdmin ? `
            <div class="panel">
              <div class="panel-header">
                <h3 class="panel-title">
                  <span>${esc(t('ops.systemCrons'))}</span>
                  <span class="badge badge-info">Built-in</span>
                </h3>
              </div>
              <div class="table-wrap">
                <table class="data-table">
                  <thead>
                    <tr>
                      <th>${esc(t('common.name'))}</th>
                      <th>Cron</th>
                      <th>${esc(t('ops.target'))}</th>
                      <th>${esc(t('ops.lastRun'))}</th>
                      <th>${esc(t('common.actions'))}</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${systemTasks.map((c) => `
                      <tr>
                        <td class="text-strong">${esc(c.name)}</td>
                        <td class="mono text-xs"><span class="badge badge-neutral">${esc(c.schedule)}</span></td>
                        <td class="mono text-xs muted">${esc(c.target)}</td>
                        <td class="text-xs"><span class="badge badge-success">${esc(c.last_status)}</span> <span class="muted">${esc(formatTime(c.last_run))}</span></td>
                        <td><button type="button" class="btn btn-sm" data-run-sys="${esc(c.id)}">${esc(t('ops.runNow'))}</button></td>
                      </tr>
                    `).join('')}
                  </tbody>
                </table>
              </div>
            </div>
          ` : ''}

          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">
                <span>${esc(t('ops.projectCrons'))}</span>
                <span class="badge badge-success">core.db</span>
              </h3>
              <div class="row gap-xs">
                <button type="button" class="btn btn-sm" id="reload-crons-btn">${esc(t('common.refresh'))}</button>
                ${canWriteCron ? `<button type="button" class="btn btn-primary btn-sm" id="new-cron-btn">${esc(t('ops.newCron'))}</button>` : ''}
              </div>
            </div>
            ${this._cronErr ? `<div class="panel-body"><div class="badge badge-warn">${esc(this._cronErr)}</div></div>` : ''}
            ${this._loadingCrons ? `<div class="empty-state">${esc(t('common.loading'))}</div>` : (visibleProjectTasks.length ? `
              <div class="table-wrap">
                <table class="data-table">
                  <thead>
                    <tr>
                      <th>${esc(t('common.name'))}</th>
                      <th>${esc(t('common.project'))}</th>
                      <th>Cron</th>
                      <th>${esc(t('ops.target'))}</th>
                      <th>${esc(t('ops.lastRun'))}</th>
                      ${canWriteCron ? `<th>${esc(t('common.actions'))}</th>` : ''}
                    </tr>
                  </thead>
                  <tbody>
                    ${visibleProjectTasks.map((c) => `
                      <tr>
                        <td>
                          <div class="text-strong">${esc(c.name)}</div>
                          <span class="badge ${c.enabled ? 'badge-success' : 'badge-neutral'}">${esc(c.enabled ? t('ops.enabled') : t('ops.disabled'))}</span>
                        </td>
                        <td class="text-xs">${c.project_id ? `<span class="badge badge-accent">${esc(this._projectName(c.project_id))}</span>` : `<span class="muted">${esc(t('common.allProjects'))}</span>`}</td>
                        <td class="mono text-xs"><span class="badge badge-neutral">${esc(c.schedule)}</span></td>
                        <td class="mono text-xs muted">${esc(c.target)}</td>
                        <td class="text-xs muted">${c.last_run_at ? esc(formatTime(c.last_run_at)) : '-'}</td>
                        ${canWriteCron ? `
                          <td class="row gap-xs">
                            <button type="button" class="btn btn-sm" data-run-cron="${esc(c.id)}">${esc(t('ops.runNow'))}</button>
                            <button type="button" class="btn btn-sm" data-toggle-cron="${esc(c.id)}">${esc(c.enabled ? t('perf.pause') : t('perf.resume'))}</button>
                            <button type="button" class="btn btn-danger btn-sm" data-del-cron="${esc(c.id)}">${esc(t('common.delete'))}</button>
                          </td>
                        ` : ''}
                      </tr>
                    `).join('')}
                  </tbody>
                </table>
              </div>
            ` : `<div class="empty-state">${esc(t('ops.noCrons'))}</div>`)}
          </div>
        </div>
      `;
    }

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">${esc(t('ops.title'))}</h2>
          <p class="page-subtitle">${esc(t('ops.subtitle'))}</p>
        </div>
      </div>
      ${tabsHtml}
      ${bodyHtml}

      <dialog id="new-backup-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('ops.createBackup'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dlg="new-backup-dialog">✕</button>
        </div>
        <form id="new-backup-form">
          <div class="dialog-body stack gap-md">
            <div class="field">
              <label for="bk-label">${esc(t('ops.backupLabel'))}</label>
              <input type="text" id="bk-label" placeholder="${esc(t('ops.backupLabelPlaceholder'))}">
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dlg="new-backup-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('ops.createBackup'))}</button>
          </div>
        </form>
      </dialog>

      <dialog id="new-cron-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('ops.newCron'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dlg="new-cron-dialog">✕</button>
        </div>
        <form id="new-cron-form">
          <div class="dialog-body stack gap-md">
            <div class="field">
              <label for="cr-name">${esc(t('common.name'))}</label>
              <input type="text" id="cr-name" required placeholder="nightly-cleanup">
            </div>
            <div class="field">
              <label for="cr-project">${esc(t('common.project'))}</label>
              <select id="cr-project" ${scopePid ? 'disabled' : ''}>
                ${scopePid
                  ? `<option value="${esc(scopePid)}" selected>${esc(this._projectName(scopePid))}</option>`
                  : `<option value="">${esc(t('common.allProjects'))}</option>` +
                    this._projects.map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join('')
                }
              </select>
            </div>
            <div class="field">
              <label for="cr-schedule">Cron (5-field)</label>
              <input type="text" id="cr-schedule" class="mono" required value="0 3 * * *">
              <div class="preset-bar">
                <button type="button" class="preset-chip" data-preset="*/5 * * * *">*/5 * * * *</button>
                <button type="button" class="preset-chip" data-preset="0 * * * *">Hourly</button>
                <button type="button" class="preset-chip" data-preset="0 3 * * *">Daily 03:00</button>
                <button type="button" class="preset-chip" data-preset="0 4 * * 1">Weekly Mon</button>
              </div>
            </div>
            <div class="field">
              <label for="cr-target">${esc(t('ops.target'))}</label>
              <input type="text" id="cr-target" class="mono" required value="POST /_internal/cron">
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dlg="new-cron-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.create'))}</button>
          </div>
        </form>
      </dialog>
    `;

    this._bindEvents();
  }

  _bindEvents() {
    const root = this.shadowRoot;
    const { t } = getI18n();

    root.querySelectorAll('[data-ops-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._tab = btn.getAttribute('data-ops-tab');
        if (this._tab === 'backups' && this._isAdmin()) {
          this.loadBackups();
        } else {
          this.render();
        }
      });
    });

    root.querySelectorAll('[data-close-dlg]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const d = root.getElementById(btn.getAttribute('data-close-dlg'));
        if (d && d.open) d.close();
      });
    });

    const reloadBtn = root.getElementById('reload-backups-btn');
    if (reloadBtn) reloadBtn.addEventListener('click', () => this.loadBackups());

    const newBackupBtn = root.getElementById('new-backup-btn');
    if (newBackupBtn) {
      newBackupBtn.addEventListener('click', () => root.getElementById('new-backup-dialog').showModal());
    }

    const newBackupForm = root.getElementById('new-backup-form');
    if (newBackupForm) {
      newBackupForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        const label = root.getElementById('bk-label').value.trim();
        root.getElementById('new-backup-dialog').close();
        this.createBackup(label);
      });
    }

    root.querySelectorAll('[data-del-backup]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const name = btn.getAttribute('data-del-backup');
        if (window.confirm(t('ops.deleteBackupConfirm', { name }))) {
          this.deleteBackup(name);
        }
      });
    });

    const newCronBtn = root.getElementById('new-cron-btn');
    if (newCronBtn) {
      newCronBtn.addEventListener('click', () => root.getElementById('new-cron-dialog').showModal());
    }

    root.querySelectorAll('[data-preset]').forEach((chip) => {
      chip.addEventListener('click', () => {
        root.getElementById('cr-schedule').value = chip.getAttribute('data-preset');
      });
    });

    const reloadCronsBtn = root.getElementById('reload-crons-btn');
    if (reloadCronsBtn) reloadCronsBtn.addEventListener('click', () => this.loadCrons());

    const newCronForm = root.getElementById('new-cron-form');
    if (newCronForm) {
      newCronForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        const projEl = root.getElementById('cr-project');
        const pid = this._scopeProjectId() || (projEl ? (projEl.value || null) : null);
        const payload = {
          name: root.getElementById('cr-name').value.trim(),
          project_id: pid,
          schedule: root.getElementById('cr-schedule').value.trim(),
          target: root.getElementById('cr-target').value.trim()
        };
        root.getElementById('new-cron-dialog').close();
        this.createCron(payload);
      });
    }

    root.querySelectorAll('[data-run-sys]').forEach((btn) => {
      btn.addEventListener('click', async () => {
        const id = btn.getAttribute('data-run-sys');
        const item = systemTasks.find((c) => c.id === id);
        if (!item) return;
        item.last_run = Date.now();
        item.last_status = 'ok';
        if (id === 'sys_daily_backup' && this._isAdmin()) {
          await this.createBackup('auto');
        } else {
          emitToast(this, t('ops.cronTriggered', { name: item.name }), 'success');
          this.render();
        }
      });
    });

    root.querySelectorAll('[data-run-cron]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = this._crons.find((c) => c.id === btn.getAttribute('data-run-cron'));
        if (item) this.runCron(item);
      });
    });

    root.querySelectorAll('[data-toggle-cron]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const item = this._crons.find((c) => c.id === btn.getAttribute('data-toggle-cron'));
        if (item) this.toggleCron(item);
      });
    });

    root.querySelectorAll('[data-del-cron]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.deleteCron(btn.getAttribute('data-del-cron'));
      });
    });
  }
}

if (!customElements.get('fp-ops-center')) {
  customElements.define('fp-ops-center', FpOpsCenter);
}
