// Native HTML Web Components (Custom Elements v1 + Shadow DOM + Constructable Stylesheets + <dialog> + <search>)
// Zero-build modular frontend for Ferropod Full-Text Search (SearchStore, Turso DB FTS, Daily Logs FTS).

const sharedSheet = new CSSStyleSheet();
sharedSheet.replaceSync(`
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

  .split-layout {
    display: grid;
    grid-template-columns: 270px minmax(0, 1fr);
    gap: 16px;
    align-items: start;
  }
  @media (max-width: 900px) {
    .split-layout { grid-template-columns: 1fr; }
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
  .panel-title { margin: 0; font-size: 14px; font-weight: 600; }
  .panel-body { padding: 16px; }

  .list-menu { display: flex; flex-direction: column; padding: 6px; gap: 4px; }
  .list-menu-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    padding: 9px 10px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: background 0.14s ease, border-color 0.14s ease;
  }
  .list-menu-item:hover { background: var(--surface-subtle); }
  .list-menu-item.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 2px;
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
  .btn-sm { height: 30px; padding: 0 10px; font-size: 12px; }
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
  .btn-ghost { background: transparent; border-color: transparent; color: var(--text-secondary); }
  .btn-ghost:hover:not(:disabled) { background: var(--surface-subtle); color: var(--text); }

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
  .badge-info { background: var(--info-soft); color: var(--info); }
  .badge-success { background: var(--success-soft); color: var(--success); }
  .badge-warn { background: var(--warn-soft); color: var(--warn); }
  .badge-danger { background: var(--danger-soft); color: var(--danger); }

  input[type="text"], input[type="search"], input[type="number"], input:not([type]), select, textarea {
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
  select {
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
  select:hover:not(:disabled) { border-color: var(--border-strong); }
  select option, select optgroup { background-color: var(--surface); color: var(--text); }
  input:disabled, select:disabled, textarea:disabled {
    opacity: 0.6;
    cursor: not-allowed;
    background-color: var(--surface-subtle);
  }
  input:focus, select:focus, textarea:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  textarea { min-height: 110px; resize: vertical; }

  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }

  search { display: block; }
  .search-bar {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .search-bar input[type="search"],
  .search-bar input[type="text"] {
    flex: 1 1 240px;
  }
  .search-bar select { width: auto; }
  .panel-header select {
    min-height: 30px;
    height: 30px;
    padding: 0 28px 0 10px;
    font-size: 12px;
    background-position: right 8px center;
  }
  .panel-header input[type="search"],
  .panel-header input[type="text"] {
    min-height: 30px;
    height: 30px;
    padding: 0 10px;
    font-size: 12px;
  }

  .checkbox-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-subtle);
    font-size: 12px;
    cursor: pointer;
    user-select: none;
  }
  .checkbox-pill input { width: auto; margin: 0; accent-color: var(--accent); }

  .facet-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .facet-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-subtle);
    color: var(--text-secondary);
    font-size: 12px;
    cursor: pointer;
    transition: all 0.14s ease;
  }
  .facet-chip:hover { border-color: var(--accent); color: var(--text); }
  .facet-chip.active {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
    font-weight: 600;
  }

  .result-list { display: flex; flex-direction: column; gap: 10px; }
  .result-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    background: var(--surface);
    transition: border-color 0.14s ease;
  }
  .result-card:hover { border-color: var(--border-strong); }
  .result-title { font-weight: 600; font-size: 14px; margin: 4px 0; }
  .result-body { font-size: 13px; color: var(--text-secondary); line-height: 1.6; }

  mark,
  mark.fts-hl {
    background: color-mix(in srgb, var(--accent) 24%, transparent);
    color: var(--text);
    padding: 0 3px;
    border-radius: 4px;
    font-weight: 600;
  }

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: 12px;
  }
  .stat-card {
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .stat-label { font-size: 12px; color: var(--muted); }
  .stat-value { font-size: 18px; font-weight: 700; }

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
    padding: 9px 12px;
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

  .empty-state {
    padding: 28px 16px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }

  .log-list {
    max-height: 540px;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
  }
  .log-row {
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .log-row:last-child { border-bottom: none; }
  .log-row-head { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; font-size: 12px; }
  .log-msg { font-family: var(--font-mono); font-size: 12.5px; word-break: break-word; }

  /* Native HTML <dialog> modal */
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
  dialog.fp-dialog::backdrop {
    background: rgba(15, 23, 42, 0.55);
    backdrop-filter: blur(2px);
  }
  .dialog-head {
    padding: 14px 16px;
    border-bottom: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .dialog-title { margin: 0; font-size: 15px; font-weight: 600; }
  .dialog-body { padding: 16px; overflow-y: auto; max-height: 65vh; }
  .dialog-foot {
    padding: 12px 16px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    background: var(--surface-subtle);
  }
  @media (max-width: 639px) {
    .panel-header, .panel-body { padding: 12px 14px; }
    input[type="text"], input[type="search"], input[type="number"], input:not([type]), select, textarea,
    .panel-header select, .panel-header input[type="search"], .panel-header input[type="text"] {
      height: auto;
      font-size: 16px;
      min-height: 42px;
      padding: 7px 11px;
    }
    select, .panel-header select { padding-right: 32px; background-position: right 10px center; }
    .btn { min-height: 42px; height: auto; }
    .btn-sm { min-height: 36px; height: auto; }
    .search-bar { flex-direction: column; align-items: stretch; }
    .search-bar input[type="search"], .search-bar input[type="text"], .search-bar .btn { width: 100%; flex: 0 0 auto; }
    .row select, .row input[type="text"] {
      flex: 1 1 calc(50% - 6px);
      width: auto !important;
      max-width: none !important;
      min-width: 0;
    }
    dialog.fp-dialog {
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

function getI18n() {
  return window.FerroI18n || {
    t: (k) => k,
    formatTime: (v) => String(v || ''),
    formatBytes: (b) => `${b || 0} B`
  };
}

function highlightText(text, query) {
  const str = String(text ?? '');
  const terms = String(query || '').trim().split(/\s+/).filter(Boolean);
  if (!terms.length) return esc(str);
  const escaped = terms
    .slice()
    .sort((a, b) => b.length - a.length)
    .map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'));
  const re = new RegExp('(' + escaped.join('|') + ')', 'gi');
  return str
    .split(re)
    .map((part, idx) => (idx % 2 === 1 ? '<mark class="fts-hl">' + esc(part) + '</mark>' : esc(part)))
    .join('');
}

function emitToast(el, message, kind = 'info') {
  el.dispatchEvent(new CustomEvent('fp-toast', {
    detail: { message, kind },
    bubbles: true,
    composed: true
  }));
}

// ============================================================================
// 3. <fp-log-search-panel> — Daily Logs Full-Text Search Component
// ============================================================================

class FpLogSearchPanel extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [sharedSheet];
    this._projects = [];
    this._liveEntries = [];
    this._scope = 'admin';
    this._filters = {
      q: '',
      source: '',
      level: '',
      project_id: '',
      request_id: '',
      windowSecs: '86400',
      limit: '100'
    };
    this._onLangChange = () => { if (this.isConnected) this.render(); };
  }

  set projects(list) {
    this._projects = Array.isArray(list) ? list : [];
    if (this.isConnected) this.render();
  }

  set liveEntries(list) {
    this._liveEntries = Array.isArray(list) ? list : [];
    if (this.isConnected) this.render();
  }

  set filters(next) {
    const scopePid = this._scope.startsWith('project:') ? this._scope.slice('project:'.length) : '';
    this._filters = {
      q: '',
      source: '',
      level: '',
      project_id: scopePid,
      request_id: '',
      windowSecs: '86400',
      limit: '100',
      ...(next || {})
    };
    if (scopePid) this._filters.project_id = scopePid;
    if (this.isConnected) this.render();
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (this._scope.startsWith('project:')) {
      this._filters.project_id = this._scope.slice('project:'.length);
    }
    if (this.isConnected) this.render();
  }

  connectedCallback() {
    window.addEventListener('fp-lang-change', this._onLangChange);
    this.render();
  }

  disconnectedCallback() {
    window.removeEventListener('fp-lang-change', this._onLangChange);
  }

  _levelBadge(level) {
    if (level === 'error') return `<span class="badge badge-danger">${esc(level)}</span>`;
    if (level === 'warn') return `<span class="badge badge-warn">${esc(level)}</span>`;
    if (level === 'debug') return `<span class="badge badge-neutral">${esc(level)}</span>`;
    return `<span class="badge badge-info">${esc(level || 'info')}</span>`;
  }

  _projectName(pid) {
    const p = this._projects.find((item) => item.id === pid);
    return p ? p.name : pid;
  }

  render() {
    const { t, formatTime } = getI18n();
    const f = this._filters;

    const scopePid = this._scope.startsWith('project:') ? this._scope.slice('project:'.length) : '';
    const terms = f.q.trim().toLowerCase().split(/\s+/).filter(Boolean);
    const entries = this._liveEntries.filter((e) => {
      const fields = [e.message, e.meta_json, e.request_id]
        .map((value) => String(value || '').toLowerCase());
      return terms.every((term) => fields.some((value) => value.includes(term)));
    });
    const summaryText = `${entries.length} · ${t('logs.ftsLiveMode')}`;

    const rowsHtml = entries.length
      ? entries.map((e) => `
          <div class="log-row">
            <div class="log-row-head">
              <span class="muted mono">${esc(formatTime(e.ts))}</span>
              ${this._levelBadge(e.level)}
              <span class="badge badge-neutral">${esc(e.source)}</span>
              ${e.project_id ? `<span class="badge badge-accent">${esc(this._projectName(e.project_id))}</span>` : ''}
              ${e.request_id ? `<span class="mono muted">${esc(e.request_id)}</span>` : ''}
            </div>
            <div class="log-msg">${highlightText(e.message, f.q)}</div>
            ${e.meta_json ? `
              <details>
                <summary class="muted text-xs" style="cursor:pointer;">${esc(t('logs.meta'))}</summary>
                <pre class="code-block" style="margin-top:6px;">${highlightText(e.meta_json, f.q)}</pre>
              </details>
            ` : ''}
          </div>
        `).join('')
      : `<div class="empty-state">${esc(t('logs.noLogs'))}</div>`;

    this.shadowRoot.innerHTML = `
      <div class="panel">
        <div class="panel-header" style="display:block;">
          <search>
            <form id="log-fts-form" class="stack gap-sm">
              <div class="search-bar">
                <input type="search" id="lf-query" value="${esc(f.q)}" placeholder="${esc(t('logs.ftsPlaceholder'))}">
                <button type="submit" class="btn btn-primary btn-sm">${esc(t('logs.queryBtn'))}</button>
              </div>
              <div class="row gap-xs">
                <select id="lf-source" style="width:auto;">
                  <option value="">${esc(t('logs.allSources'))}</option>
                  ${['access', 'runtime', 'queue', 'cron', 'audit', 'system'].map((s) =>
                    `<option value="${s}" ${f.source === s ? 'selected' : ''}>${s}</option>`
                  ).join('')}
                </select>
                <select id="lf-level" style="width:auto;">
                  <option value="">${esc(t('logs.allLevels'))}</option>
                  ${['info', 'warn', 'error', 'debug'].map((l) =>
                    `<option value="${l}" ${f.level === l ? 'selected' : ''}>${l}</option>`
                  ).join('')}
                </select>
                <select id="lf-project" style="width:auto;max-width:190px;" ${scopePid ? 'disabled' : ''}>
                  ${scopePid
                    ? `<option value="${esc(scopePid)}" selected>${esc(this._projectName(scopePid))}</option>`
                    : `<option value="">${esc(t('common.all'))} (${esc(t('common.project'))})</option>` +
                      this._projects.map((p) =>
                        `<option value="${esc(p.id)}" ${f.project_id === p.id ? 'selected' : ''}>${esc(p.name)}</option>`
                      ).join('')
                  }
                </select>
                <input type="text" id="lf-reqid" style="max-width:170px;" value="${esc(f.request_id)}" placeholder="${esc(t('logs.requestIdPlaceholder'))}">
                <select id="lf-window" style="width:auto;">
                  <option value="900" ${f.windowSecs === '900' ? 'selected' : ''}>${esc(t('logs.window15m'))}</option>
                  <option value="3600" ${f.windowSecs === '3600' ? 'selected' : ''}>${esc(t('logs.window1h'))}</option>
                  <option value="86400" ${f.windowSecs === '86400' ? 'selected' : ''}>${esc(t('logs.window24h'))}</option>
                  <option value="604800" ${f.windowSecs === '604800' ? 'selected' : ''}>${esc(t('logs.window7d'))}</option>
                  <option value="" ${f.windowSecs === '' ? 'selected' : ''}>${esc(t('logs.windowAll'))}</option>
                </select>
                <select id="lf-limit" style="width:auto;">
                  ${['50', '100', '200', '500'].map((n) =>
                    `<option value="${n}" ${f.limit === n ? 'selected' : ''}>${n}</option>`
                  ).join('')}
                </select>
              </div>
              <div class="row-between text-xs muted">
                <span>${esc(summaryText)}</span>
                <span class="badge badge-success">${esc(t('logs.ftsLiveMode'))}</span>
              </div>
            </form>
          </search>
        </div>
        <div class="log-list">${rowsHtml}</div>
      </div>
    `;

    this._bindEvents();
  }

  _syncFiltersFromDom() {
    const root = this.shadowRoot;
    this._filters = {
      q: root.getElementById('lf-query').value,
      source: root.getElementById('lf-source').value,
      level: root.getElementById('lf-level').value,
      project_id: root.getElementById('lf-project').value,
      request_id: root.getElementById('lf-reqid').value.trim(),
      windowSecs: root.getElementById('lf-window').value,
      limit: root.getElementById('lf-limit').value
    };
  }

  _bindEvents() {
    const root = this.shadowRoot;
    const form = root.getElementById('log-fts-form');
    const emitQuery = () => {
      this._syncFiltersFromDom();
      this.dispatchEvent(new CustomEvent('fp-log-query', {
        detail: { ...this._filters },
        bubbles: true,
        composed: true
      }));
      this.render();
    };
    if (form) {
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        emitQuery();
      });
    }
    ['lf-source', 'lf-level', 'lf-project', 'lf-window', 'lf-limit'].forEach((id) => {
      const sel = root.getElementById(id);
      if (sel) sel.addEventListener('change', emitQuery);
    });
  }
}

if (!customElements.get('fp-log-search-panel')) {
  customElements.define('fp-log-search-panel', FpLogSearchPanel);
}
