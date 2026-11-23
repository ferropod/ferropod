// Native HTML Web Component: <fp-ops-center>
// Backups & Snapshots (create, download .tar.gz, restore, auto-schedule), Scheduled Cron Jobs,
// Multi-Webhook Alerts, and Realtime Pub-Sub Channels & Live SSE Inspector.

const opsSheet = new CSSStyleSheet();
opsSheet.replaceSync(`
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
  button, input, select, textarea { font: inherit; color: inherit; }
  code, pre, .mono { font-family: var(--font-mono); }
  .muted { color: var(--muted); }
  .text-xs { font-size: 12px; }
  .text-strong { font-weight: 600; color: var(--text); }
  .break-all { word-break: break-all; overflow-wrap: anywhere; }
  .cell-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; }
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
  .badge-danger { background: var(--danger-soft); color: var(--danger); }
  .badge-info { background: var(--info-soft); color: var(--info); }

  .table-wrap { width: 100%; overflow-x: auto; -webkit-overflow-scrolling: touch; }
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
    max-height: 280px;
    overflow-y: auto;
  }
  .empty-state { padding: 28px 16px; text-align: center; color: var(--muted); font-size: 13px; }

  input[type="text"], input[type="number"], input[type="url"], input[type="password"], select, textarea {
    width: 100%;
    padding: 8px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    outline: none;
  }
  textarea { min-height: 76px; resize: vertical; }
  input:focus, select:focus, textarea:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
  .checkbox-row { display: inline-flex; align-items: center; gap: 7px; font-size: 12.5px; cursor: pointer; }
  .checkbox-row input { width: auto; }

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
    width: min(520px, calc(100vw - 24px));
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
  .dialog-body { padding: 16px; overflow-y: auto; max-height: 68vh; }
  .dialog-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    background: var(--surface-subtle);
  }
  @media (max-width: 767px) {
    .form-grid { grid-template-columns: 1fr; }
    .panel-header, .panel-body { padding: 12px 14px; }
    input[type="text"], input[type="number"], input[type="url"], input[type="password"], select, textarea { font-size: 16px; min-height: 42px; }
    .btn { min-height: 38px; }
    .btn-sm { min-height: 36px; }
    dialog.fp-dialog { width: calc(100vw - 16px); max-height: 90dvh; }

    .data-table.stack-mobile thead { display: none; }
    .data-table.stack-mobile,
    .data-table.stack-mobile tbody,
    .data-table.stack-mobile tr,
    .data-table.stack-mobile td { display: block; width: 100%; }
    .data-table.stack-mobile tr { padding: 12px 14px; border-bottom: 1px solid var(--border); }
    .data-table.stack-mobile tr:last-child { border-bottom: none; }
    .data-table.stack-mobile td {
      border: none;
      padding: 5px 0;
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      flex-wrap: wrap;
      gap: 10px;
      text-align: right;
      min-width: 0;
      overflow-wrap: anywhere;
      word-break: break-word;
    }
    .data-table.stack-mobile td > * { min-width: 0; max-width: 100%; }
    .data-table.stack-mobile td::before {
      content: attr(data-label);
      color: var(--muted);
      font-size: 12px;
      font-weight: 500;
      text-align: left;
      flex-shrink: 0;
    }
    .data-table.stack-mobile td.cell-primary {
      display: block;
      text-align: left;
      padding-bottom: 8px;
      margin-bottom: 4px;
      border-bottom: 1px dashed var(--border);
    }
    .data-table.stack-mobile td.cell-primary::before { display: none; }
    .data-table.stack-mobile td.cell-actions-td {
      display: block;
      padding: 10px 0 0;
      margin-top: 6px;
      border-top: 1px dashed var(--border);
    }
    .data-table.stack-mobile td.cell-actions-td::before { display: none; }
    .data-table.stack-mobile td.cell-actions-td .cell-actions { width: 100%; justify-content: flex-end; }
    .data-table.stack-mobile td.cell-actions-td .btn { flex: 1 1 auto; }
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

const nowMs = Date.now();
const systemTasks = [
  { id: 'sys_kv_ttl', name: 'KV TTL Batch Cleanup', schedule: '*/1 * * * *', target: 'internal:kv_ttl_gc', enabled: true, last_run: nowMs - 42000, last_status: 'ok' },
  { id: 'sys_blob_gc', name: 'Orphan Blob Async GC', schedule: '*/5 * * * *', target: 'internal:blob_gc', enabled: true, last_run: nowMs - 180000, last_status: 'ok' },
  { id: 'sys_queue_lease', name: 'Queue Lease Reaper & DLQ', schedule: '*/1 * * * *', target: 'internal:queue_reaper', enabled: true, last_run: nowMs - 30000, last_status: 'ok' },
  { id: 'sys_daily_backup', name: 'Daily Auto Snapshot', schedule: '0 3 * * *', target: 'internal:auto_snapshot', enabled: true, last_run: nowMs - 3600000 * 6, last_status: 'ok' }
];

const ALERT_EVENT_CHOICES = ['*', 'auto_rollback', 'cron_failure', 'worker_oom', 'ai_budget_exceeded'];

class FpOpsCenter extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [opsSheet];
    this._token = '';
    this._scope = 'admin';
    this._projects = [];
    this._tab = 'backups'; // backups | cron | alerts | realtime
    this._backups = [];
    this._backupSchedule = { enabled: false, interval_hours: 24, max_backups: 7, last_backup_at_ms: null };
    this._loadingBackups = false;
    this._creatingBackup = false;
    this._restoreTarget = '';
    this._backupErr = '';
    this._crons = [];
    this._loadingCrons = false;
    this._cronErr = '';
    this._webhooks = [];
    this._loadingWebhooks = false;
    this._rtChannels = [];
    this._rtSubChannel = 'default';
    this._rtEventSource = null;
    this._rtConnected = false;
    this._rtEvents = [];
    this._rtPubChannel = 'default';
    this._rtPubEvent = 'message';
    this._rtPubData = '{"hello":"world"}';
  }

  set token(val) {
    this._token = String(val || '');
    if (this.isConnected) {
      if (this._tab === 'backups' && this._isAdmin()) this.loadBackups();
      if (this._tab === 'cron') this.loadCrons();
      if (this._tab === 'alerts' && this._isAdmin()) this.loadWebhooks();
      if (this._tab === 'realtime') this.loadRealtimeChannels();
    }
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (!this._isAdmin() && (this._tab === 'backups' || this._tab === 'alerts')) this._tab = 'cron';
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

  _rawToken() {
    return this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
  }

  _authHeaders(json = false) {
    const tok = this._rawToken();
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
      this.loadWebhooks();
    }
    this.loadCrons();
  }

  disconnectedCallback() {
    this._stopRealtimeSse(false);
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
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
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
      const [bRes, sRes] = await Promise.all([
        fetch('/_api/v1/backups', { headers: this._authHeaders() }),
        fetch('/_api/v1/backups/schedule', { headers: this._authHeaders() })
      ]);
      if (!bRes.ok) throw new Error(`HTTP ${bRes.status}`);
      this._backups = (await bRes.json()) || [];
      if (sRes.ok) {
        this._backupSchedule = await sRes.json();
      }
    } catch (err) {
      this._backupErr = err.message || 'Failed to load backups';
    } finally {
      this._loadingBackups = false;
      this.render();
    }
  }

  async saveBackupSchedule(payload) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/backups/schedule', {
        method: 'PATCH',
        headers: this._authHeaders(true),
        body: JSON.stringify(payload)
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      this._backupSchedule = await res.json();
      emitToast(this, t('ops.autoBackupSaved'), 'success');
      this.render();
    } catch (err) {
      emitToast(this, err.message, 'error');
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
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      emitToast(this, t('ops.backupCreated'), 'success');
      await this.loadBackups();
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._creatingBackup = false;
      this.render();
    }
  }

  async downloadBackup(name) {
    try {
      const res = await fetch('/_api/v1/backups/' + encodeURIComponent(name) + '/download', {
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `${name}.tar.gz`;
      document.body.appendChild(a);
      a.click();
      a.remove();
      URL.revokeObjectURL(url);
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async restoreBackup(name, restart) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/backups/' + encodeURIComponent(name) + '/restore', {
        method: 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify({ restart: Boolean(restart) })
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      const data = await res.json();
      emitToast(this, t('ops.restoreDone', { count: (data.restored_files || []).length }), 'success');
      await this.loadBackups();
    } catch (err) {
      emitToast(this, err.message, 'error');
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

  async loadWebhooks() {
    if (!this._isAdmin()) return;
    this._loadingWebhooks = true;
    this.render();
    try {
      const res = await fetch('/_api/v1/alerts/webhooks', { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._webhooks = (await res.json()) || [];
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._loadingWebhooks = false;
      this.render();
    }
  }

  async saveWebhook(payload, id) {
    const { t } = getI18n();
    try {
      const url = id
        ? '/_api/v1/alerts/webhooks/' + encodeURIComponent(id)
        : '/_api/v1/alerts/webhooks';
      const res = await fetch(url, {
        method: id ? 'PATCH' : 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify(payload)
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      emitToast(this, t('alerts.saved'), 'success');
      await this.loadWebhooks();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async testWebhook(id) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/alerts/webhooks/' + encodeURIComponent(id) + '/test', {
        method: 'POST',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const out = await res.json();
      emitToast(
        this,
        out.ok ? t('alerts.testOk', { status: out.status_code }) : (out.error || t('alerts.testFailed')),
        out.ok ? 'success' : 'error'
      );
      await this.loadWebhooks();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async deleteWebhook(id) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/alerts/webhooks/' + encodeURIComponent(id), {
        method: 'DELETE',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      emitToast(this, t('alerts.deleted'), 'success');
      await this.loadWebhooks();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async loadRealtimeChannels() {
    try {
      const res = await fetch('/_api/v1/realtime/channels', { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._rtChannels = (await res.json()) || [];
      this.render();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  _startRealtimeSse() {
    this._stopRealtimeSse(false);
    const ch = (this._rtSubChannel || 'default').trim();
    if (!ch) return;
    const url = '/_realtime/' + encodeURIComponent(ch) + '/sse?token=' + encodeURIComponent(this._rawToken());
    const es = new EventSource(url);
    this._rtEventSource = es;
    this._rtConnected = true;
    const onMsg = (ev) => {
      this._rtEvents.unshift({
        type: ev.type || 'message',
        data: ev.data,
        ts: Date.now()
      });
      if (this._rtEvents.length > 50) this._rtEvents.length = 50;
      this.render();
    };
    es.onmessage = onMsg;
    es.addEventListener(this._rtPubEvent || 'message', onMsg);
    es.onerror = () => {
      this._rtConnected = false;
      this.render();
    };
    this.render();
  }

  _stopRealtimeSse(doRender = true) {
    if (this._rtEventSource) {
      this._rtEventSource.close();
      this._rtEventSource = null;
    }
    this._rtConnected = false;
    if (doRender && this.isConnected) this.render();
  }

  async publishRealtime() {
    const { t } = getI18n();
    const ch = (this._rtPubChannel || 'default').trim();
    if (!ch) return;
    try {
      const parsedData = JSON.parse(this._rtPubData || '{}');
      const res = await fetch('/_realtime/' + encodeURIComponent(ch) + '/publish', {
        method: 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify({
          event: this._rtPubEvent.trim() || 'message',
          data: parsedData
        })
      });
      if (!res.ok) throw new Error((await res.text()) || `HTTP ${res.status}`);
      emitToast(this, t('realtime.published'), 'success');
      await this.loadRealtimeChannels();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  _openWebhookModal(existing) {
    const dlg = this.shadowRoot.getElementById('webhook-dialog');
    if (!dlg) return;
    const { t } = getI18n();
    const form = this.shadowRoot.getElementById('webhook-form');
    this.shadowRoot.getElementById('wh-dlg-title').textContent = existing
      ? `${t('common.edit')} · ${existing.name}`
      : t('alerts.newWebhook');
    this.shadowRoot.getElementById('wh-name').value = existing ? existing.name : '';
    this.shadowRoot.getElementById('wh-url').value = existing ? existing.url : '';
    this.shadowRoot.getElementById('wh-secret').value = '';
    this.shadowRoot.getElementById('wh-secret').placeholder = existing && existing.has_secret ? t('ai.keepKeyHint') : 'optional-hmac-secret';
    this.shadowRoot.getElementById('wh-enabled').checked = existing ? Boolean(existing.enabled) : true;
    const evSet = new Set(existing ? (existing.events || ['*']) : ['*']);
    this.shadowRoot.querySelectorAll('[data-wh-ev]').forEach((cb) => {
      cb.checked = evSet.has(cb.getAttribute('data-wh-ev'));
    });
    form.dataset.editing = existing ? existing.id : '';
    dlg.showModal();
  }

  render() {
    const { t, formatTime, formatBytes } = getI18n();
    const isAdmin = this._isAdmin();
    const scopePid = this._scopeProjectId();
    const canWrite = this._scope !== 'readonly';

    const tabsHtml = `
      <div class="tabs">
        ${isAdmin ? `<button type="button" class="tab-btn ${this._tab === 'backups' ? 'active' : ''}" data-ops-tab="backups">${esc(t('ops.tabBackups'))}</button>` : ''}
        <button type="button" class="tab-btn ${this._tab === 'cron' ? 'active' : ''}" data-ops-tab="cron">${esc(t('ops.tabCron'))}</button>
        ${isAdmin ? `<button type="button" class="tab-btn ${this._tab === 'alerts' ? 'active' : ''}" data-ops-tab="alerts">${esc(t('ops.tabAlerts'))}</button>` : ''}
        <button type="button" class="tab-btn ${this._tab === 'realtime' ? 'active' : ''}" data-ops-tab="realtime">${esc(t('ops.tabRealtime'))}</button>
      </div>
    `;

    let bodyHtml = '';
    if (this._tab === 'backups' && isAdmin) {
      const rows = this._backups.map((b) => `
        <tr>
          <td class="cell-primary">
            <div class="mono text-strong break-all">${esc(b.name)}</div>
            <details style="margin-top:4px;">
              <summary class="muted text-xs" style="cursor:pointer;">${esc(t('ops.filesCount', { count: b.file_count }))}</summary>
              <pre class="code-block" style="margin-top:6px;">${esc((b.files || []).join('\n'))}</pre>
            </details>
          </td>
          <td data-label="${esc(t('ops.size'))}" class="mono text-xs">${esc(formatBytes(b.size_bytes))}</td>
          <td data-label="${esc(t('common.createdAt'))}" class="muted text-xs">${esc(formatTime(b.created_at_ms))}</td>
          <td class="cell-actions-td">
            <div class="cell-actions">
              <button type="button" class="btn btn-sm" data-dl-backup="${esc(b.name)}">${esc(t('ops.download'))}</button>
              <button type="button" class="btn btn-sm" data-restore-backup="${esc(b.name)}">${esc(t('ops.restore'))}</button>
              <button type="button" class="btn btn-danger btn-sm" data-del-backup="${esc(b.name)}">${esc(t('common.delete'))}</button>
            </div>
          </td>
        </tr>
      `).join('');

      const sched = this._backupSchedule || {};
      bodyHtml = `
        <div class="stack gap-md">
          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">${esc(t('ops.autoBackupTitle'))}</h3>
              ${sched.last_backup_at_ms ? `<span class="text-xs muted">${esc(t('ops.lastRun'))}: ${esc(formatTime(sched.last_backup_at_ms))}</span>` : ''}
            </div>
            <div class="panel-body">
              <form id="auto-backup-form" class="row">
                <label class="checkbox-row">
                  <input type="checkbox" id="ab-enabled" ${sched.enabled ? 'checked' : ''}>
                  <span>${esc(t('ops.enabled'))}</span>
                </label>
                <label class="row text-xs muted">${esc(t('ops.autoBackupInterval'))}
                  <input type="number" id="ab-interval" min="1" max="720" value="${esc(sched.interval_hours || 24)}" style="width:86px;">
                </label>
                <label class="row text-xs muted">${esc(t('ops.autoBackupKeep'))}
                  <input type="number" id="ab-keep" min="1" max="100" value="${esc(sched.max_backups || 7)}" style="width:80px;">
                </label>
                <button type="submit" class="btn btn-primary btn-sm">${esc(t('common.save'))}</button>
              </form>
            </div>
          </div>

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
                ? `<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>${esc(t('ops.snapshotName'))}</th><th>${esc(t('ops.size'))}</th><th>${esc(t('common.createdAt'))}</th><th>${esc(t('common.actions'))}</th></tr></thead><tbody>${rows}</tbody></table></div>`
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
    } else if (this._tab === 'alerts' && isAdmin) {
      const whRows = this._webhooks.map((w) => `
        <tr>
          <td class="cell-primary">
            <div class="row-between">
              <span class="text-strong">${esc(w.name)}</span>
              <span class="badge ${w.enabled ? 'badge-success' : 'badge-neutral'}">${esc(w.enabled ? t('ops.enabled') : t('ops.disabled'))}</span>
            </div>
            <div class="mono text-xs muted break-all">${esc(w.url)}</div>
          </td>
          <td data-label="${esc(t('alerts.events'))}"><div class="row gap-xs">${(w.events || ['*']).map((ev) => `<span class="badge badge-neutral mono">${esc(ev)}</span>`).join('')}</div></td>
          <td data-label="${esc(t('alerts.secret'))}">${w.has_secret ? `<span class="badge badge-accent">HMAC</span>` : `<span class="badge badge-neutral">-</span>`}</td>
          <td data-label="${esc(t('ops.lastRun'))}" class="text-xs">
            ${w.last_status ? `<span class="badge ${w.last_status < 300 ? 'badge-success' : 'badge-danger'} mono">${esc(w.last_status)}</span>` : ''}
            ${w.last_error ? `<div class="muted break-all">${esc(w.last_error)}</div>` : ''}
            ${w.last_triggered_at ? `<div class="muted">${esc(formatTime(w.last_triggered_at))}</div>` : (!w.last_status && !w.last_error ? '-' : '')}
          </td>
          <td class="cell-actions-td"><div class="cell-actions">
            <button type="button" class="btn btn-sm" data-test-wh="${esc(w.id)}">${esc(t('alerts.testBtn'))}</button>
            <button type="button" class="btn btn-sm" data-edit-wh="${esc(w.id)}">${esc(t('common.edit'))}</button>
            <button type="button" class="btn btn-danger btn-sm" data-del-wh="${esc(w.id)}">${esc(t('common.delete'))}</button>
          </div></td>
        </tr>
      `).join('');

      bodyHtml = `
        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">
              <span>${esc(t('alerts.title'))}</span>
              <span class="badge badge-neutral">${this._webhooks.length}</span>
            </h3>
            <div class="row gap-xs">
              <button type="button" class="btn btn-sm" id="reload-wh-btn">${esc(t('common.refresh'))}</button>
              <button type="button" class="btn btn-primary btn-sm" id="new-wh-btn">${esc(t('alerts.newWebhook'))}</button>
            </div>
          </div>
          ${this._loadingWebhooks && !this._webhooks.length
            ? `<div class="empty-state">${esc(t('common.loading'))}</div>`
            : (whRows
              ? `<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr>
                  <th>${esc(t('common.name'))}</th>
                  <th>${esc(t('alerts.events'))}</th>
                  <th>${esc(t('alerts.secret'))}</th>
                  <th>${esc(t('ops.lastRun'))}</th>
                  <th>${esc(t('common.actions'))}</th>
                </tr></thead><tbody>${whRows}</tbody></table></div>`
              : `<div class="empty-state">${esc(t('alerts.empty'))}</div>`)
          }
        </div>
      `;
    } else if (this._tab === 'realtime') {
      const chRows = this._rtChannels.map((c) => `
        <tr>
          <td class="cell-primary mono text-strong">${esc(c.channel)}</td>
          <td data-label="${esc(t('realtime.subscribers'))}" class="mono text-xs">${esc(c.subscribers)}</td>
          <td data-label="${esc(t('realtime.publishedCount'))}" class="mono text-xs">${esc(c.published_total)}</td>
          <td data-label="${esc(t('ops.lastRun'))}" class="muted text-xs">${c.last_published_at_ms ? esc(formatTime(c.last_published_at_ms)) : '-'}</td>
          <td class="cell-actions-td"><div class="cell-actions">
            <button type="button" class="btn btn-sm" data-pick-rt="${esc(c.channel)}">${esc(t('realtime.inspectBtn'))}</button>
          </div></td>
        </tr>
      `).join('');

      bodyHtml = `
        <div class="stack gap-md">
          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">${esc(t('realtime.channelsTitle'))}</h3>
              <button type="button" class="btn btn-sm" id="rt-refresh-btn">${esc(t('common.refresh'))}</button>
            </div>
            ${chRows
              ? `<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>${esc(t('realtime.channel'))}</th><th>${esc(t('realtime.subscribers'))}</th><th>${esc(t('realtime.publishedCount'))}</th><th>${esc(t('ops.lastRun'))}</th><th>${esc(t('common.actions'))}</th></tr></thead><tbody>${chRows}</tbody></table></div>`
              : `<div class="empty-state">${esc(t('realtime.noChannels'))}</div>`
            }
          </div>

          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">${esc(t('realtime.streamTitle'))}</h3>
              <span class="badge ${this._rtConnected ? 'badge-success' : 'badge-neutral'}">${esc(this._rtConnected ? t('perf.live') : t('perf.paused'))}</span>
            </div>
            <div class="panel-body stack gap-md">
              <div class="row">
                <input type="text" id="rt-sub-ch" value="${esc(this._rtSubChannel)}" placeholder="default" style="max-width:260px;">
                <button type="button" class="btn btn-primary btn-sm" id="rt-sub-toggle">${esc(this._rtConnected ? t('logs.disconnect') : t('logs.connect'))}</button>
                <button type="button" class="btn btn-sm" id="rt-clear-btn">${esc(t('logs.clear'))}</button>
              </div>
              <pre class="code-block">${this._rtEvents.length ? esc(this._rtEvents.map((ev) => `[${formatTime(ev.ts)}] (${ev.type}) ${ev.data}`).join('\n')) : esc(t('realtime.noEvents'))}</pre>
            </div>
          </div>

          ${canWrite ? `
            <div class="panel">
              <div class="panel-header"><h3 class="panel-title">${esc(t('realtime.publishTitle'))}</h3></div>
              <div class="panel-body">
                <form id="rt-pub-form" class="stack gap-sm">
                  <div class="form-grid">
                    <div class="field"><label>${esc(t('realtime.channel'))}</label><input type="text" id="rt-pub-ch" value="${esc(this._rtPubChannel)}" required></div>
                    <div class="field"><label>${esc(t('realtime.eventName'))}</label><input type="text" id="rt-pub-ev" value="${esc(this._rtPubEvent)}" required></div>
                  </div>
                  <div class="field"><label>JSON Data</label><textarea id="rt-pub-data" class="mono">${esc(this._rtPubData)}</textarea></div>
                  <div class="row"><button type="submit" class="btn btn-primary btn-sm">${esc(t('realtime.publishBtn'))}</button></div>
                </form>
              </div>
            </div>
          ` : ''}
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
                <table class="data-table stack-mobile">
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
                        <td class="cell-primary text-strong">${esc(c.name)}</td>
                        <td data-label="Cron" class="mono text-xs"><span class="badge badge-neutral">${esc(c.schedule)}</span></td>
                        <td data-label="${esc(t('ops.target'))}" class="mono text-xs muted break-all">${esc(c.target)}</td>
                        <td data-label="${esc(t('ops.lastRun'))}" class="text-xs"><span class="badge badge-success">${esc(c.last_status)}</span> <span class="muted">${esc(formatTime(c.last_run))}</span></td>
                        <td class="cell-actions-td"><div class="cell-actions"><button type="button" class="btn btn-sm" data-run-sys="${esc(c.id)}">${esc(t('ops.runNow'))}</button></div></td>
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
                ${canWrite ? `<button type="button" class="btn btn-primary btn-sm" id="new-cron-btn">${esc(t('ops.newCron'))}</button>` : ''}
              </div>
            </div>
            ${this._cronErr ? `<div class="panel-body"><div class="badge badge-warn">${esc(this._cronErr)}</div></div>` : ''}
            ${this._loadingCrons ? `<div class="empty-state">${esc(t('common.loading'))}</div>` : (visibleProjectTasks.length ? `
              <div class="table-wrap">
                <table class="data-table stack-mobile">
                  <thead>
                    <tr>
                      <th>${esc(t('common.name'))}</th>
                      <th>${esc(t('common.project'))}</th>
                      <th>Cron</th>
                      <th>${esc(t('ops.target'))}</th>
                      <th>${esc(t('ops.lastRun'))}</th>
                      ${canWrite ? `<th>${esc(t('common.actions'))}</th>` : ''}
                    </tr>
                  </thead>
                  <tbody>
                    ${visibleProjectTasks.map((c) => `
                      <tr>
                        <td class="cell-primary">
                          <div class="row" style="justify-content:space-between;gap:8px;">
                            <span class="text-strong">${esc(c.name)}</span>
                            <span class="badge ${c.enabled ? 'badge-success' : 'badge-neutral'}">${esc(c.enabled ? t('ops.enabled') : t('ops.disabled'))}</span>
                          </div>
                        </td>
                        <td data-label="${esc(t('common.project'))}" class="text-xs">${c.project_id ? `<span class="badge badge-accent">${esc(this._projectName(c.project_id))}</span>` : `<span class="muted">${esc(t('common.allProjects'))}</span>`}</td>
                        <td data-label="Cron" class="mono text-xs"><span class="badge badge-neutral">${esc(c.schedule)}</span></td>
                        <td data-label="${esc(t('ops.target'))}" class="mono text-xs muted break-all">${esc(c.target)}</td>
                        <td data-label="${esc(t('ops.lastRun'))}" class="text-xs muted">
                          ${c.last_status ? `<span class="badge ${c.last_status === 'ok' ? 'badge-success' : 'badge-danger'}">${esc(c.last_status)}</span>` : ''}
                          ${c.last_run_at ? `<span>${esc(formatTime(c.last_run_at))}</span>` : (!c.last_status ? '-' : '')}
                        </td>
                        ${canWrite ? `
                          <td class="cell-actions-td">
                            <div class="cell-actions">
                              <button type="button" class="btn btn-sm" data-run-cron="${esc(c.id)}">${esc(t('ops.runNow'))}</button>
                              <button type="button" class="btn btn-sm" data-toggle-cron="${esc(c.id)}">${esc(c.enabled ? t('perf.pause') : t('perf.resume'))}</button>
                              <button type="button" class="btn btn-danger btn-sm" data-del-cron="${esc(c.id)}">${esc(t('common.delete'))}</button>
                            </div>
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

      <dialog id="restore-backup-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('ops.restore'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dlg="restore-backup-dialog">✕</button>
        </div>
        <form id="restore-backup-form">
          <div class="dialog-body stack gap-md">
            <p id="restore-backup-msg" style="margin:0;"></p>
            <label class="checkbox-row">
              <input type="checkbox" id="restore-restart-cb" checked>
              <span>${esc(t('ops.restoreRestart'))}</span>
            </label>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dlg="restore-backup-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-danger">${esc(t('ops.restore'))}</button>
          </div>
        </form>
      </dialog>

      <dialog id="webhook-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title" id="wh-dlg-title">${esc(t('alerts.newWebhook'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dlg="webhook-dialog">✕</button>
        </div>
        <form id="webhook-form">
          <div class="dialog-body stack gap-md">
            <div class="field"><label>${esc(t('common.name'))}</label><input type="text" id="wh-name" required placeholder="ops-slack"></div>
            <div class="field"><label>URL</label><input type="url" id="wh-url" required placeholder="https://hooks.example.com/ferropod"></div>
            <div class="field"><label>${esc(t('alerts.secret'))}</label><input type="password" id="wh-secret" placeholder="optional-hmac-secret"></div>
            <div class="field">
              <label>${esc(t('alerts.events'))}</label>
              <div class="row">
                ${ALERT_EVENT_CHOICES.map((ev) => `<label class="checkbox-row"><input type="checkbox" data-wh-ev="${esc(ev)}" ${ev === '*' ? 'checked' : ''}><span class="mono">${esc(ev)}</span></label>`).join('')}
              </div>
            </div>
            <label class="checkbox-row"><input type="checkbox" id="wh-enabled" checked><span>${esc(t('ops.enabled'))}</span></label>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dlg="webhook-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.save'))}</button>
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
              <select id="cr-project" required ${scopePid ? 'disabled' : ''}>
                ${scopePid
                  ? `<option value="${esc(scopePid)}" selected>${esc(this._projectName(scopePid))}</option>`
                  : `<option value="" disabled selected>${esc(t('tokens.selectProject'))}</option>` +
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
              <input type="text" id="cr-target" class="mono" required value="/_internal/cron">
            </div>
            <details style="border:1px solid var(--border);border-radius:var(--radius-sm);padding:8px 12px;background:var(--surface-subtle);">
              <summary style="cursor:pointer;font-size:12.5px;font-weight:600;color:var(--text-secondary);">${esc(t('common.advancedOptions'))}</summary>
              <div class="stack gap-sm" style="margin-top:10px;">
                <div class="field">
                  <label for="cr-desc">${esc(t('common.description'))}</label>
                  <input type="text" id="cr-desc" placeholder="${esc(t('common.descriptionPlaceholder'))}">
                </div>
                <div class="row">
                  <div class="field" style="flex:1 1 130px;">
                    <label for="cr-tz">${esc(t('ops.timezone'))}</label>
                    <input type="text" id="cr-tz" value="UTC" placeholder="UTC">
                  </div>
                  <div class="field" style="flex:1 1 130px;">
                    <label for="cr-timeout">${esc(t('ops.timeoutMs'))}</label>
                    <input type="text" id="cr-timeout" placeholder="10000">
                  </div>
                  <div class="field" style="flex:1 1 110px;">
                    <label for="cr-retries">${esc(t('ops.maxRetries'))}</label>
                    <input type="text" id="cr-retries" value="0" placeholder="0">
                  </div>
                </div>
                <label style="display:flex;align-items:center;gap:8px;font-size:12.5px;cursor:pointer;">
                  <input type="checkbox" id="cr-enabled" checked> ${esc(t('ops.enableImmediately'))}
                </label>
              </div>
            </details>
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
        if (this._tab === 'backups' && this._isAdmin()) this.loadBackups();
        else if (this._tab === 'alerts' && this._isAdmin()) this.loadWebhooks();
        else if (this._tab === 'realtime') this.loadRealtimeChannels();
        else this.render();
      });
    });

    root.querySelectorAll('[data-close-dlg]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const d = root.getElementById(btn.getAttribute('data-close-dlg'));
        if (d && d.open) d.close();
      });
    });

    const abForm = root.getElementById('auto-backup-form');
    if (abForm) {
      abForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this.saveBackupSchedule({
          enabled: root.getElementById('ab-enabled').checked,
          interval_hours: Number(root.getElementById('ab-interval').value || 24),
          max_backups: Number(root.getElementById('ab-keep').value || 7)
        });
      });
    }

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

    root.querySelectorAll('[data-dl-backup]').forEach((btn) => {
      btn.addEventListener('click', () => this.downloadBackup(btn.getAttribute('data-dl-backup')));
    });

    root.querySelectorAll('[data-restore-backup]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const name = btn.getAttribute('data-restore-backup');
        this._restoreTarget = name;
        const msg = root.getElementById('restore-backup-msg');
        if (msg) msg.textContent = t('ops.restoreConfirm', { name });
        root.getElementById('restore-backup-dialog').showModal();
      });
    });

    const restoreForm = root.getElementById('restore-backup-form');
    if (restoreForm) {
      restoreForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        const name = this._restoreTarget;
        const restart = Boolean(root.getElementById('restore-restart-cb').checked);
        root.getElementById('restore-backup-dialog').close();
        if (name) this.restoreBackup(name, restart);
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

    const reloadWh = root.getElementById('reload-wh-btn');
    if (reloadWh) reloadWh.addEventListener('click', () => this.loadWebhooks());
    const newWh = root.getElementById('new-wh-btn');
    if (newWh) newWh.addEventListener('click', () => this._openWebhookModal(null));

    root.querySelectorAll('[data-test-wh]').forEach((btn) => {
      btn.addEventListener('click', () => this.testWebhook(btn.getAttribute('data-test-wh')));
    });
    root.querySelectorAll('[data-edit-wh]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const w = this._webhooks.find((x) => x.id === btn.getAttribute('data-edit-wh'));
        if (w) this._openWebhookModal(w);
      });
    });
    root.querySelectorAll('[data-del-wh]').forEach((btn) => {
      btn.addEventListener('click', () => this.deleteWebhook(btn.getAttribute('data-del-wh')));
    });

    const whForm = root.getElementById('webhook-form');
    if (whForm) {
      whForm.addEventListener('submit', async (ev) => {
        ev.preventDefault();
        const id = whForm.dataset.editing || '';
        const events = [];
        root.querySelectorAll('[data-wh-ev]').forEach((cb) => {
          if (cb.checked) events.push(cb.getAttribute('data-wh-ev'));
        });
        const payload = {
          name: root.getElementById('wh-name').value.trim(),
          url: root.getElementById('wh-url').value.trim(),
          events: events.length ? events : ['*'],
          enabled: root.getElementById('wh-enabled').checked
        };
        const sec = root.getElementById('wh-secret').value.trim();
        if (sec) payload.secret = sec;
        root.getElementById('webhook-dialog').close();
        await this.saveWebhook(payload, id || null);
      });
    }

    const rtRefresh = root.getElementById('rt-refresh-btn');
    if (rtRefresh) rtRefresh.addEventListener('click', () => this.loadRealtimeChannels());

    root.querySelectorAll('[data-pick-rt]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const ch = btn.getAttribute('data-pick-rt');
        this._rtSubChannel = ch;
        this._rtPubChannel = ch;
        this._startRealtimeSse();
      });
    });

    const rtSubToggle = root.getElementById('rt-sub-toggle');
    if (rtSubToggle) {
      rtSubToggle.addEventListener('click', () => {
        this._rtSubChannel = root.getElementById('rt-sub-ch').value.trim() || 'default';
        if (this._rtConnected) this._stopRealtimeSse(true);
        else this._startRealtimeSse();
      });
    }
    const rtClear = root.getElementById('rt-clear-btn');
    if (rtClear) {
      rtClear.addEventListener('click', () => {
        this._rtEvents = [];
        this.render();
      });
    }
    const rtPubForm = root.getElementById('rt-pub-form');
    if (rtPubForm) {
      rtPubForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._rtPubChannel = root.getElementById('rt-pub-ch').value.trim() || 'default';
        this._rtPubEvent = root.getElementById('rt-pub-ev').value.trim() || 'message';
        this._rtPubData = root.getElementById('rt-pub-data').value;
        this.publishRealtime();
      });
    }

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
        if (!pid) {
          emitToast(this, t('tokens.selectProject'), 'error');
          return;
        }
        const timeoutRaw = (root.getElementById('cr-timeout')?.value || '').trim();
        const retriesRaw = (root.getElementById('cr-retries')?.value || '').trim();
        const descRaw = (root.getElementById('cr-desc')?.value || '').trim();
        const tzRaw = (root.getElementById('cr-tz')?.value || '').trim();
        const enabledEl = root.getElementById('cr-enabled');
        const payload = {
          name: root.getElementById('cr-name').value.trim(),
          project_id: pid,
          schedule: root.getElementById('cr-schedule').value.trim(),
          target: root.getElementById('cr-target').value.trim(),
          description: descRaw || null,
          timezone: tzRaw || 'UTC',
          timeout_ms: timeoutRaw ? Number(timeoutRaw) : null,
          max_retries: retriesRaw ? Number(retriesRaw) : 0,
          enabled: enabledEl ? Boolean(enabledEl.checked) : true
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
