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
    touch-action: manipulation;
    max-width: 100%;
  }
  a { color: inherit; text-decoration: none; }
  button, input, select, textarea { font: inherit; color: inherit; }
  code, pre, .mono { font-family: var(--font-mono); }
  .muted { color: var(--muted); }
  .text-xs { font-size: 12px; }
  .text-sm { font-size: 13px; }
  .text-strong { font-weight: 600; color: var(--text); }
  .truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .break-all { word-break: break-all; overflow-wrap: anywhere; }
  .id-sub { word-break: break-all; overflow-wrap: anywhere; }
  .route-badges { display: flex; flex-wrap: wrap; gap: 4px; }
  .route-badge { white-space: normal; word-break: break-all; overflow-wrap: anywhere; max-width: 100%; }
  .route-badge-btn { border: 1px solid transparent; cursor: pointer; font-family: var(--font-mono); }
  .route-badge-btn:hover { border-color: var(--info); }
  .workloads-head-tools { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; }
  .workloads-head-tools input[type="search"] { min-width: 220px; max-width: 300px; width: auto; }
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

  input[type="text"], input[type="search"], select, textarea {
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
  @media (max-width: 767px) {
    .page-head { flex-direction: column; align-items: stretch; gap: 10px; }
    .page-head .row { width: 100%; }
    #new-runtime-btn { width: 100%; }
    .stats-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }
    .stat-card { padding: 12px; }
    .stat-value { font-size: 15px; word-break: break-word; }
    .panel-header, .panel-body { padding: 12px 14px; }
    input[type="text"], input[type="search"], select, textarea { font-size: 16px; min-height: 42px; }
    .workloads-head-tools { width: 100%; }
    .workloads-head-tools input[type="search"] { width: 100%; min-width: 0; max-width: none; }
    .btn { min-height: 38px; }
    .btn-sm { min-height: 36px; }
    .invoke-bar { flex-direction: column; align-items: stretch; }
    .invoke-bar select, .invoke-bar input[type="text"], .invoke-bar .btn { width: 100%; flex: 0 0 auto; }
    dialog.fp-dialog { width: calc(100vw - 16px); max-height: 90dvh; }

    .data-table.stack-mobile thead { display: none; }
    .data-table.stack-mobile,
    .data-table.stack-mobile tbody,
    .data-table.stack-mobile tr,
    .data-table.stack-mobile td {
      display: block;
      width: 100%;
    }
    .data-table.stack-mobile tr {
      padding: 14px;
      border-bottom: 1px solid var(--border);
    }
    .data-table.stack-mobile tr:last-child { border-bottom: none; }
    .data-table.stack-mobile tr.group-heading-row {
      padding: 8px 14px;
      background: var(--surface-subtle);
      border-bottom: 1px solid var(--border);
    }
    .data-table.stack-mobile tr.group-heading-row th {
      display: block;
      width: 100%;
      padding: 0;
      border: none;
      background: transparent;
      font-size: 12px;
      font-weight: 700;
      color: var(--muted);
    }
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
    .data-table.stack-mobile td > * {
      min-width: 0;
      max-width: 100%;
    }
    .data-table.stack-mobile td::before {
      content: attr(data-label);
      color: var(--muted);
      font-size: 12px;
      font-weight: 500;
      text-align: left;
      flex-shrink: 0;
    }
    .data-table.stack-mobile td .route-badges {
      justify-content: flex-end;
      max-width: 68%;
    }
    .data-table.stack-mobile td.cell-primary {
      display: block;
      padding: 0 0 8px;
      text-align: left;
      border-bottom: 1px solid var(--border);
      margin-bottom: 6px;
    }
    .data-table.stack-mobile td.cell-primary::before { display: none; }
    .data-table.stack-mobile td.cell-primary .link-pill {
      max-width: 100%;
      padding: 5px 11px;
      font-size: 13px;
    }
    .data-table.stack-mobile td.cell-actions-td {
      display: block;
      padding: 10px 0 0;
      margin-top: 6px;
      border-top: 1px dashed var(--border);
    }
    .data-table.stack-mobile td.cell-actions-td::before { display: none; }
    .data-table.stack-mobile td.cell-actions-td .cell-actions {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 8px;
      width: 100%;
    }
    .data-table.stack-mobile td.cell-actions-td .btn {
      width: 100%;
      min-height: 38px;
    }
  }
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
    this._search = '';
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

  _getFilteredProjects() {
    const sorted = [...this._projects].sort((a, b) =>
      (a.group || '').localeCompare(b.group || '') || a.name.localeCompare(b.name)
    );
    const q = this._search.trim().toLowerCase();
    if (!q) return sorted;
    return sorted.filter((p) => {
      if (
        p.name.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        (p.group && p.group.toLowerCase().includes(q))
      ) {
        return true;
      }
      return this._routes.some((r) => r.project_id === p.id && r.host.toLowerCase().includes(q));
    });
  }

  _renderWorkloadsContentHtml(filtered) {
    const { t } = getI18n();
    if (!filtered.length) {
      return `<div class="empty-state">${esc(this._projects.length ? t('runtime.noMatch') : t('runtime.noWorkloads'))}</div>`;
    }
    const hasGroups = filtered.some((runtime) => runtime.group);
    let lastGroup;
    const workloadRows = filtered.map((p) => {
      const pRoutes = this._routes.filter((r) => r.project_id === p.id);
      const hasActive = Boolean(p.active_deployment_id);
      const group = p.group || '';
      const heading = group !== lastGroup && hasGroups
        ? `<tr class="group-heading-row"><th colspan="6">${esc(group || t('groups.ungrouped'))}</th></tr>`
        : '';
      lastGroup = group;
      return `${heading}
        <tr>
          <td class="cell-primary">
            <button type="button" class="link-pill text-strong" data-go-hash="#/runtime/${encodeURIComponent(p.id)}">${esc(p.name)}</button>
            <div class="muted mono text-xs id-sub" style="margin-top:4px;">${esc(p.id)}</div>
          </td>
          <td data-label="${esc(t('common.status'))}">
            ${hasActive
              ? `<span class="badge badge-success">${esc(t('runtime.active'))}</span>`
              : `<span class="badge badge-neutral">${esc(t('runtime.idle'))}</span>`}
          </td>
          <td data-label="${esc(t('projects.activeDeployment'))}" class="mono text-xs break-all">${esc(p.active_deployment_id || '-')}</td>
          <td data-label="${esc(t('nav.routes'))}">
            ${pRoutes.length
              ? `<div class="route-badges">${pRoutes.map((r) => `<button type="button" class="badge badge-info mono route-badge route-badge-btn" data-go-hash="#/runtime/${encodeURIComponent(p.id)}" data-open-tab="routes">${esc(r.host)}</button>`).join('')}</div>`
              : `<span class="muted text-xs">-</span>`}
          </td>
          <td data-label="${esc(t('projects.maxConcurrency'))}" class="mono text-xs">${esc(p.max_concurrency)}</td>
          <td class="cell-actions-td">
            <div class="cell-actions">
              <button type="button" class="btn btn-sm" data-go-hash="#/runtime/${encodeURIComponent(p.id)}" data-open-bindings="1">${esc(t('runtime.manageBindings'))}</button>
              <button type="button" class="btn btn-sm" data-pick-invoke="${esc(p.id)}">${esc(t('runtime.testInvoke'))}</button>
            </div>
          </td>
        </tr>
      `;
    }).join('');

    return `
      <div class="table-wrap">
        <table class="data-table stack-mobile">
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
    `;
  }

  render() {
    const { t } = getI18n();
    const profile = (this._status && this._status.profile) || 'Standard';
    const isLowMem = String(profile).toLowerCase() === 'lowmem';
    const memLimit = isLowMem ? '32 MiB / instance' : '128 MiB / instance';
    const maxConc = isLowMem ? '16 concurrent' : '64 concurrent';
    const filtered = this._getFilteredProjects();

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
            <h3 class="panel-title">${esc(t('runtime.workloads'))} <span id="workloads-count">(${filtered.length})</span></h3>
            <div class="workloads-head-tools">
              <span class="text-xs muted">${esc(t('runtime.workloadsHint'))}</span>
              <input type="search" id="runtime-search-input" value="${esc(this._search)}" placeholder="${esc(t('runtime.searchPlaceholder'))}">
            </div>
          </div>
          <div id="workloads-content">${this._renderWorkloadsContentHtml(filtered)}</div>
        </div>

        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">
              <span>${esc(t('runtime.invokeTitle'))}</span>
              <span class="badge badge-success">Live</span>
            </h3>
          </div>
          <div class="panel-body stack gap-sm">
            <form id="invoke-form" class="stack gap-sm">
              <div class="invoke-bar">
                <select id="inv-proj">
                  ${this._projects.length
                    ? this._projects.map((p) => `<option value="${esc(p.id)}" ${this._invoke.projectId === p.id ? 'selected' : ''}>${esc(p.name)}</option>`).join('')
                    : `<option value="">(${esc(t('common.empty'))})</option>`}
                </select>
                <select id="inv-method">
                  ${['GET', 'POST', 'PUT', 'PATCH', 'DELETE'].map((m) => `<option value="${m}" ${this._invoke.method === m ? 'selected' : ''}>${m}</option>`).join('')}
                </select>
                <input type="text" id="inv-path" class="mono" value="${esc(this._invoke.path)}" placeholder="/api/hello">
                <button type="submit" class="btn btn-primary btn-sm" ${this._invoke.loading ? 'disabled' : ''}>${esc(t('runtime.sendRequest'))}</button>
              </div>
              <div class="field ${['POST', 'PUT', 'PATCH'].includes(this._invoke.method) ? '' : 'hidden'}" id="inv-body-wrap">
                <label>Request Body</label>
                <textarea id="inv-body" class="mono" placeholder='{"key":"value"}'>${esc(this._invoke.body || '')}</textarea>
              </div>
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

  _bindWorkloadsEvents() {
    const content = this.shadowRoot.getElementById('workloads-content');
    if (!content) return;
    content.querySelectorAll('[data-go-hash]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const hash = btn.getAttribute('data-go-hash');
        const openTab = btn.getAttribute('data-open-tab') || (btn.getAttribute('data-open-bindings') === '1' ? 'bindings' : undefined);
        this.dispatchEvent(new CustomEvent('fp-navigate', {
          detail: { hash, projectTab: openTab },
          bubbles: true,
          composed: true
        }));
      });
    });

    content.querySelectorAll('[data-pick-invoke]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._invoke.projectId = btn.getAttribute('data-pick-invoke');
        this.render();
      });
    });
  }

  _bindEvents() {
    const sr = this.shadowRoot;
    this._bindWorkloadsEvents();

    const searchInput = sr.getElementById('runtime-search-input');
    if (searchInput) {
      searchInput.addEventListener('input', () => {
        this._search = searchInput.value;
        const filtered = this._getFilteredProjects();
        const contentEl = sr.getElementById('workloads-content');
        const countEl = sr.getElementById('workloads-count');
        if (countEl) countEl.textContent = `(${filtered.length})`;
        if (contentEl) {
          contentEl.innerHTML = this._renderWorkloadsContentHtml(filtered);
          this._bindWorkloadsEvents();
        }
      });
    }

    const form = sr.getElementById('invoke-form');
    const methodSel = sr.getElementById('inv-method');
    const bodyWrap = sr.getElementById('inv-body-wrap');
    if (methodSel && bodyWrap) {
      methodSel.addEventListener('change', () => {
        this._invoke.method = methodSel.value;
        bodyWrap.classList.toggle('hidden', !['POST', 'PUT', 'PATCH'].includes(methodSel.value));
      });
    }
    if (form) {
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._invoke.projectId = sr.getElementById('inv-proj').value;
        this._invoke.method = sr.getElementById('inv-method').value;
        this._invoke.path = sr.getElementById('inv-path').value.trim() || '/';
        const bodyEl = sr.getElementById('inv-body');
        this._invoke.body = ['POST', 'PUT', 'PATCH'].includes(this._invoke.method) && bodyEl ? bodyEl.value : '';
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
  { type: 'bucket', label: 'Object Bucket', route: '#/blobs', defaultVar: 'BUCKET' },
  { type: 'search', label: 'Search Index', route: '#/search', defaultVar: 'SEARCH' }
];

class FpProjectBindings extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [runtimeSheet];
    this._project = null;
    this._activeDeployment = null;
    this._resources = { dbs: [], kvNamespaces: [], queues: [], buckets: [], searchIndexes: [] };
    this._canWrite = false;
    this._token = '';
    this._rows = []; // [{ type, logical, target }]
    this._saving = false;
    this._targetEnv = 'prod';
    this._initializedForDep = null;
  }

  set project(p) { this._project = p; this._syncInitialRows(); if (this.isConnected) this.render(); }
  set activeDeployment(d) { this._activeDeployment = d; this._syncInitialRows(); if (this.isConnected) this.render(); }
  set resources(r) { this._resources = r || { dbs: [], kvNamespaces: [], queues: [], buckets: [], searchIndexes: [] }; if (this.isConnected) this.render(); }
  set canWrite(v) { this._canWrite = Boolean(v); if (this.isConnected) this.render(); }
  set token(v) { this._token = String(v || ''); }

  _syncInitialRows() {
    const depKey = (this._project ? this._project.id : '') + ':' + (this._activeDeployment ? this._activeDeployment.id : 'none');
    if (this._initializedForDep === depKey) return;
    this._initializedForDep = depKey;
    const b = (this._activeDeployment && this._activeDeployment.bindings) || {};
    const rows = [];
    ['db', 'kv', 'queue', 'bucket', 'search'].forEach((type) => {
      const map = b[type] || {};
      Object.keys(map).forEach((logical) => {
        rows.push({ type, logical, target: map[logical] });
      });
    });
    this._rows = rows;
  }

  _optionsForType(type) {
    const resources = {
      db: this._resources.dbs,
      kv: this._resources.kvNamespaces,
      queue: this._resources.queues,
      bucket: this._resources.buckets,
      search: this._resources.searchIndexes
    };
    return (resources[type] || [])
      .filter((resource) => this._project && resource.project_id === this._project.id)
      .map((resource) => ({ value: resource.name, label: resource.name }));
  }

  _buildBindingsObj() {
    const current = this._activeDeployment?.bindings || {};
    const out = {
      kv: {}, db: {}, queue: {}, bucket: {}, search: {},
      env: current.env || {}
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
    Object.entries(bindingsObj.search).forEach(([k, v]) => lines.push(`// search binding "${k}" -> ${v}\nlet res = search::query("${k}", "query", false, 10)?;`));
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
