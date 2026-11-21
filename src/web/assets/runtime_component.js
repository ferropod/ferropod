// Native HTML Web Components:
//   1. <fp-runtime-panel>    — Runtime & Workers overview, limits, workloads & invoke playground (#/runtime)
//   2. <fp-project-bindings> — Visual Resource Bindings manager (DB / KV / Queue / Bucket / Search) inside Project Detail

const runtimeSheet = new CSSStyleSheet();
runtimeSheet.replaceSync(`
  *, *::before, *::after { box-sizing: border-box; }
  :host {
    display: block;
    color: var(--text);
    font-family: var(--font-sans);
    font-size: 14px;
    line-height: 1.5;
  }
  a { color: inherit; text-decoration: none; }
  button, input, select, textarea { font: inherit; color: inherit; }
  code, pre, .mono { font-family: var(--font-mono); }
  .muted { color: var(--muted); }
  .text-xs { font-size: 12px; }
  .text-sm { font-size: 13px; }
  .text-strong { font-weight: 600; color: var(--text); }
  .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

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
    margin-bottom: 18px;
  }
  .page-title {
    margin: 0;
    font-size: 20px;
    font-weight: 700;
    letter-spacing: -0.02em;
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .page-subtitle { margin: 4px 0 0; color: var(--muted); font-size: 13px; }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(175px, 1fr));
    gap: 12px;
  }
  .stat-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    box-shadow: var(--shadow-sm);
  }
  .stat-label { font-size: 12px; color: var(--muted); }
  .stat-value { font-size: 18px; font-weight: 700; }

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
  .badge-danger { background: var(--danger-soft); color: var(--danger); }

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
    vertical-align: middle;
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

  input[type="text"], select, textarea {
    width: 100%;
    padding: 8px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    outline: none;
  }
  input:focus, select:focus, textarea:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  textarea { min-height: 84px; resize: vertical; }

  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }

  .binding-row {
    display: grid;
    grid-template-columns: 140px 180px minmax(180px, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-subtle);
  }

  @media (max-width: 760px) {
    .binding-row { grid-template-columns: 1fr; }
  }

  .invoke-bar {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .invoke-bar select { width: auto; }
  .invoke-bar input[type="text"] { flex: 1 1 220px; }

  .link-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 12px;
    cursor: pointer;
    transition: border-color 0.14s ease, color 0.14s ease;
  }
  .link-pill:hover { border-color: var(--accent); color: var(--accent); }

  dialog.fp-dialog {
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-md);
    width: min(520px, calc(100vw - 24px));
    max-height: 88vh;
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
  .dialog-body { padding: 16px; overflow-y: auto; max-height: 66vh; }
  .dialog-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    background: var(--surface-subtle);
  }
  .grid-2 { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
  @media (max-width: 560px) { .grid-2 { grid-template-columns: 1fr; } }
  .hidden { display: none !important; }
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
    formatTime: (v) => String(v || '')
  };
}

function emitToast(el, message, kind = 'info') {
  el.dispatchEvent(new CustomEvent('fp-toast', {
    detail: { message, kind },
    bubbles: true,
    composed: true
  }));
}

function emitNavigate(el, hash) {
  el.dispatchEvent(new CustomEvent('fp-navigate', {
    detail: { hash },
    bubbles: true,
    composed: true
  }));
}

// ============================================================================
// 1. <fp-runtime-panel> — Runtime & Workers Overview + Invoke Playground
// ============================================================================

class FpRuntimePanel extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [runtimeSheet];
    this._status = null;
    this._projects = [];
    this._routes = [];
    this._token = '';
    this._canWrite = true;
    this._invoke = {
      projectId: '',
      method: 'GET',
      path: '/health',
      body: '',
      result: null,
      loading: false
    };
  }

  set status(v) { this._status = v; if (this.isConnected) this.render(); }
  set token(v) { this._token = String(v || ''); }
  set canWrite(v) { this._canWrite = Boolean(v); if (this.isConnected) this.render(); }
  set projects(list) {
    this._projects = Array.isArray(list) ? list : [];
    if (!this._invoke.projectId && this._projects.length) {
      this._invoke.projectId = this._projects[0].id;
    }
    if (this.isConnected) this.render();
  }
  set routes(list) { this._routes = Array.isArray(list) ? list : []; if (this.isConnected) this.render(); }

  _authHeaders(json = false) {
    const tok = this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
    const h = {};
    if (tok) h.Authorization = 'Bearer ' + tok;
    if (json) h['Content-Type'] = 'application/json';
    return h;
  }

  connectedCallback() {
    this.render();
  }

  async runInvoke() {
    const { t } = getI18n();
    const proj = this._projects.find((p) => p.id === this._invoke.projectId);
    const route = proj ? this._routes.find((r) => r.project_id === proj.id) : null;
    this._invoke.loading = true;
    this.render();

    const started = performance.now();
    try {
      if (this._invoke.path === '/_healthz') {
        const res = await fetch('/_healthz');
        const txt = await res.text();
        this._invoke.result = {
          status: res.status,
          latencyMs: (performance.now() - started).toFixed(1),
          mode: 'live',
          host: window.location.host,
          body: txt
        };
      } else if (proj) {
        const res = await fetch('/_api/v1/runtimes/' + encodeURIComponent(proj.id) + '/invoke', {
          method: 'POST',
          headers: this._authHeaders(true),
          body: JSON.stringify({
            method: this._invoke.method,
            path: this._invoke.path || '/',
            body: this._invoke.body || null
          })
        });
        const text = await res.text();
        let data = null;
        try {
          data = JSON.parse(text);
        } catch (_) {
          data = null;
        }
        if (!res.ok) {
          this._invoke.result = {
            status: res.status,
            latencyMs: (performance.now() - started).toFixed(1),
            mode: 'live',
            host: route ? route.host : `${proj.name}.local`,
            body: (data && data.error) ? data.error : text
          };
        } else {
          this._invoke.result = {
            status: data.status,
            latencyMs: typeof data.latency_ms === 'number' ? data.latency_ms.toFixed(1) : (performance.now() - started).toFixed(1),
            mode: 'live',
            host: route ? route.host : `${proj.name}.local`,
            body: data.body || ''
          };
        }
      }
    } catch (err) {
      emitToast(this, err.message || t('common.empty'), 'error');
    } finally {
      this._invoke.loading = false;
      this.render();
    }
  }

  render() {
    const { t } = getI18n();
    const profile = (this._status && this._status.profile) || 'Standard';
    const isLowMem = String(profile).toLowerCase() === 'lowmem';
    const memLimit = isLowMem ? '32 MiB / instance' : '128 MiB / instance';
    const maxConc = isLowMem ? '16 concurrent' : '64 concurrent';

    const runtimes = [...this._projects].sort((a, b) =>
      (a.group || '').localeCompare(b.group || '') || a.name.localeCompare(b.name)
    );
    const hasGroups = runtimes.some((runtime) => runtime.group);
    let lastGroup;
    const workloadRows = runtimes.map((p) => {
      const pRoutes = this._routes.filter((r) => r.project_id === p.id);
      const hasActive = Boolean(p.active_deployment_id);
      const group = p.group || '';
      const heading = group !== lastGroup && hasGroups
        ? `<tr><th colspan="6">${esc(group || t('groups.ungrouped'))}</th></tr>`
        : '';
      lastGroup = group;
      return `${heading}
        <tr>
          <td>
            <button type="button" class="link-pill text-strong" data-go-hash="#/runtime/${encodeURIComponent(p.id)}">${esc(p.name)}</button>
            <div class="muted mono text-xs" style="margin-top:3px;">${esc(p.id)}</div>
          </td>
          <td>
            ${hasActive
              ? `<span class="badge badge-success">${esc(t('runtime.active'))}</span>`
              : `<span class="badge badge-neutral">${esc(t('runtime.idle'))}</span>`}
          </td>
          <td class="mono text-xs">${esc(p.active_deployment_id || '-')}</td>
          <td>
            ${pRoutes.length
              ? pRoutes.map((r) => `<span class="badge badge-info mono">${esc(r.host)}</span>`).join(' ')
              : `<span class="muted text-xs">-</span>`}
          </td>
          <td class="mono text-xs">${esc(p.max_concurrency)}</td>
          <td class="row gap-xs">
            <button type="button" class="btn btn-sm" data-go-hash="#/runtime/${encodeURIComponent(p.id)}" data-open-bindings="1">${esc(t('runtime.manageBindings'))}</button>
            <button type="button" class="btn btn-sm" data-pick-invoke="${esc(p.id)}">${esc(t('runtime.testInvoke'))}</button>
          </td>
        </tr>
      `;
    }).join('');

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">
            ${esc(t('runtime.title'))}
            <span class="badge badge-info">Wasmtime · WASI HTTP 0.2</span>
          </h2>
          <p class="page-subtitle">${esc(t('runtime.subtitle'))}</p>
        </div>
        <div class="row gap-xs">
          ${this._canWrite ? `<button type="button" class="btn btn-primary btn-sm" id="new-runtime-btn">+ ${esc(t('runtime.newRuntime'))}</button>` : ''}
        </div>
      </div>

      <div class="stack gap-md">
        <div class="stats-grid">
          <div class="stat-card">
            <span class="stat-label">${esc(t('runtime.engine'))}</span>
            <span class="stat-value">Wasmtime AOT</span>
            <span class="text-xs muted">Static Live · Wasm M2</span>
          </div>
          <div class="stat-card">
            <span class="stat-label">${esc(t('runtime.memLimit'))}</span>
            <span class="stat-value mono">${esc(memLimit)}</span>
            <span class="text-xs muted">Profile: ${esc(profile)}</span>
          </div>
          <div class="stat-card">
            <span class="stat-label">${esc(t('runtime.concurrency'))}</span>
            <span class="stat-value mono">${esc(maxConc)}</span>
            <span class="text-xs muted">Epoch timeout: 30s</span>
          </div>
          <div class="stat-card">
            <span class="stat-label">${esc(t('runtime.witInterfaces'))}</span>
            <div class="row gap-xs" style="margin-top:4px;">
              <span class="badge badge-accent mono">db</span>
              <span class="badge badge-accent mono">kv</span>
              <span class="badge badge-accent mono">queue</span>
              <span class="badge badge-accent mono">bucket</span>
              <span class="badge badge-info mono">search</span>
              <span class="badge badge-neutral mono">env</span>
            </div>
          </div>
        </div>

        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">${esc(t('runtime.workloads'))} (${this._projects.length})</h3>
            <span class="text-xs muted">${esc(t('runtime.workloadsHint'))}</span>
          </div>
          ${workloadRows ? `
            <div class="table-wrap">
              <table class="data-table">
                <thead>
                  <tr>
                    <th>${esc(t('runtime.runtimeName'))}</th>
                    <th>${esc(t('common.status'))}</th>
                    <th>${esc(t('projects.activeDeployment'))}</th>
                    <th>${esc(t('nav.routes'))}</th>
                    <th>${esc(t('projects.maxConcurrency'))}</th>
                    <th>${esc(t('common.actions'))}</th>
                  </tr>
                </thead>
                <tbody>${workloadRows}</tbody>
              </table>
            </div>
          ` : `<div class="empty-state">${esc(t('runtime.noWorkloads'))}</div>`}
        </div>

        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">
              <span>${esc(t('runtime.invokeTitle'))}</span>
              <span class="badge badge-success">Live</span>
            </h3>
          </div>
          <div class="panel-body stack gap-sm">
            <form id="invoke-form" class="invoke-bar">
              <select id="inv-proj">
                ${this._projects.length
                  ? this._projects.map((p) => `<option value="${esc(p.id)}" ${this._invoke.projectId === p.id ? 'selected' : ''}>${esc(p.name)}</option>`).join('')
                  : `<option value="">(${esc(t('common.empty'))})</option>`}
              </select>
              <select id="inv-method">
                ${['GET', 'POST', 'PUT', 'DELETE'].map((m) => `<option value="${m}" ${this._invoke.method === m ? 'selected' : ''}>${m}</option>`).join('')}
              </select>
              <input type="text" id="inv-path" class="mono" value="${esc(this._invoke.path)}" placeholder="/api/hello">
              <button type="submit" class="btn btn-primary btn-sm" ${this._invoke.loading ? 'disabled' : ''}>${esc(t('runtime.sendRequest'))}</button>
            </form>
            ${this._invoke.result ? `
              <div class="row-between text-xs muted">
                <div class="row gap-xs">
                  <span class="badge ${this._invoke.result.status < 400 ? 'badge-success' : 'badge-warn'}">HTTP ${this._invoke.result.status}</span>
                  <span class="mono">${this._invoke.result.latencyMs} ms</span>
                  <span class="mono">Host: ${esc(this._invoke.result.host)}</span>
                </div>
                <span class="badge badge-neutral">${esc(this._invoke.result.mode)}</span>
              </div>
              <pre class="code-block">${esc(this._invoke.result.body)}</pre>
            ` : ''}
          </div>
        </div>
      </div>

    `;

    this._bindEvents();
  }

  _bindEvents() {
    const sr = this.shadowRoot;
    sr.querySelectorAll('[data-go-hash]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const hash = btn.getAttribute('data-go-hash');
        const openBindings = btn.getAttribute('data-open-bindings') === '1';
        this.dispatchEvent(new CustomEvent('fp-navigate', {
          detail: { hash, projectTab: openBindings ? 'bindings' : undefined },
          bubbles: true,
          composed: true
        }));
      });
    });

    sr.querySelectorAll('[data-pick-invoke]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._invoke.projectId = btn.getAttribute('data-pick-invoke');
        this.render();
      });
    });

    const form = sr.getElementById('invoke-form');
    if (form) {
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._invoke.projectId = sr.getElementById('inv-proj').value;
        this._invoke.method = sr.getElementById('inv-method').value;
        this._invoke.path = sr.getElementById('inv-path').value.trim() || '/';
        this.runInvoke();
      });
    }

    const createButton = sr.getElementById('new-runtime-btn');
    if (createButton) {
      createButton.addEventListener('click', () => {
        this.dispatchEvent(new CustomEvent('fp-create-runtime'));
      });
    }
  }
}

// ============================================================================
// 2. <fp-project-bindings> — Visual Resource Bindings Manager for a Project
// ============================================================================

const BINDING_TYPES = [
  { type: 'db', label: 'Database (Turso)', route: '#/dbs', defaultVar: 'DB' },
  { type: 'kv', label: 'KV Namespace', route: '#/kv', defaultVar: 'KV' },
  { type: 'queue', label: 'Queue', route: '#/queues', defaultVar: 'QUEUE' },
  { type: 'bucket', label: 'Object Bucket', route: '#/blobs', defaultVar: 'BUCKET' }
];

class FpProjectBindings extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [runtimeSheet];
    this._project = null;
    this._activeDeployment = null;
    this._resources = { dbs: [], kvNamespaces: [], queues: [], buckets: [] };
    this._canWrite = false;
    this._token = '';
    this._rows = []; // [{ type, logical, target }]
    this._saving = false;
    this._targetEnv = 'prod';
    this._initializedForDep = null;
  }

  set project(p) { this._project = p; this._syncInitialRows(); if (this.isConnected) this.render(); }
  set activeDeployment(d) { this._activeDeployment = d; this._syncInitialRows(); if (this.isConnected) this.render(); }
  set resources(r) { this._resources = r || { dbs: [], kvNamespaces: [], queues: [], buckets: [] }; if (this.isConnected) this.render(); }
  set canWrite(v) { this._canWrite = Boolean(v); if (this.isConnected) this.render(); }
  set token(v) { this._token = String(v || ''); }

  _syncInitialRows() {
    const depKey = (this._project ? this._project.id : '') + ':' + (this._activeDeployment ? this._activeDeployment.id : 'none');
    if (this._initializedForDep === depKey) return;
    this._initializedForDep = depKey;
    const b = (this._activeDeployment && this._activeDeployment.bindings) || {};
    const rows = [];
    ['db', 'kv', 'queue', 'bucket'].forEach((type) => {
      const map = b[type] || {};
      Object.keys(map).forEach((logical) => {
        rows.push({ type, logical, target: map[logical] });
      });
    });
    this._rows = rows;
  }

  _optionsForType(type) {
    if (type === 'db') return (this._resources.dbs || []).map((d) => ({ value: d.name, label: `${d.name}${d.project_id ? '' : ' (global)'}` }));
    if (type === 'kv') return (this._resources.kvNamespaces || []).map((n) => ({ value: n.name, label: `${n.name}${n.project_id ? '' : ' (global)'}` }));
    if (type === 'queue') return (this._resources.queues || []).map((q) => ({ value: q.name, label: `${q.name}${q.project_id ? '' : ' (global)'}` }));
    if (type === 'bucket') return (this._resources.buckets || []).map((b) => ({ value: b.name, label: `${b.name}${b.project_id ? '' : ' (global)'}` }));
    return [];
  }

  _buildBindingsObj() {
    const current = this._activeDeployment?.bindings || {};
    const out = {
      kv: {}, db: {}, queue: {}, bucket: {},
      env: current.env || {},
      search: current.search || {}
    };
    this._rows.forEach((r) => {
      const k = String(r.logical || '').trim();
      const v = String(r.target || '').trim();
      if (k && v && out[r.type]) {
        out[r.type][k] = v;
      }
    });
    return out;
  }

  _buildWitSnippet(bindingsObj) {
    const lines = [];
    Object.entries(bindingsObj.db).forEach(([k, v]) => lines.push(`// db binding "${k}" -> ${v}\nlet res = db::query("${k}", "SELECT * FROM items LIMIT ?1", &[SqlValue::Integer(10)])?;`));
    Object.entries(bindingsObj.kv).forEach(([k, v]) => lines.push(`// kv binding "${k}" -> ${v}\nlet val = kv::get("${k}", "session:1")?;\nlet page = kv::list("${k}", Some("user:"), None, 50)?;`));
    Object.entries(bindingsObj.queue).forEach(([k, v]) => lines.push(`// queue binding "${k}" -> ${v}\nqueue::enqueue("${k}", b"{\\"task\\":1}", None)?;`));
    Object.entries(bindingsObj.bucket).forEach(([k, v]) => lines.push(`// bucket binding "${k}" -> ${v}\nlet page = bucket::list("${k}", Some("assets/"), None, 50)?;`));
    lines.push(`// structured host logging\nlog::emit(log::Level::Info, "handled request", None);`);
    return lines.join('\n\n');
  }

  async applyBindings() {
    if (!this._project) return;
    const { t } = getI18n();
    const dep = this._activeDeployment || {};
    const bindings = this._buildBindingsObj();
    const payload = {
      artifact_path: dep.artifact_path || null,
      assets_bucket: dep.assets_bucket || null,
      assets_prefix: dep.assets_prefix || null,
      spa_fallback: Boolean(dep.spa_fallback),
      bindings,
      crons: Array.isArray(dep.crons) ? dep.crons : [],
      target_env: this._targetEnv
    };

    const tok = this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
    this._saving = true;
    this.render();
    try {
      const res = await fetch('/_api/v1/runtimes/' + encodeURIComponent(this._project.id) + '/deployments', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...(tok ? { Authorization: 'Bearer ' + tok } : {})
        },
        body: JSON.stringify(payload)
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      emitToast(this, t('bindings.saved'), 'success');
      this._initializedForDep = null;
      this.dispatchEvent(new CustomEvent('fp-bindings-saved', { bubbles: true, composed: true }));
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._saving = false;
      this.render();
    }
  }

  connectedCallback() {
    this._syncInitialRows();
    this.render();
  }

  render() {
    const { t } = getI18n();
    const bindingsObj = this._buildBindingsObj();

    const rowsHtml = this._rows.length
      ? this._rows.map((r, idx) => {
          const opts = this._optionsForType(r.type);
          const hasCurrent = opts.some((o) => o.value === r.target);
          const typeMeta = BINDING_TYPES.find((bt) => bt.type === r.type) || BINDING_TYPES[0];
          return `
            <div class="binding-row" data-row-idx="${idx}">
              <select data-field="type" ${this._canWrite ? '' : 'disabled'}>
                ${BINDING_TYPES.map((bt) => `<option value="${bt.type}" ${r.type === bt.type ? 'selected' : ''}>${esc(bt.label)}</option>`).join('')}
              </select>
              <input type="text" class="mono" data-field="logical" value="${esc(r.logical)}" placeholder="${esc(typeMeta.defaultVar)}" ${this._canWrite ? '' : 'disabled'}>
              <select data-field="target" ${this._canWrite ? '' : 'disabled'}>
                <option value="">-- ${esc(t('bindings.selectResource'))} --</option>
                ${!hasCurrent && r.target ? `<option value="${esc(r.target)}" selected>${esc(r.target)}</option>` : ''}
                ${opts.map((o) => `<option value="${esc(o.value)}" ${r.target === o.value ? 'selected' : ''}>${esc(o.label)}</option>`).join('')}
              </select>
              <div class="row gap-xs">
                <button type="button" class="btn btn-ghost btn-sm" data-go-res="${esc(typeMeta.route)}" title="Open">↗</button>
                ${this._canWrite ? `<button type="button" class="btn btn-danger btn-sm" data-remove-row="${idx}">${esc(t('common.delete'))}</button>` : ''}
              </div>
            </div>
          `;
        }).join('')
      : `<div class="empty-state">${esc(t('bindings.empty'))}</div>`;

    this.shadowRoot.innerHTML = `
      <div class="stack gap-md">
        <div class="panel">
          <div class="panel-header">
            <div>
              <h3 class="panel-title">${esc(t('bindings.title'))}</h3>
              <div class="muted text-xs" style="margin-top:2px;">${esc(t('bindings.subtitle'))}</div>
            </div>
            ${this._canWrite ? `
              <div class="row gap-xs">
                <button type="button" class="btn btn-sm" id="add-binding-btn">+ ${esc(t('bindings.add'))}</button>
                <select id="bind-target-env" style="width:auto;">
                  <option value="prod" ${this._targetEnv === 'prod' ? 'selected' : ''}>Prod</option>
                  <option value="preview" ${this._targetEnv === 'preview' ? 'selected' : ''}>Preview</option>
                </select>
                <button type="button" class="btn btn-primary btn-sm" id="apply-bindings-btn" ${this._saving ? 'disabled' : ''}>${esc(this._saving ? t('common.loading') : t('bindings.apply'))}</button>
              </div>
            ` : ''}
          </div>
          <div class="panel-body stack gap-sm">
            ${rowsHtml}
          </div>
        </div>

        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">${esc(t('bindings.previewTitle'))}</h3>
            <span class="badge badge-neutral mono">wit/platform.wit</span>
          </div>
          <div class="panel-body stack gap-sm">
            <pre class="code-block">${esc(this._buildWitSnippet(bindingsObj))}</pre>
            <details>
              <summary class="muted text-xs" style="cursor:pointer;">bindings_json</summary>
              <pre class="code-block" style="margin-top:6px;">${esc(JSON.stringify(bindingsObj, null, 2))}</pre>
            </details>
          </div>
        </div>
      </div>
    `;

    this._bindEvents();
  }

  _bindEvents() {
    const sr = this.shadowRoot;
    const addBtn = sr.getElementById('add-binding-btn');
    if (addBtn) {
      addBtn.addEventListener('click', () => {
        const defaultOpts = this._optionsForType('db');
        this._rows.push({
          type: 'db',
          logical: 'DB',
          target: defaultOpts.length ? defaultOpts[0].value : ''
        });
        this.render();
      });
    }

    const envSel = sr.getElementById('bind-target-env');
    if (envSel) {
      envSel.addEventListener('change', (ev) => {
        this._targetEnv = ev.target.value;
      });
    }

    const applyBtn = sr.getElementById('apply-bindings-btn');
    if (applyBtn) {
      applyBtn.addEventListener('click', () => this.applyBindings());
    }

    sr.querySelectorAll('.binding-row').forEach((rowEl) => {
      const idx = Number(rowEl.getAttribute('data-row-idx'));
      const typeSel = rowEl.querySelector('[data-field="type"]');
      const logInp = rowEl.querySelector('[data-field="logical"]');
      const tgtSel = rowEl.querySelector('[data-field="target"]');

      if (typeSel) {
        typeSel.addEventListener('change', (ev) => {
          const newType = ev.target.value;
          const bt = BINDING_TYPES.find((b) => b.type === newType);
          const opts = this._optionsForType(newType);
          this._rows[idx].type = newType;
          if (!this._rows[idx].logical || BINDING_TYPES.some((b) => b.defaultVar === this._rows[idx].logical)) {
            this._rows[idx].logical = bt ? bt.defaultVar : 'RES';
          }
          this._rows[idx].target = opts.length ? opts[0].value : '';
          this.render();
        });
      }
      if (logInp) {
        logInp.addEventListener('input', (ev) => {
          this._rows[idx].logical = ev.target.value;
        });
      }
      if (tgtSel) {
        tgtSel.addEventListener('change', (ev) => {
          this._rows[idx].target = ev.target.value;
          this.render();
        });
      }
    });

    sr.querySelectorAll('[data-remove-row]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const idx = Number(btn.getAttribute('data-remove-row'));
        this._rows.splice(idx, 1);
        this.render();
      });
    });

    sr.querySelectorAll('[data-go-res]').forEach((btn) => {
      btn.addEventListener('click', () => {
        emitNavigate(this, btn.getAttribute('data-go-res'));
      });
    });
  }
}

if (!customElements.get('fp-runtime-panel')) {
  customElements.define('fp-runtime-panel', FpRuntimePanel);
}
if (!customElements.get('fp-project-bindings')) {
  customElements.define('fp-project-bindings', FpProjectBindings);
}
