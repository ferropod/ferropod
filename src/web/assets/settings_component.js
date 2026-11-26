// Native HTML Web Component: <fp-platform-settings>
// Platform Settings: Domains & HTTPS/ACME, Runtime & Retention, AI Gateway / MCP / Alert Webhooks, System Environment.

const settingsSheet = new CSSStyleSheet();
settingsSheet.replaceSync(`
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
    margin-bottom: 16px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs::-webkit-scrollbar { display: none; }
  .tab-btn {
    padding: 9px 14px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--muted);
    font-size: 13.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    display: inline-flex;
    align-items: center;
    gap: 6px;
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

  .form-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: 14px;
  }
  .info-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(175px, 1fr));
    gap: 12px;
  }
  .endpoint-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: 12px;
  }
  .info-card {
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 10px 12px;
    min-width: 0;
    overflow: hidden;
  }
  .endpoint-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 6px;
    min-width: 0;
  }
  .endpoint-url {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    word-break: break-all;
    line-height: 1.4;
  }
  .endpoint-row .btn {
    flex-shrink: 0;
  }
  .info-label { font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .info-val { font-size: 14px; font-weight: 600; margin-top: 3px; word-break: break-all; }

  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12.5px; font-weight: 600; color: var(--text-secondary); }
  .field input[type="text"],
  .field input[type="number"],
  .field input[type="password"],
  .field input[type="url"],
  .field input[type="email"],
  .field input:not([type]),
  .field select,
  .field textarea,
  select {
    width: 100%;
    min-height: 38px;
    padding: 7px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background-color: var(--surface);
    color: var(--text);
    font-size: 13px;
    line-height: 1.4;
    outline: none;
    transition: border-color 0.15s ease, box-shadow 0.15s ease, background-color 0.15s ease;
  }
  select,
  .field select {
    appearance: none;
    -webkit-appearance: none;
    -moz-appearance: none;
    padding-right: 32px;
    background-image: var(--select-arrow);
    background-repeat: no-repeat;
    background-position: right 10px center;
    background-size: 14px 14px;
    cursor: pointer;
    text-overflow: ellipsis;
  }
  select:hover:not(:disabled),
  .field select:hover:not(:disabled) {
    border-color: var(--border-strong);
  }
  select option,
  select optgroup {
    background-color: var(--surface);
    color: var(--text);
  }
  .field input:focus,
  .field select:focus,
  .field textarea:focus,
  select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .field input:disabled,
  .field select:disabled,
  .field textarea:disabled,
  select:disabled {
    opacity: 0.6;
    cursor: not-allowed;
    background-color: var(--surface-subtle);
  }
  .field textarea { min-height: 76px; resize: vertical; }
  .field-hint { font-size: 12px; color: var(--muted); }
  .input-inline { display: flex; gap: 8px; align-items: stretch; flex-wrap: wrap; }
  .input-inline > input { flex: 1 1 200px; min-width: 0; }

  .checkbox-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    cursor: pointer;
    font-size: 13.5px;
    color: var(--text);
  }
  .checkbox-row input[type="checkbox"] {
    margin-top: 3px;
    accent-color: var(--accent);
    width: 15px;
    height: 15px;
    flex-shrink: 0;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 36px;
    padding: 0 14px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s ease, border-color 0.15s ease, color 0.15s ease;
  }
  .btn:hover:not(:disabled) { background: var(--surface-subtle); border-color: var(--border-strong); }
  .btn:disabled { opacity: 0.55; cursor: not-allowed; }
  .btn-primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }
  .btn-primary:hover:not(:disabled) { background: var(--accent-hover); border-color: var(--accent-hover); }
  .btn-danger {
    color: var(--danger);
    border-color: var(--border);
    background: var(--surface);
  }
  .btn-danger:hover:not(:disabled) { background: var(--danger-soft); border-color: var(--danger); }
  .btn-ghost { border-color: transparent; background: transparent; color: var(--muted); }
  .btn-ghost:hover:not(:disabled) { background: var(--surface-subtle); color: var(--text); }
  .btn-sm { height: 30px; padding: 0 10px; font-size: 12px; }

  .badge {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 600;
    border: 1px solid var(--border);
    background: var(--surface-subtle);
    color: var(--text-secondary);
  }
  .badge-success { background: var(--success-soft); color: var(--success); border-color: transparent; }
  .badge-warn { background: var(--warn-soft); color: var(--warn); border-color: transparent; }
  .badge-danger { background: var(--danger-soft); color: var(--danger); border-color: transparent; }
  .badge-info { background: var(--info-soft); color: var(--info); border-color: transparent; }
  .badge-accent { background: var(--accent-soft); color: var(--accent); border-color: transparent; }

  .banner {
    padding: 12px 14px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    font-size: 13px;
    margin-bottom: 14px;
  }
  .banner-info { background: var(--info-soft); color: var(--info); border-color: color-mix(in srgb, var(--info) 30%, var(--border)); }
  .banner-warn { background: var(--warn-soft); color: var(--warn); border-color: color-mix(in srgb, var(--warn) 35%, var(--border)); }
  .banner-danger { background: var(--danger-soft); color: var(--danger); border-color: color-mix(in srgb, var(--danger) 35%, var(--border)); }
  .banner-success { background: var(--success-soft); color: var(--success); border-color: color-mix(in srgb, var(--success) 35%, var(--border)); }

  .chips { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
  .code-block {
    margin: 0;
    padding: 10px 12px;
    border-radius: var(--radius-sm);
    background: var(--bg);
    border: 1px solid var(--border);
    font-size: 12px;
    overflow-x: auto;
    white-space: pre-wrap;
    word-break: break-all;
  }

  .table-wrap { width: 100%; overflow-x: auto; }
  table.data-table { width: 100%; border-collapse: collapse; font-size: 13px; }
  table.data-table th,
  table.data-table td {
    text-align: left;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
    vertical-align: middle;
  }
  table.data-table th {
    font-size: 11.5px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
  }
  table.data-table tr:last-child td { border-bottom: none; }

  .empty-state {
    padding: 24px 16px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
  .action-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    margin-top: 14px;
  }

  dialog {
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 0;
    width: min(560px, 94vw);
    max-height: 90dvh;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.35);
  }
  dialog::backdrop {
    background: rgba(15, 23, 42, 0.55);
    backdrop-filter: blur(2px);
  }
  .dlg-head {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-weight: 600;
  }
  .dlg-body { padding: 16px; overflow-y: auto; max-height: 68dvh; }
  .dlg-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  @media (max-width: 639px) {
    .panel-header, .panel-body { padding: 12px 14px; }
    .info-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .field input[type="text"],
    .field input[type="number"],
    .field input[type="password"],
    .field input[type="url"],
    .field input[type="email"],
    .field input:not([type]),
    .field select,
    .field textarea,
    select {
      min-height: 42px;
      font-size: 16px;
    }
    .btn { min-height: 42px; height: auto; }
    .btn-sm { min-height: 36px; height: auto; }
    dialog {
      margin: auto 0 0 0;
      width: 100%;
      max-width: 100%;
      max-height: 92dvh;
      border-radius: 16px 16px 0 0;
      border-bottom: none;
      padding-bottom: env(safe-area-inset-bottom);
    }
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

function isLoopbackClientHost(hostname) {
  const h = String(hostname || '').trim().toLowerCase();
  return h === 'localhost' || h.endsWith('.localhost') || h === '127.0.0.1' || h === '::1' || h === '[::1]';
}

class FpPlatformSettings extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [settingsSheet];
    this._token = '';
    this._scope = '';
    this._tab = 'domains';
    this._bundle = null;
    this._draft = null;
    this._loading = false;
    this._saving = false;
    this._error = '';
    this._dnsPanelResult = null;
    this._dnsBaseResult = null;
    this._checkingDns = '';
    this._alertTestResult = null;
    this._testingAlert = false;
    this._aiProviders = [];
    this._aiUsage = null;
    this._loadingAi = false;
    this._onLangChange = () => this.render();
  }

  set token(v) {
    this._token = v || '';
  }

  set scope(v) {
    this._scope = v || '';
  }

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

  notifySettingsChanged() {
    this.dispatchEvent(new CustomEvent('fp-settings-changed', {
      bubbles: true,
      composed: true,
      detail: { bundle: this._bundle }
    }));
  }

  async copyText(text) {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(text);
        this.toast(this.t('common.copied'), 'success');
        return;
      }
    } catch (_) {}
    const ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand('copy');
      this.toast(this.t('common.copied'), 'success');
    } catch (_) {}
    document.body.removeChild(ta);
  }

  async api(path, opts = {}) {
    const headers = Object.assign({ Accept: 'application/json' }, opts.headers || {});
    if (this._token) headers.Authorization = 'Bearer ' + this._token;
    if (opts.json !== undefined) {
      headers['Content-Type'] = 'application/json';
      opts.body = JSON.stringify(opts.json);
    }
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
    this.loadAll();
  }

  disconnectedCallback() {
    window.removeEventListener('fp-lang-change', this._onLangChange);
  }

  initDraftFromBundle(bundle) {
    const eff = bundle.effective || {};
    this._draft = {
      instance_name: eff.instance_name || '',
      panel_domain: eff.panel_domain || '',
      restrict_panel_to_domain: Boolean(eff.restrict_panel_to_domain),
      confirm_lockout_risk: false,
      base_domain: eff.base_domain || '',
      public_scheme: eff.public_scheme || 'auto',
      trust_proxy_headers: Boolean(eff.trust_proxy_headers),
      acme_enabled: Boolean(eff.acme_enabled),
      acme_email: eff.acme_email || '',
      acme_staging: Boolean(eff.acme_staging),
      http_redirect_https: Boolean(eff.http_redirect_https),
      extra_tls_domains: Array.isArray(eff.extra_tls_domains) ? eff.extra_tls_domains.join('\n') : '',
      log_retention_days: eff.log_retention_days != null ? String(eff.log_retention_days) : '',
      queue_pending_retention_secs: eff.queue_pending_retention_secs != null ? String(eff.queue_pending_retention_secs) : '',
      queue_dlq_retention_secs: eff.queue_dlq_retention_secs != null ? String(eff.queue_dlq_retention_secs) : '',
      blob_gc_grace_secs: eff.blob_gc_grace_secs != null ? String(eff.blob_gc_grace_secs) : '',
      wasm_max_memory_mb: eff.wasm_max_memory_mb != null ? String(eff.wasm_max_memory_mb) : '',
      wasm_epoch_timeout_ms: eff.wasm_epoch_timeout_ms != null ? String(eff.wasm_epoch_timeout_ms) : '',
      wasm_allow_private_outbound: Boolean(eff.wasm_allow_private_outbound),
      auto_rollback_enabled: Boolean(eff.auto_rollback_enabled),
      auto_rollback_window_secs: eff.auto_rollback_window_secs != null ? String(eff.auto_rollback_window_secs) : '',
      auto_rollback_min_requests: eff.auto_rollback_min_requests != null ? String(eff.auto_rollback_min_requests) : '',
      auto_rollback_error_rate_pct: eff.auto_rollback_error_rate_pct != null ? String(eff.auto_rollback_error_rate_pct) : '',
      alert_webhook_url: eff.alert_webhook_url || '',
      alert_webhook_secret: '',
      clear_alert_webhook_secret: false,
      alert_on_auto_rollback: Boolean(eff.alert_on_auto_rollback),
      alert_on_cron_failure: Boolean(eff.alert_on_cron_failure),
      alert_on_worker_oom: Boolean(eff.alert_on_worker_oom)
    };
  }

  syncDraftFromDom() {
    if (!this._draft) return;
    const sr = this.shadowRoot;
    const textKeys = [
      'instance_name', 'panel_domain', 'base_domain', 'public_scheme', 'acme_email',
      'extra_tls_domains', 'log_retention_days', 'queue_pending_retention_secs',
      'queue_dlq_retention_secs', 'blob_gc_grace_secs', 'wasm_max_memory_mb',
      'wasm_epoch_timeout_ms', 'auto_rollback_window_secs', 'auto_rollback_min_requests',
      'auto_rollback_error_rate_pct', 'alert_webhook_url', 'alert_webhook_secret'
    ];
    for (const k of textKeys) {
      const el = sr.getElementById('f-' + k);
      if (el) this._draft[k] = el.value;
    }
    const boolKeys = [
      'restrict_panel_to_domain', 'confirm_lockout_risk', 'trust_proxy_headers',
      'acme_enabled', 'acme_staging', 'http_redirect_https',
      'wasm_allow_private_outbound', 'auto_rollback_enabled',
      'clear_alert_webhook_secret', 'alert_on_auto_rollback',
      'alert_on_cron_failure', 'alert_on_worker_oom'
    ];
    for (const k of boolKeys) {
      const el = sr.getElementById('f-' + k);
      if (el) this._draft[k] = Boolean(el.checked);
    }
  }

  async loadAll() {
    this._loading = true;
    this._error = '';
    this.render();
    try {
      const bundle = await this.api('/_api/v1/settings');
      this._bundle = bundle;
      this.initDraftFromBundle(bundle);
      await this.loadAiData();
    } catch (e) {
      this._error = e.message;
    } finally {
      this._loading = false;
      this.render();
    }
  }

  async loadAiData() {
    this._loadingAi = true;
    try {
      const [providers, usage] = await Promise.all([
        this.api('/_api/v1/ai/providers'),
        this.api('/_api/v1/ai/usage?limit=20')
      ]);
      this._aiProviders = Array.isArray(providers) ? providers : [];
      this._aiUsage = usage || null;
    } catch (_) {
      this._aiProviders = [];
      this._aiUsage = null;
    } finally {
      this._loadingAi = false;
    }
  }

  needsLockoutConfirm() {
    if (!this._draft) return false;
    if (!this._draft.restrict_panel_to_domain) return false;
    const targetDom = String(this._draft.panel_domain || '').trim().replace(/\.+$/, '').toLowerCase();
    if (!targetDom) return false;
    const curHost = String(window.location.hostname || '').trim().replace(/\.+$/, '').toLowerCase();
    if (!curHost || isLoopbackClientHost(curHost)) return false;
    return curHost !== targetDom;
  }

  parseOptionalInt(raw) {
    const t = String(raw || '').trim();
    if (!t) return null;
    const n = Number(t);
    if (!Number.isInteger(n)) throw new Error(this.t('settings.invalidInteger', { value: t }));
    return n;
  }

  async savePatch(patch) {
    this._saving = true;
    this.render();
    try {
      const bundle = await this.api('/_api/v1/settings', {
        method: 'PATCH',
        json: patch
      });
      this._bundle = bundle;
      this.initDraftFromBundle(bundle);
      this.toast(this.t('settings.saved'), 'success');
      this.notifySettingsChanged();
    } catch (e) {
      this.toast(e.message, 'error');
    } finally {
      this._saving = false;
      this.render();
    }
  }

  async saveDomainsTab() {
    this.syncDraftFromDom();
    const d = this._draft;
    const extraDomains = String(d.extra_tls_domains || '')
      .split(/[\n,]+/)
      .map((s) => s.trim())
      .filter(Boolean);
    const patch = {
      instance_name: d.instance_name.trim() || null,
      panel_domain: d.panel_domain.trim() || null,
      restrict_panel_to_domain: d.restrict_panel_to_domain,
      base_domain: d.base_domain.trim() || null,
      public_scheme: d.public_scheme === 'auto' ? null : d.public_scheme,
      trust_proxy_headers: d.trust_proxy_headers,
      acme_enabled: d.acme_enabled,
      acme_email: d.acme_email.trim() || null,
      acme_staging: d.acme_staging,
      http_redirect_https: d.http_redirect_https,
      extra_tls_domains: extraDomains.length ? extraDomains : null,
      confirm_lockout_risk: d.confirm_lockout_risk ? true : undefined
    };
    await this.savePatch(patch);
  }

  async saveRuntimeTab() {
    this.syncDraftFromDom();
    const d = this._draft;
    let patch;
    try {
      patch = {
        log_retention_days: this.parseOptionalInt(d.log_retention_days),
        queue_pending_retention_secs: this.parseOptionalInt(d.queue_pending_retention_secs),
        queue_dlq_retention_secs: this.parseOptionalInt(d.queue_dlq_retention_secs),
        blob_gc_grace_secs: this.parseOptionalInt(d.blob_gc_grace_secs),
        wasm_max_memory_mb: this.parseOptionalInt(d.wasm_max_memory_mb),
        wasm_epoch_timeout_ms: this.parseOptionalInt(d.wasm_epoch_timeout_ms),
        wasm_allow_private_outbound: d.wasm_allow_private_outbound,
        auto_rollback_enabled: d.auto_rollback_enabled,
        auto_rollback_window_secs: this.parseOptionalInt(d.auto_rollback_window_secs),
        auto_rollback_min_requests: this.parseOptionalInt(d.auto_rollback_min_requests),
        auto_rollback_error_rate_pct: this.parseOptionalInt(d.auto_rollback_error_rate_pct)
      };
    } catch (e) {
      this.toast(e.message, 'error');
      return;
    }
    await this.savePatch(patch);
  }

  async saveAlertsSection() {
    this.syncDraftFromDom();
    const d = this._draft;
    const patch = {
      alert_webhook_url: d.alert_webhook_url.trim() || null,
      alert_on_auto_rollback: d.alert_on_auto_rollback,
      alert_on_cron_failure: d.alert_on_cron_failure,
      alert_on_worker_oom: d.alert_on_worker_oom
    };
    if (d.clear_alert_webhook_secret) {
      patch.clear_alert_webhook_secret = true;
    } else if (d.alert_webhook_secret.trim()) {
      patch.alert_webhook_secret = d.alert_webhook_secret.trim();
    }
    await this.savePatch(patch);
  }

  async resetSection(section) {
    this._saving = true;
    this.render();
    try {
      const bundle = await this.api('/_api/v1/settings/reset', {
        method: 'POST',
        json: { section }
      });
      this._bundle = bundle;
      this.initDraftFromBundle(bundle);
      this.toast(this.t('settings.resetDone'), 'success');
      this.notifySettingsChanged();
    } catch (e) {
      this.toast(e.message, 'error');
    } finally {
      this._saving = false;
      this.render();
    }
  }

  async runDnsCheck(kind) {
    this.syncDraftFromDom();
    const domain = kind === 'panel'
      ? String(this._draft.panel_domain || '').trim()
      : String(this._draft.base_domain || '').trim();
    if (!domain) {
      this.toast(this.t('settings.dnsDomainRequired'), 'error');
      return;
    }
    this._checkingDns = kind;
    this.render();
    try {
      const res = await this.api('/_api/v1/settings/dns-check', {
        method: 'POST',
        json: { domain, check_wildcard: kind === 'base' }
      });
      if (kind === 'panel') this._dnsPanelResult = res;
      else this._dnsBaseResult = res;
    } catch (e) {
      this.toast(e.message, 'error');
    } finally {
      this._checkingDns = '';
      this.render();
    }
  }

  async testAlertWebhook() {
    this._testingAlert = true;
    this._alertTestResult = null;
    this.render();
    try {
      this._alertTestResult = await this.api('/_api/v1/settings/alerts/test', { method: 'POST' });
      if (this._alertTestResult.ok) {
        this.toast(this.t('settings.alertTestOk'), 'success');
      } else {
        this.toast(this._alertTestResult.error || this.t('settings.alertTestFail'), 'error');
      }
    } catch (e) {
      this.toast(e.message, 'error');
    } finally {
      this._testingAlert = false;
      this.render();
    }
  }

  effectiveOrigin() {
    const eff = (this._bundle && this._bundle.effective) || {};
    const scheme = eff.public_scheme || window.location.protocol.replace(':', '') || 'http';
    if (eff.panel_domain) {
      const port = window.location.port ? (':' + window.location.port) : '';
      const omitPort = (scheme === 'https' && port === ':443') || (scheme === 'http' && port === ':80');
      return scheme + '://' + eff.panel_domain + (omitPort ? '' : port);
    }
    return window.location.origin;
  }

  renderDnsResult(res) {
    if (!res) return '';
    const ips = res.resolved_ips && res.resolved_ips.length ? res.resolved_ips.join(', ') : '-';
    const wildIps = res.wildcard_probe_ips && res.wildcard_probe_ips.length ? res.wildcard_probe_ips.join(', ') : '';
    return `
      <div class="banner ${res.reachable_dns ? 'banner-success' : 'banner-warn'}" style="margin-top:8px;margin-bottom:0;">
        <div class="row-between">
          <div>
            <span class="badge ${res.reachable_dns ? 'badge-success' : 'badge-warn'}">${res.reachable_dns ? esc(this.t('settings.dnsReachable')) : esc(this.t('settings.dnsUnreachable'))}</span>
            <span class="mono text-xs" style="margin-left:8px;">${esc(res.domain)} &rarr; ${esc(ips)}</span>
          </div>
          <div class="row gap-xs">
            ${res.in_tls_domains ? `<span class="badge badge-info">TLS</span>` : ''}
            ${res.matches_route ? `<span class="badge badge-accent">Route</span>` : ''}
          </div>
        </div>
        ${res.wildcard_probe_host ? `<div class="text-xs mono" style="margin-top:4px;">${esc(res.wildcard_probe_host)} &rarr; ${esc(wildIps || res.error || '-')}</div>` : ''}
        ${!res.wildcard_probe_host && res.error ? `<div class="text-xs" style="margin-top:4px;">${esc(res.error)}</div>` : ''}
      </div>
    `;
  }

  renderDomainsTab() {
    const d = this._draft;
    const rt = (this._bundle && this._bundle.runtime) || {};
    const admin = this.isAdmin();
    const dis = (!admin || this._saving) ? 'disabled' : '';
    const lockoutWarn = this.needsLockoutConfirm();
    const tlsDomains = Array.isArray(rt.tls_domains) ? rt.tls_domains : [];

    return `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.domainsIdentityTitle'))}</h3>
          ${admin ? `<button type="button" class="btn btn-sm" data-reset="domains" ${dis}>${esc(this.t('settings.resetSection'))}</button>` : ''}
        </div>
        <div class="panel-body stack gap-md">
          <div class="form-grid">
            <div class="field">
              <label for="f-instance_name">${esc(this.t('settings.instanceName'))}</label>
              <input type="text" id="f-instance_name" maxlength="80" value="${esc(d.instance_name)}" placeholder="prod-tokyo" ${dis}>
              <span class="field-hint">${esc(this.t('settings.instanceNameHint'))}</span>
            </div>
            <div class="field">
              <label for="f-public_scheme">${esc(this.t('settings.publicScheme'))}</label>
              <select id="f-public_scheme" ${dis}>
                <option value="auto" ${d.public_scheme === 'auto' ? 'selected' : ''}>${esc(this.t('settings.schemeAuto'))}</option>
                <option value="https" ${d.public_scheme === 'https' ? 'selected' : ''}>https</option>
                <option value="http" ${d.public_scheme === 'http' ? 'selected' : ''}>http</option>
              </select>
              <span class="field-hint">${esc(this.t('settings.publicSchemeHint'))}</span>
            </div>
          </div>

          <div class="field">
            <label for="f-panel_domain">${esc(this.t('settings.panelDomain'))}</label>
            <div class="input-inline">
              <input type="text" id="f-panel_domain" value="${esc(d.panel_domain)}" placeholder="panel.example.com" ${dis}>
              ${admin ? `<button type="button" class="btn" id="btn-dns-panel" ${this._checkingDns === 'panel' ? 'disabled' : ''}>${this._checkingDns === 'panel' ? esc(this.t('settings.checking')) : esc(this.t('settings.checkDns'))}</button>` : ''}
            </div>
            <span class="field-hint">${esc(this.t('settings.panelDomainHint'))}</span>
            ${this.renderDnsResult(this._dnsPanelResult)}
          </div>

          <label class="checkbox-row">
            <input type="checkbox" id="f-restrict_panel_to_domain" ${d.restrict_panel_to_domain ? 'checked' : ''} ${dis}>
            <span>
              <strong>${esc(this.t('settings.restrictPanel'))}</strong><br>
              <span class="field-hint">${esc(this.t('settings.restrictPanelHint'))}</span>
            </span>
          </label>

          ${lockoutWarn ? `
            <div class="banner banner-warn" style="margin-bottom:0;">
              <div><strong>${esc(this.t('settings.lockoutWarningTitle'))}</strong>: ${esc(this.t('settings.lockoutWarningBody', { current: window.location.hostname, target: d.panel_domain }))}</div>
              <label class="checkbox-row" style="margin-top:8px;">
                <input type="checkbox" id="f-confirm_lockout_risk" ${d.confirm_lockout_risk ? 'checked' : ''} ${dis}>
                <span>${esc(this.t('settings.confirmLockoutRisk'))}</span>
              </label>
            </div>
          ` : ''}

          <div class="field">
            <label for="f-base_domain">${esc(this.t('settings.baseDomain'))}</label>
            <div class="input-inline">
              <input type="text" id="f-base_domain" value="${esc(d.base_domain)}" placeholder="apps.example.com" ${dis}>
              ${admin ? `<button type="button" class="btn" id="btn-dns-base" ${this._checkingDns === 'base' ? 'disabled' : ''}>${this._checkingDns === 'base' ? esc(this.t('settings.checking')) : esc(this.t('settings.checkWildcardDns'))}</button>` : ''}
            </div>
            <span class="field-hint">${esc(this.t('settings.baseDomainHint'))}</span>
            ${this.renderDnsResult(this._dnsBaseResult)}
          </div>

          <label class="checkbox-row">
            <input type="checkbox" id="f-trust_proxy_headers" ${d.trust_proxy_headers ? 'checked' : ''} ${dis}>
            <span>
              <strong>${esc(this.t('settings.trustProxyHeaders'))}</strong><br>
              <span class="field-hint">${esc(this.t('settings.trustProxyHeadersHint'))}</span>
            </span>
          </label>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.tlsTitle'))}</h3>
          <span class="badge ${rt.tls_active ? 'badge-success' : 'badge-warn'}">${esc(rt.tls_mode || 'disabled')}</span>
        </div>
        <div class="panel-body stack gap-md">
          ${rt.restart_required_for_tls ? `<div class="banner banner-warn" style="margin-bottom:0;">${esc(this.t('settings.tlsRestartNotice'))}</div>` : ''}
          ${rt.last_acme_error ? `<div class="banner banner-danger" style="margin-bottom:0;"><strong>ACME:</strong> <span class="mono text-xs">${esc(rt.last_acme_error)}</span></div>` : ''}

          <div class="info-grid">
            <div class="info-card">
              <div class="info-label">${esc(this.t('settings.tlsMode'))}</div>
              <div class="info-val">${esc(rt.tls_mode || 'disabled')} ${rt.tls_active ? '✓' : ''}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('settings.httpsBind'))}</div>
              <div class="info-val mono">${esc(rt.https_bind || '-')}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('settings.tlsGeneration'))}</div>
              <div class="info-val mono">#${esc(rt.tls_reload_generation ?? 0)}</div>
            </div>
            <div class="info-card">
              <div class="info-label">${esc(this.t('settings.acmeLastEvent'))}</div>
              <div class="info-val text-xs mono">${esc(rt.last_acme_event || '-')}</div>
            </div>
          </div>

          <div>
            <div class="text-xs muted">${esc(this.t('settings.managedTlsDomains'))} (${tlsDomains.length})</div>
            <div class="chips">
              ${tlsDomains.length ? tlsDomains.map((dom) => `<span class="badge badge-info mono">${esc(dom)}</span>`).join('') : `<span class="muted text-xs">${esc(this.t('common.none'))}</span>`}
            </div>
          </div>

          <div class="form-grid">
            <div class="field">
              <label for="f-acme_email">${esc(this.t('settings.acmeEmail'))}</label>
              <input type="email" id="f-acme_email" value="${esc(d.acme_email)}" placeholder="ops@example.com" ${dis}>
            </div>
            <div class="field">
              <label for="f-extra_tls_domains">${esc(this.t('settings.extraTlsDomains'))}</label>
              <textarea id="f-extra_tls_domains" placeholder="api.example.com, docs.example.com" ${dis}>${esc(d.extra_tls_domains)}</textarea>
            </div>
          </div>

          <div class="stack gap-sm">
            <label class="checkbox-row">
              <input type="checkbox" id="f-acme_enabled" ${d.acme_enabled ? 'checked' : ''} ${dis}>
              <span>${esc(this.t('settings.acmeEnabled'))}</span>
            </label>
            <label class="checkbox-row">
              <input type="checkbox" id="f-acme_staging" ${d.acme_staging ? 'checked' : ''} ${dis}>
              <span>${esc(this.t('settings.acmeStaging'))}</span>
            </label>
            <label class="checkbox-row">
              <input type="checkbox" id="f-http_redirect_https" ${d.http_redirect_https ? 'checked' : ''} ${dis}>
              <span>${esc(this.t('settings.httpRedirectHttps'))}</span>
            </label>
          </div>

          ${admin ? `
            <div class="action-bar">
              <span class="field-hint">${esc(this.t('settings.hotReloadHint'))}</span>
              <button type="button" class="btn btn-primary" id="btn-save-domains" ${dis}>${this._saving ? esc(this.t('settings.saving')) : esc(this.t('common.save'))}</button>
            </div>
          ` : ''}
        </div>
      </div>
    `;
  }

  renderRuntimeTab() {
    const d = this._draft;
    const def = (this._bundle && this._bundle.defaults) || {};
    const admin = this.isAdmin();
    const dis = (!admin || this._saving) ? 'disabled' : '';

    return `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.retentionTitle'))}</h3>
          ${admin ? `<button type="button" class="btn btn-sm" data-reset="maintenance" ${dis}>${esc(this.t('settings.resetSection'))}</button>` : ''}
        </div>
        <div class="panel-body">
          <div class="form-grid">
            <div class="field">
              <label for="f-log_retention_days">${esc(this.t('settings.logRetentionDays'))}</label>
              <input type="number" id="f-log_retention_days" min="1" max="3650" value="${esc(d.log_retention_days)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.log_retention_days }))}</span>
            </div>
            <div class="field">
              <label for="f-queue_pending_retention_secs">${esc(this.t('settings.queuePendingRetention'))}</label>
              <input type="number" id="f-queue_pending_retention_secs" min="60" max="31536000" value="${esc(d.queue_pending_retention_secs)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.queue_pending_retention_secs }))}</span>
            </div>
            <div class="field">
              <label for="f-queue_dlq_retention_secs">${esc(this.t('settings.queueDlqRetention'))}</label>
              <input type="number" id="f-queue_dlq_retention_secs" min="60" max="31536000" value="${esc(d.queue_dlq_retention_secs)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.queue_dlq_retention_secs }))}</span>
            </div>
            <div class="field">
              <label for="f-blob_gc_grace_secs">${esc(this.t('settings.blobGcGrace'))}</label>
              <input type="number" id="f-blob_gc_grace_secs" min="0" max="2592000" value="${esc(d.blob_gc_grace_secs)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.blob_gc_grace_secs }))}</span>
            </div>
          </div>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.wasmDefaultsTitle'))}</h3>
          ${admin ? `<button type="button" class="btn btn-sm" data-reset="runtime" ${dis}>${esc(this.t('settings.resetSection'))}</button>` : ''}
        </div>
        <div class="panel-body stack gap-md">
          <div class="form-grid">
            <div class="field">
              <label for="f-wasm_max_memory_mb">${esc(this.t('settings.wasmMaxMemoryMb'))}</label>
              <input type="number" id="f-wasm_max_memory_mb" min="4" max="2048" value="${esc(d.wasm_max_memory_mb)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.wasm_max_memory_mb + ' MiB' }))}</span>
            </div>
            <div class="field">
              <label for="f-wasm_epoch_timeout_ms">${esc(this.t('settings.wasmEpochTimeoutMs'))}</label>
              <input type="number" id="f-wasm_epoch_timeout_ms" min="100" max="300000" value="${esc(d.wasm_epoch_timeout_ms)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.wasm_epoch_timeout_ms + ' ms' }))}</span>
            </div>
          </div>
          <label class="checkbox-row">
            <input type="checkbox" id="f-wasm_allow_private_outbound" ${d.wasm_allow_private_outbound ? 'checked' : ''} ${dis}>
            <span>${esc(this.t('settings.wasmAllowPrivateOutbound'))}</span>
          </label>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.autoRollbackTitle'))}</h3>
          ${admin ? `<button type="button" class="btn btn-sm" data-reset="rollback" ${dis}>${esc(this.t('settings.resetSection'))}</button>` : ''}
        </div>
        <div class="panel-body stack gap-md">
          <label class="checkbox-row">
            <input type="checkbox" id="f-auto_rollback_enabled" ${d.auto_rollback_enabled ? 'checked' : ''} ${dis}>
            <span>${esc(this.t('settings.autoRollbackEnabled'))}</span>
          </label>
          <div class="form-grid">
            <div class="field">
              <label for="f-auto_rollback_window_secs">${esc(this.t('settings.autoRollbackWindow'))}</label>
              <input type="number" id="f-auto_rollback_window_secs" min="30" max="3600" value="${esc(d.auto_rollback_window_secs)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.auto_rollback_window_secs + 's' }))}</span>
            </div>
            <div class="field">
              <label for="f-auto_rollback_min_requests">${esc(this.t('settings.autoRollbackMinRequests'))}</label>
              <input type="number" id="f-auto_rollback_min_requests" min="1" max="10000" value="${esc(d.auto_rollback_min_requests)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.auto_rollback_min_requests }))}</span>
            </div>
            <div class="field">
              <label for="f-auto_rollback_error_rate_pct">${esc(this.t('settings.autoRollbackErrorRate'))}</label>
              <input type="number" id="f-auto_rollback_error_rate_pct" min="1" max="100" value="${esc(d.auto_rollback_error_rate_pct)}" ${dis}>
              <span class="field-hint">${esc(this.t('settings.defaultLabel', { value: def.auto_rollback_error_rate_pct + '%' }))}</span>
            </div>
          </div>
          ${admin ? `
            <div class="action-bar">
              <span class="field-hint">${esc(this.t('settings.runtimeApplyHint'))}</span>
              <button type="button" class="btn btn-primary" id="btn-save-runtime" ${dis}>${this._saving ? esc(this.t('settings.saving')) : esc(this.t('common.save'))}</button>
            </div>
          ` : ''}
        </div>
      </div>
    `;
  }

  renderIntegrationsTab() {
    const d = this._draft;
    const eff = (this._bundle && this._bundle.effective) || {};
    const admin = this.isAdmin();
    const dis = (!admin || this._saving) ? 'disabled' : '';
    const origin = this.effectiveOrigin();
    const mcpUrl = origin + '/_mcp';
    const openapiUrl = origin + '/_api/v1/openapi.json';
    const llmsUrl = origin + '/_api/v1/llms.txt';
    const mcpConfig = JSON.stringify({
      mcpServers: {
        ferropod: {
          url: mcpUrl,
          headers: { Authorization: 'Bearer <FERROPOD_TOKEN>' }
        }
      }
    }, null, 2);

    const usage = this._aiUsage;
    const aiProxyUrl = origin + '/_ai/v1/chat/completions';

    return `
      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.alertsTitle'))}</h3>
          <div class="row gap-xs">
            ${eff.has_alert_webhook_secret ? `<span class="badge badge-info">${esc(this.t('settings.secretConfigured'))}</span>` : ''}
            <button type="button" class="btn btn-sm" id="btn-open-ops-alerts">${esc(this.t('ops.tabAlerts'))} &rarr;</button>
            ${admin ? `<button type="button" class="btn btn-sm" data-reset="alerts" ${dis}>${esc(this.t('settings.resetSection'))}</button>` : ''}
          </div>
        </div>
        <div class="panel-body stack gap-md">
          <div class="form-grid">
            <div class="field">
              <label for="f-alert_webhook_url">${esc(this.t('settings.alertWebhookUrl'))}</label>
              <input type="url" id="f-alert_webhook_url" value="${esc(d.alert_webhook_url)}" placeholder="https://hooks.example.com/ferropod" ${dis}>
            </div>
            <div class="field">
              <label for="f-alert_webhook_secret">${esc(this.t('settings.alertWebhookSecret'))}</label>
              <input type="password" id="f-alert_webhook_secret" autocomplete="new-password" value="${esc(d.alert_webhook_secret)}" placeholder="${eff.has_alert_webhook_secret ? esc(this.t('settings.secretKeepHint')) : '••••••••'}" ${dis}>
              ${eff.has_alert_webhook_secret ? `
                <label class="checkbox-row text-xs"><input type="checkbox" id="f-clear_alert_webhook_secret" ${d.clear_alert_webhook_secret ? 'checked' : ''} ${dis}> <span>${esc(this.t('settings.clearSecret'))}</span></label>
              ` : ''}
            </div>
          </div>

          <div class="row gap-md">
            <label class="checkbox-row"><input type="checkbox" id="f-alert_on_auto_rollback" ${d.alert_on_auto_rollback ? 'checked' : ''} ${dis}> <span>${esc(this.t('settings.alertOnRollback'))}</span></label>
            <label class="checkbox-row"><input type="checkbox" id="f-alert_on_cron_failure" ${d.alert_on_cron_failure ? 'checked' : ''} ${dis}> <span>${esc(this.t('settings.alertOnCronFailure'))}</span></label>
            <label class="checkbox-row"><input type="checkbox" id="f-alert_on_worker_oom" ${d.alert_on_worker_oom ? 'checked' : ''} ${dis}> <span>${esc(this.t('settings.alertOnWorkerOom'))}</span></label>
          </div>

          ${this._alertTestResult ? `
            <div class="banner ${this._alertTestResult.ok ? 'banner-success' : 'banner-danger'}" style="margin-bottom:0;">
              <span>${this._alertTestResult.ok ? esc(this.t('settings.alertTestOk')) : esc(this._alertTestResult.error || this.t('settings.alertTestFail'))}</span>
              <span class="mono text-xs" style="margin-left:8px;">HTTP ${esc(this._alertTestResult.status_code ?? '-')} · ${esc(this._alertTestResult.latency_ms)} ms</span>
            </div>
          ` : ''}

          ${admin ? `
            <div class="action-bar">
              <button type="button" class="btn" id="btn-test-alert" ${this._testingAlert ? 'disabled' : ''}>${this._testingAlert ? esc(this.t('settings.testingAlert')) : esc(this.t('settings.sendTestAlert'))}</button>
              <button type="button" class="btn btn-primary" id="btn-save-alerts" ${dis}>${this._saving ? esc(this.t('settings.saving')) : esc(this.t('common.save'))}</button>
            </div>
          ` : ''}
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.aiGatewayTitle'))}</h3>
          <button type="button" class="btn btn-sm" id="btn-open-ai">${esc(this.t('nav.ai'))} &rarr;</button>
        </div>
        <div class="panel-body">
          <div class="info-grid">
            <div class="info-card">
              <div class="info-label">${esc(this.t('nav.ai'))}</div>
              <div class="info-val mono">${esc(this._aiProviders.length)}</div>
            </div>
            ${usage ? `
              <div class="info-card">
                <div class="info-label">${esc(this.t('settings.aiRequests'))}</div>
                <div class="info-val mono">${esc(usage.total_requests)} (${esc(usage.cached_requests)} cached)</div>
              </div>
              <div class="info-card">
                <div class="info-label">${esc(this.t('settings.aiTotalTokens'))}</div>
                <div class="info-val mono">${esc(usage.total_tokens)}</div>
              </div>
            ` : ''}
            <div class="info-card">
              <div class="info-label">${esc(this.t('settings.aiProxyEndpoint'))}</div>
              <div class="endpoint-row">
                <span class="mono text-xs endpoint-url">${esc(aiProxyUrl)}</span>
                <button type="button" class="btn btn-sm" data-copy-val="${esc(aiProxyUrl)}">${esc(this.t('common.copy'))}</button>
              </div>
            </div>
          </div>
        </div>
      </div>

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.mcpTitle'))}</h3>
        </div>
        <div class="panel-body stack gap-sm">
          <div class="endpoint-grid">
            <div class="info-card">
              <div class="info-label">MCP HTTP</div>
              <div class="endpoint-row">
                <span class="mono text-xs endpoint-url">${esc(mcpUrl)}</span>
                <button type="button" class="btn btn-sm" data-copy-val="${esc(mcpUrl)}">${esc(this.t('common.copy'))}</button>
              </div>
            </div>
            <div class="info-card">
              <div class="info-label">OpenAPI 3.1</div>
              <div class="endpoint-row">
                <span class="mono text-xs endpoint-url">${esc(openapiUrl)}</span>
                <button type="button" class="btn btn-sm" data-copy-val="${esc(openapiUrl)}">${esc(this.t('common.copy'))}</button>
              </div>
            </div>
            <div class="info-card">
              <div class="info-label">llms.txt</div>
              <div class="endpoint-row">
                <span class="mono text-xs endpoint-url">${esc(llmsUrl)}</span>
                <button type="button" class="btn btn-sm" data-copy-val="${esc(llmsUrl)}">${esc(this.t('common.copy'))}</button>
              </div>
            </div>
          </div>
          <div class="row-between" style="margin-top:6px;">
            <span class="text-xs muted">${esc(this.t('settings.mcpSnippetLabel'))}</span>
            <button type="button" class="btn btn-sm" id="btn-copy-mcp">${esc(this.t('common.copy'))}</button>
          </div>
          <pre class="code-block mono" id="mcp-snippet-pre">${esc(mcpConfig)}</pre>
        </div>
      </div>
    `;
  }

  renderSystemTab() {
    const rt = (this._bundle && this._bundle.runtime) || {};
    return `
      ${rt.insecure_default_master_key ? `
        <div class="banner banner-danger">
          <strong>${esc(this.t('settings.masterKeyWarnTitle'))}</strong>: ${esc(this.t('settings.masterKeyWarnBody'))}
        </div>
      ` : ''}

      <div class="panel">
        <div class="panel-header">
          <h3 class="panel-title">${esc(this.t('settings.systemEnvTitle'))}</h3>
          <button type="button" class="btn btn-sm" id="btn-open-updates">${esc(this.t('nav.updates'))} &rarr;</button>
        </div>
        <div class="panel-body">
          <div class="info-grid">
            <div class="info-card"><div class="info-label">${esc(this.t('updates.currentVersion'))}</div><div class="info-val mono">v${esc(rt.version || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('overview.profile'))}</div><div class="info-val">${esc(rt.profile || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.httpBind'))}</div><div class="info-val mono">${esc(rt.bind_addr || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.httpsBind'))}</div><div class="info-val mono">${esc(rt.https_bind || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.dataDir'))}</div><div class="info-val mono">${esc(rt.data_dir || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.certCacheDir'))}</div><div class="info-val mono">${esc(rt.tls_cert_cache_dir || '-')}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.memoryBudget'))}</div><div class="info-val">${esc(this.formatBytes(rt.memory_budget_bytes))}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.cpuCores'))}</div><div class="info-val mono">${esc(rt.cpu_cores)}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.tokioThreads'))}</div><div class="info-val mono">${esc(rt.tokio_worker_threads)}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.maxConcurrency'))}</div><div class="info-val mono">${esc(rt.max_concurrency)}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.queueConcurrency'))}</div><div class="info-val mono">${esc(rt.queue_concurrency)}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.maxOpenDbs'))}</div><div class="info-val mono">${esc(rt.max_open_user_dbs)}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.maxBlobUpload'))}</div><div class="info-val">${esc(this.formatBytes(rt.max_blob_upload_bytes))}</div></div>
            <div class="info-card"><div class="info-label">${esc(this.t('settings.wasmCacheCap'))}</div><div class="info-val mono">${esc(rt.wasm_cache_max_compiled)}</div></div>
            ${rt.local_api_host ? `
              <div class="info-card"><div class="info-label">Local API Proxy</div><div class="info-val mono text-xs">${esc(rt.local_api_host + (rt.local_api_prefix || '') + ' -> ' + (rt.local_api_upstream || ''))}</div></div>
            ` : ''}
          </div>
        </div>
      </div>
    `;
  }

  bindEvents() {
    const sr = this.shadowRoot;
    sr.querySelectorAll('[data-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this.syncDraftFromDom();
        this._tab = btn.getAttribute('data-tab');
        this.render();
      });
    });

    const reloadBtn = sr.getElementById('btn-reload-settings');
    if (reloadBtn) reloadBtn.addEventListener('click', () => this.loadAll());

    const resetAllBtn = sr.getElementById('btn-reset-all');
    if (resetAllBtn) {
      resetAllBtn.addEventListener('click', () => {
        this.requestConfirm(this.t('settings.confirmResetAll'), () => {
          this.resetSection('all');
        });
      });
    }

    sr.querySelectorAll('[data-reset]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const sec = btn.getAttribute('data-reset');
        this.resetSection(sec);
      });
    });

    // Live sync for anti-lockout banner visibility & draft preservation
    sr.querySelectorAll('input, select, textarea').forEach((el) => {
      const evt = (el.type === 'checkbox' || el.tagName === 'SELECT') ? 'change' : 'input';
      el.addEventListener(evt, () => {
        const prevWarn = this.needsLockoutConfirm();
        this.syncDraftFromDom();
        if (el.id === 'f-restrict_panel_to_domain' || (el.id === 'f-panel_domain' && prevWarn !== this.needsLockoutConfirm())) {
          this.render();
        }
      });
    });

    const dnsPanelBtn = sr.getElementById('btn-dns-panel');
    if (dnsPanelBtn) dnsPanelBtn.addEventListener('click', () => this.runDnsCheck('panel'));

    const dnsBaseBtn = sr.getElementById('btn-dns-base');
    if (dnsBaseBtn) dnsBaseBtn.addEventListener('click', () => this.runDnsCheck('base'));

    const saveDomBtn = sr.getElementById('btn-save-domains');
    if (saveDomBtn) saveDomBtn.addEventListener('click', () => this.saveDomainsTab());

    const saveRtBtn = sr.getElementById('btn-save-runtime');
    if (saveRtBtn) saveRtBtn.addEventListener('click', () => this.saveRuntimeTab());

    const saveAlertsBtn = sr.getElementById('btn-save-alerts');
    if (saveAlertsBtn) saveAlertsBtn.addEventListener('click', () => this.saveAlertsSection());

    const testAlertBtn = sr.getElementById('btn-test-alert');
    if (testAlertBtn) testAlertBtn.addEventListener('click', () => this.testAlertWebhook());

    sr.querySelectorAll('[data-copy-val]').forEach((btn) => {
      btn.addEventListener('click', () => this.copyText(btn.getAttribute('data-copy-val')));
    });

    const copyMcpBtn = sr.getElementById('btn-copy-mcp');
    const mcpPre = sr.getElementById('mcp-snippet-pre');
    if (copyMcpBtn && mcpPre) {
      copyMcpBtn.addEventListener('click', () => this.copyText(mcpPre.textContent || ''));
    }

    [
      ['btn-open-updates', '#/updates'],
      ['btn-open-ai', '#/ai'],
      ['btn-open-ops-alerts', '#/ops']
    ].forEach(([id, hash]) => {
      const btn = sr.getElementById(id);
      if (btn) {
        btn.addEventListener('click', () => {
          this.dispatchEvent(new CustomEvent('fp-navigate', {
            bubbles: true,
            composed: true,
            detail: { hash }
          }));
        });
      }
    });
  }

  render() {
    const admin = this.isAdmin();
    let bodyHtml = '';
    if (this._loading && !this._bundle) {
      bodyHtml = `<div class="panel"><div class="empty-state">${esc(this.t('common.loading'))}</div></div>`;
    } else if (this._error) {
      bodyHtml = `<div class="banner banner-danger">${esc(this._error)}</div>`;
    } else if (this._bundle && this._draft) {
      let tabContent = '';
      if (this._tab === 'domains') tabContent = this.renderDomainsTab();
      else if (this._tab === 'runtime') tabContent = this.renderRuntimeTab();
      else if (this._tab === 'integrations') tabContent = this.renderIntegrationsTab();
      else tabContent = this.renderSystemTab();

      bodyHtml = `
        ${!admin ? `<div class="banner banner-info">${esc(this.t('common.readonlyHint'))}</div>` : ''}
        <div class="tabs" role="tablist">
          <button type="button" class="tab-btn ${this._tab === 'domains' ? 'active' : ''}" data-tab="domains">${esc(this.t('settings.tabDomains'))}</button>
          <button type="button" class="tab-btn ${this._tab === 'runtime' ? 'active' : ''}" data-tab="runtime">${esc(this.t('settings.tabRuntime'))}</button>
          <button type="button" class="tab-btn ${this._tab === 'integrations' ? 'active' : ''}" data-tab="integrations">${esc(this.t('settings.tabIntegrations'))}</button>
          <button type="button" class="tab-btn ${this._tab === 'system' ? 'active' : ''}" data-tab="system">${esc(this.t('settings.tabSystem'))}</button>
        </div>
        ${tabContent}
      `;
    }

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">${esc(this.t('settings.title'))}</h2>
          <p class="page-subtitle">${esc(this.t('settings.subtitle'))}</p>
        </div>
        <div class="row gap-xs">
          <button type="button" class="btn btn-sm" id="btn-reload-settings" ${this._loading ? 'disabled' : ''}>${esc(this.t('common.refresh'))}</button>
          ${admin && this._bundle ? `<button type="button" class="btn btn-sm btn-danger" id="btn-reset-all" ${this._saving ? 'disabled' : ''}>${esc(this.t('settings.resetAll'))}</button>` : ''}
        </div>
      </div>
      ${bodyHtml}
    `;
    this.bindEvents();
  }
}

if (!customElements.get('fp-platform-settings')) {
  customElements.define('fp-platform-settings', FpPlatformSettings);
}
