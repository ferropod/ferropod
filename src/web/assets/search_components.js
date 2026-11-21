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
    padding: 7px 12px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: background 0.14s ease, border-color 0.14s ease;
  }
  .btn:hover { background: var(--surface-subtle); border-color: var(--border-strong); }
  .btn-sm { padding: 5px 10px; font-size: 12px; }
  .btn-primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .btn-primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); }
  .btn-danger {
    color: var(--danger);
    border-color: color-mix(in srgb, var(--danger) 35%, var(--border));
    background: var(--danger-soft);
  }
  .btn-danger:hover { border-color: var(--danger); }
  .btn-ghost { background: transparent; border-color: transparent; color: var(--text-secondary); }
  .btn-ghost:hover { background: var(--surface-subtle); color: var(--text); }

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

  input[type="text"], input[type="search"], select, textarea {
    width: 100%;
    padding: 8px 11px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    outline: none;
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
    divide-y: 1px solid var(--border);
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

function getMock() {
  return window.FerroSearchMock;
}

function emitToast(el, message, kind = 'info') {
  el.dispatchEvent(new CustomEvent('fp-toast', {
    detail: { message, kind },
    bubbles: true,
    composed: true
  }));
}

// ============================================================================
// 1. <fp-search-workbench> — Standalone SearchStore (Tantivy) Console
// ============================================================================

class FpSearchWorkbench extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [sharedSheet];
    this._projects = [];
    this._scope = 'admin';
    this._state = {
      selectedIndexId: '',
      projectFilter: '',
      tab: 'playground', // playground | docs | schema
      query: '',
      fuzzy: true,
      sort: 'relevance',
      facets: {}
    };
  }

  connectedCallback() {
    const indexes = getMock().listIndexes(this._scope);
    if (!this._state.selectedIndexId && indexes.length) {
      this._state.selectedIndexId = indexes[0].id;
    }
    this.render();
  }

  set canWrite(val) {
    this._canWrite = Boolean(val);
    if (this.isConnected) this.render();
  }
  get canWrite() {
    return this._canWrite !== false;
  }

  set projects(list) {
    this._projects = Array.isArray(list) ? list : [];
    if (this.isConnected) this.render();
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (this.isConnected) this.render();
  }

  _scopeProjectId() {
    return this._scope.startsWith('project:') ? this._scope.slice('project:'.length) : '';
  }

  _projectName(pid) {
    if (!pid) return '';
    const found = this._projects.find((p) => p.id === pid);
    return found ? found.name : pid;
  }

  _canWriteIndex(idx) {
    if (!this.canWrite || this._scope === 'readonly') return false;
    if (this._scope === 'admin') return true;
    const spid = this._scopeProjectId();
    return Boolean(spid && idx && idx.project_id === spid);
  }

  render() {
    const { t, formatTime, formatBytes } = getI18n();
    const mock = getMock();
    const scopePid = this._scopeProjectId();
    const allVisible = mock.listIndexes(this._scope);
    const indexes = !scopePid && this._state.projectFilter
      ? allVisible.filter((i) =>
          this._state.projectFilter === '__global__'
            ? !i.project_id
            : i.project_id === this._state.projectFilter
        )
      : allVisible;

    if (!indexes.some((i) => i.id === this._state.selectedIndexId)) {
      this._state.selectedIndexId = indexes.length ? indexes[0].id : '';
      this._state.facets = {};
    }
    const activeIdx = indexes.find((i) => i.id === this._state.selectedIndexId) || null;
    const canWriteActive = this._canWriteIndex(activeIdx);

    const projectFilterHtml = !scopePid && this._projects.length ? `
      <div style="padding:8px 10px;border-bottom:1px solid var(--border);">
        <select id="project-filter-select" class="text-xs">
          <option value="">${esc(t('common.all'))} (${esc(t('common.project'))})</option>
          <option value="__global__" ${this._state.projectFilter === '__global__' ? 'selected' : ''}>${esc(t('common.allProjects'))}</option>
          ${this._projects.map((p) =>
            `<option value="${esc(p.id)}" ${this._state.projectFilter === p.id ? 'selected' : ''}>${esc(p.name)}</option>`
          ).join('')}
        </select>
      </div>
    ` : '';

    const indexMenuHtml = indexes.length
      ? indexes.map((idx) => `
          <button type="button" class="list-menu-item ${idx.id === this._state.selectedIndexId ? 'active' : ''}" data-select-index="${esc(idx.id)}">
            <span class="stack" style="min-width:0;gap:2px;">
              <span class="truncate mono">${esc(idx.name)}</span>
              ${idx.project_id ? `<span class="text-xs muted truncate">${esc(this._projectName(idx.project_id))}</span>` : ''}
            </span>
            <span class="row gap-xs" style="flex-shrink:0;">
              <span class="badge badge-neutral">${esc(idx.tokenizer)}</span>
              <span class="badge badge-accent">${idx.doc_count}</span>
            </span>
          </button>
        `).join('')
      : `<div class="empty-state">${esc(t('search.noIndexes'))}</div>`;

    let rightHtml = `<div class="panel"><div class="empty-state">${esc(t('search.selectIndex'))}</div></div>`;
    if (activeIdx) {
      const tabsHtml = `
        <div class="tabs" role="tablist">
          <button type="button" role="tab" class="tab-btn ${this._state.tab === 'playground' ? 'active' : ''}" data-tab="playground">${esc(t('search.tabPlayground'))}</button>
          <button type="button" role="tab" class="tab-btn ${this._state.tab === 'docs' ? 'active' : ''}" data-tab="docs">${esc(t('search.tabDocs'))} (${activeIdx.doc_count})</button>
          <button type="button" role="tab" class="tab-btn ${this._state.tab === 'schema' ? 'active' : ''}" data-tab="schema">${esc(t('search.tabSchema'))}</button>
        </div>
      `;

      let tabBody = '';
      if (this._state.tab === 'playground') {
        const res = mock.search(activeIdx.id, {
          query: this._state.query,
          fuzzy: this._state.fuzzy,
          sort: this._state.sort,
          facets: this._state.facets,
          limit: 30
        });

        const facetEntries = Object.entries(res.facets || {});
        const hasActiveFacets = Object.values(this._state.facets).some(Boolean);
        const facetsHtml = facetEntries.length ? `
          <div class="facet-bar">
            ${facetEntries.map(([field, counts]) =>
              Object.entries(counts).map(([val, cnt]) => {
                const isActive = this._state.facets[field] === val;
                return `<button type="button" class="facet-chip ${isActive ? 'active' : ''}" data-facet-field="${esc(field)}" data-facet-val="${esc(val)}">${esc(field)}: ${esc(val)} <span>(${cnt})</span></button>`;
              }).join('')
            ).join('')}
            ${hasActiveFacets ? `<button type="button" class="btn btn-ghost btn-sm" id="clear-facets-btn">${esc(t('search.clearFacets'))}</button>` : ''}
          </div>
        ` : '';

        const hitsHtml = res.hits.length
          ? `<div class="result-list">
              ${res.hits.map(({ doc, score }) => {
                const titleHtml = mock.highlightText(doc.title || doc.id, this._state.query);
                const bodyHtml = mock.highlightText(doc.body || doc.description || '', this._state.query);
                const tagBadges = (Array.isArray(doc.tags) ? doc.tags : []).map((tg) =>
                  `<span class="badge badge-neutral">#${esc(tg)}</span>`
                ).join('');
                return `
                  <article class="result-card">
                    <div class="row-between">
                      <div class="row gap-xs">
                        <span class="mono text-xs muted">${esc(doc.id)}</span>
                        ${doc.category ? `<span class="badge badge-info">${esc(doc.category)}</span>` : ''}
                        ${doc.lang ? `<span class="badge badge-neutral">${esc(doc.lang)}</span>` : ''}
                        ${doc.price_cny !== undefined ? `<span class="badge badge-warn">¥${esc(doc.price_cny)}</span>` : ''}
                      </div>
                      <div class="row gap-xs">
                        <span class="badge badge-accent mono">${esc(t('search.score'))} ${score.toFixed(2)}</span>
                        ${canWriteActive ? `<button type="button" class="btn btn-ghost btn-sm" data-del-doc="${esc(doc.id)}">${esc(t('common.delete'))}</button>` : ''}
                      </div>
                    </div>
                    <div class="result-title">${titleHtml}</div>
                    <div class="result-body">${bodyHtml}</div>
                    <div class="row-between" style="margin-top:8px;">
                      <div class="row gap-xs">${tagBadges}</div>
                      <span class="muted text-xs">${esc(formatTime(doc.updated_at))}</span>
                    </div>
                    <details style="margin-top:8px;">
                      <summary class="muted text-xs" style="cursor:pointer;">${esc(t('search.rawJson'))}</summary>
                      <pre class="code-block" style="margin-top:6px;">${esc(JSON.stringify(doc, null, 2))}</pre>
                    </details>
                  </article>
                `;
              }).join('')}
            </div>`
          : `<div class="empty-state">${esc(t('search.noHits'))}</div>`;

        tabBody = `
          <div class="panel">
            <div class="panel-body stack gap-md">
              <search>
                <form id="search-form" class="search-bar">
                  <input type="search" id="search-input" value="${esc(this._state.query)}" placeholder="${esc(t('search.queryPlaceholder'))}">
                  <label class="checkbox-pill">
                    <input type="checkbox" id="fuzzy-toggle" ${this._state.fuzzy ? 'checked' : ''}>
                    <span>${esc(t('search.fuzzy'))}</span>
                  </label>
                  <select id="sort-select">
                    <option value="relevance" ${this._state.sort === 'relevance' ? 'selected' : ''}>${esc(t('search.sortRelevance'))}</option>
                    <option value="newest" ${this._state.sort === 'newest' ? 'selected' : ''}>${esc(t('search.sortNewest'))}</option>
                  </select>
                  <button type="submit" class="btn btn-primary">${esc(t('search.searchBtn'))}</button>
                </form>
              </search>
              ${facetsHtml}
              <div class="row-between text-xs muted">
                <span>${esc(t('search.hitsSummary', { total: res.total, ms: res.took_ms, tokenizer: res.tokenizer }))}</span>
                <span class="mono">/data/search/${esc(activeIdx.project_id || '_global')}/${esc(activeIdx.id)}/</span>
              </div>
              ${hitsHtml}
            </div>
          </div>
        `;
      } else if (this._state.tab === 'docs') {
        const docs = mock.listDocs(activeIdx.id);
        tabBody = `
          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title mono">${esc(activeIdx.name)}</h3>
              <div class="row gap-xs">
                ${canWriteActive ? `<button type="button" class="btn btn-sm" id="commit-btn">${esc(t('search.commitOptimize'))}</button>` : ''}
                ${canWriteActive ? `<button type="button" class="btn btn-primary btn-sm" id="upsert-doc-btn">${esc(t('search.upsertDoc'))}</button>` : ''}
              </div>
            </div>
            ${docs.length ? `
              <div class="table-wrap">
                <table class="data-table">
                  <thead>
                    <tr>
                      <th>ID</th>
                      <th>Title</th>
                      <th>Facet / Tags</th>
                      <th>${esc(t('common.updatedAt'))}</th>
                      ${canWriteActive ? `<th>${esc(t('common.actions'))}</th>` : ''}
                    </tr>
                  </thead>
                  <tbody>
                    ${docs.map((d) => `
                      <tr>
                        <td class="mono text-xs">${esc(d.id)}</td>
                        <td>
                          <div class="text-strong">${esc(d.title || '-')}</div>
                          <div class="muted text-xs truncate" style="max-width:360px;">${esc(d.body || '')}</div>
                        </td>
                        <td>
                          ${d.category ? `<span class="badge badge-info">${esc(d.category)}</span>` : ''}
                          ${d.lang ? `<span class="badge badge-neutral">${esc(d.lang)}</span>` : ''}
                        </td>
                        <td class="muted text-xs">${esc(formatTime(d.updated_at))}</td>
                        ${canWriteActive ? `<td><button type="button" class="btn btn-danger btn-sm" data-del-doc="${esc(d.id)}">${esc(t('common.delete'))}</button></td>` : ''}
                      </tr>
                    `).join('')}
                  </tbody>
                </table>
              </div>
            ` : `<div class="empty-state">${esc(t('common.empty'))}</div>`}
          </div>
        `;
      } else {
        tabBody = `
          <div class="stack gap-md">
            <div class="stats-grid">
              <div class="stat-card"><span class="stat-label">${esc(t('search.statDocs'))}</span><span class="stat-value">${activeIdx.doc_count}</span></div>
              <div class="stat-card"><span class="stat-label">${esc(t('search.statSegments'))}</span><span class="stat-value">${activeIdx.segments}</span></div>
              <div class="stat-card"><span class="stat-label">${esc(t('search.statDisk'))}</span><span class="stat-value">${esc(formatBytes(activeIdx.disk_bytes))}</span></div>
              <div class="stat-card"><span class="stat-label">${esc(t('search.statBudget'))}</span><span class="stat-value">${activeIdx.writer_budget_mb} MiB</span></div>
            </div>
            <div class="panel">
              <div class="panel-header">
                <h3 class="panel-title">Schema (${esc(activeIdx.tokenizer)}) ${activeIdx.project_id ? `<span class="badge badge-accent">${esc(this._projectName(activeIdx.project_id))}</span>` : `<span class="badge badge-neutral">${esc(t('common.allProjects'))}</span>`}</h3>
                ${canWriteActive ? `<button type="button" class="btn btn-danger btn-sm" id="delete-index-btn">${esc(t('search.deleteIndex'))}</button>` : ''}
              </div>
              <div class="table-wrap">
                <table class="data-table">
                  <thead>
                    <tr>
                      <th>${esc(t('search.fieldName'))}</th>
                      <th>${esc(t('search.fieldType'))}</th>
                      <th>${esc(t('search.tokenizer'))}</th>
                      <th>${esc(t('search.fieldOptions'))}</th>
                    </tr>
                  </thead>
                  <tbody>
                    ${(activeIdx.schema || []).map((f) => `
                      <tr>
                        <td class="mono text-strong">${esc(f.name)}</td>
                        <td><span class="badge badge-info">${esc(f.type)}</span></td>
                        <td class="mono text-xs">${esc(f.tokenizer)}</td>
                        <td class="row gap-xs">
                          ${f.stored ? '<span class="badge badge-neutral">stored</span>' : ''}
                          ${f.indexed ? '<span class="badge badge-success">indexed</span>' : ''}
                          ${f.fast ? '<span class="badge badge-accent">fast</span>' : ''}
                        </td>
                      </tr>
                    `).join('')}
                  </tbody>
                </table>
              </div>
            </div>
          </div>
        `;
      }

      rightHtml = `<div class="stack gap-md">${tabsHtml}${tabBody}</div>`;
    }

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">
            ${esc(t('search.title'))}
            <span class="badge badge-warn">${esc(t('search.mockBadge'))}</span>
          </h2>
          <p class="page-subtitle">${esc(t('search.subtitle'))}</p>
        </div>
        ${this.canWrite ? `<button type="button" class="btn btn-primary" id="new-index-btn">${esc(t('search.newIndex'))}</button>` : ''}
      </div>

      <div class="split-layout">
        <div class="panel">
          <div class="panel-header"><h3 class="panel-title">${esc(t('search.indexes'))}</h3></div>
          ${projectFilterHtml}
          <div class="list-menu">${indexMenuHtml}</div>
        </div>
        ${rightHtml}
      </div>

      <dialog id="create-index-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('search.newIndex'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dialog="create-index-dialog">✕</button>
        </div>
        <form id="create-index-form">
          <div class="dialog-body stack gap-md">
            <div class="field">
              <label for="ci-name">${esc(t('search.indexName'))}</label>
              <input type="text" id="ci-name" required placeholder="${esc(t('search.indexNamePlaceholder'))}">
            </div>
            <div class="field">
              <label for="ci-project">${esc(t('common.project'))}</label>
              <select id="ci-project" ${scopePid ? 'disabled' : ''}>
                ${scopePid
                  ? `<option value="${esc(scopePid)}" selected>${esc(this._projectName(scopePid))}</option>`
                  : `<option value="">${esc(t('common.allProjects'))}</option>` +
                    this._projects.map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join('')
                }
              </select>
            </div>
            <div class="field">
              <label for="ci-tokenizer">${esc(t('search.tokenizer'))}</label>
              <select id="ci-tokenizer">
                <option value="jieba">jieba (中文词典分词)</option>
                <option value="jieba+ngram">jieba + ngram (中英文混合 + 前缀)</option>
                <option value="ngram">ngram (2-gram 通用子串)</option>
                <option value="simple">simple (英文/拉丁标准分词)</option>
              </select>
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dialog="create-index-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.create'))}</button>
          </div>
        </form>
      </dialog>

      <dialog id="upsert-doc-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('search.upsertDoc'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" data-close-dialog="upsert-doc-dialog">✕</button>
        </div>
        <form id="upsert-doc-form">
          <div class="dialog-body stack gap-md">
            <div class="field">
              <label for="ud-json">${esc(t('search.docJsonLabel'))}</label>
              <textarea id="ud-json" class="mono" style="min-height:190px;">${esc(JSON.stringify({
                id: 'doc-' + String(Date.now()).slice(-4),
                title: '新建示例文档',
                body: '这是一条写入 Tantivy 索引的测试文档，支持中文分词与高亮。',
                category: 'guide',
                lang: 'zh',
                tags: ['demo', 'search']
              }, null, 2))}</textarea>
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" data-close-dialog="upsert-doc-dialog">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.save'))}</button>
          </div>
        </form>
      </dialog>
    `;

    this._bindEvents(activeIdx);
  }

  _bindEvents(activeIdx) {
    const root = this.shadowRoot;
    const { t } = getI18n();
    const mock = getMock();

    root.querySelectorAll('[data-select-index]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._state.selectedIndexId = btn.getAttribute('data-select-index');
        this._state.facets = {};
        this.render();
      });
    });

    root.querySelectorAll('[data-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._state.tab = btn.getAttribute('data-tab');
        this.render();
      });
    });

    root.querySelectorAll('[data-close-dialog]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const dlg = root.getElementById(btn.getAttribute('data-close-dialog'));
        if (dlg && dlg.open) dlg.close();
      });
    });

    const newIdxBtn = root.getElementById('new-index-btn');
    if (newIdxBtn) {
      newIdxBtn.addEventListener('click', () => {
        root.getElementById('create-index-dialog').showModal();
      });
    }

    const projFilter = root.getElementById('project-filter-select');
    if (projFilter) {
      projFilter.addEventListener('change', (ev) => {
        this._state.projectFilter = ev.target.value;
        this.render();
      });
    }

    const createForm = root.getElementById('create-index-form');
    if (createForm) {
      createForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        try {
          const projEl = root.getElementById('ci-project');
          const created = mock.createIndex({
            name: root.getElementById('ci-name').value.trim(),
            project_id: projEl ? (projEl.value || null) : null,
            tokenizer: root.getElementById('ci-tokenizer').value
          }, this._scope);
          root.getElementById('create-index-dialog').close();
          this._state.selectedIndexId = created.id;
          this._state.tab = 'playground';
          this._state.facets = {};
          emitToast(this, t('search.indexCreated'), 'success');
          this.render();
        } catch (err) {
          emitToast(this, err.message, 'error');
        }
      });
    }

    const searchForm = root.getElementById('search-form');
    if (searchForm) {
      searchForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._state.query = root.getElementById('search-input').value;
        this.render();
      });
    }
    const searchInput = root.getElementById('search-input');
    if (searchInput) {
      searchInput.addEventListener('input', (ev) => {
        this._state.query = ev.target.value;
      });
    }
    const fuzzyToggle = root.getElementById('fuzzy-toggle');
    if (fuzzyToggle) {
      fuzzyToggle.addEventListener('change', (ev) => {
        this._state.fuzzy = ev.target.checked;
        this.render();
      });
    }
    const sortSel = root.getElementById('sort-select');
    if (sortSel) {
      sortSel.addEventListener('change', (ev) => {
        this._state.sort = ev.target.value;
        this.render();
      });
    }

    root.querySelectorAll('[data-facet-field]').forEach((chip) => {
      chip.addEventListener('click', () => {
        const field = chip.getAttribute('data-facet-field');
        const val = chip.getAttribute('data-facet-val');
        if (this._state.facets[field] === val) {
          delete this._state.facets[field];
        } else {
          this._state.facets[field] = val;
        }
        this.render();
      });
    });

    const clearFacets = root.getElementById('clear-facets-btn');
    if (clearFacets) {
      clearFacets.addEventListener('click', () => {
        this._state.facets = {};
        this.render();
      });
    }

    const upsertBtn = root.getElementById('upsert-doc-btn');
    if (upsertBtn) {
      upsertBtn.addEventListener('click', () => {
        root.getElementById('upsert-doc-dialog').showModal();
      });
    }

    const upsertForm = root.getElementById('upsert-doc-form');
    if (upsertForm && activeIdx) {
      upsertForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        try {
          const parsed = JSON.parse(root.getElementById('ud-json').value);
          mock.upsertDoc(activeIdx.id, parsed);
          root.getElementById('upsert-doc-dialog').close();
          emitToast(this, t('search.docSaved'), 'success');
          this.render();
        } catch (err) {
          emitToast(this, t('search.invalidJson'), 'error');
        }
      });
    }

    const commitBtn = root.getElementById('commit-btn');
    if (commitBtn && activeIdx) {
      commitBtn.addEventListener('click', () => {
        mock.commitIndex(activeIdx.id);
        emitToast(this, t('search.committed'), 'info');
        this.render();
      });
    }

    root.querySelectorAll('[data-del-doc]').forEach((btn) => {
      btn.addEventListener('click', () => {
        if (!activeIdx) return;
        mock.deleteDoc(activeIdx.id, btn.getAttribute('data-del-doc'));
        emitToast(this, t('search.docDeleted'), 'success');
        this.render();
      });
    });

    const delIdxBtn = root.getElementById('delete-index-btn');
    if (delIdxBtn && activeIdx) {
      delIdxBtn.addEventListener('click', () => {
        if (window.confirm(t('search.deleteIndexConfirm', { name: activeIdx.name }))) {
          mock.deleteIndex(activeIdx.id);
          emitToast(this, t('search.indexDeleted'), 'success');
          this.render();
        }
      });
    }
  }
}

// ============================================================================
// 2. <fp-db-fts-panel> — Turso DB Built-in FTS Tab Component
// ============================================================================

class FpDbFtsPanel extends HTMLElement {
  static get observedAttributes() {
    return ['db-id', 'db-name', 'can-write'];
  }

  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [sharedSheet];
    this._selectedFts = '';
    this._query = 'MVCC';
  }

  connectedCallback() {
    this.render();
  }

  attributeChangedCallback() {
    if (this.isConnected) this.render();
  }

  render() {
    const { t, formatBytes } = getI18n();
    const mock = getMock();
    const dbId = this.getAttribute('db-id') || 'default';
    const dbName = this.getAttribute('db-name') || dbId;
    const canWrite = this.getAttribute('can-write') === 'true';

    const indexes = mock.listDbFtsIndexes(dbId);
    if (!indexes.some((i) => i.name === this._selectedFts)) {
      this._selectedFts = indexes.length ? indexes[0].name : '';
    }

    const testRes = this._selectedFts ? mock.queryDbFts(dbId, this._selectedFts, this._query) : null;

    this.shadowRoot.innerHTML = `
      <div class="stack gap-md">
        <div class="panel">
          <div class="panel-header">
            <div>
              <h3 class="panel-title row gap-xs">
                <span class="mono">${esc(dbName)}</span>
                <span class="badge badge-warn">${esc(t('search.mockBadge'))}</span>
              </h3>
              <div class="muted text-xs" style="margin-top:3px;">${esc(t('dbs.ftsSubtitle'))}</div>
            </div>
            ${canWrite ? `<button type="button" class="btn btn-primary btn-sm" id="new-fts-btn">${esc(t('dbs.ftsNewIndex'))}</button>` : ''}
          </div>
          ${indexes.length ? `
            <div class="table-wrap">
              <table class="data-table">
                <thead>
                  <tr>
                    <th>${esc(t('dbs.ftsIndexName'))}</th>
                    <th>${esc(t('dbs.ftsTable'))}</th>
                    <th>${esc(t('dbs.ftsTokenizer'))}</th>
                    <th>${esc(t('dbs.ftsRowsIndexed'))}</th>
                    <th>${esc(t('common.actions'))}</th>
                  </tr>
                </thead>
                <tbody>
                  ${indexes.map((idx) => `
                    <tr>
                      <td class="mono text-strong">${esc(idx.name)}</td>
                      <td class="mono text-xs">${esc(idx.table)}(${esc(idx.columns.join(', '))})</td>
                      <td><span class="badge badge-info">${esc(idx.tokenizer)}</span></td>
                      <td class="text-xs">${idx.rows_indexed} <span class="muted">(${esc(formatBytes(idx.size_bytes))})</span></td>
                      <td class="row gap-xs">
                        <button type="button" class="btn btn-sm ${this._selectedFts === idx.name ? 'btn-primary' : ''}" data-pick-fts="${esc(idx.name)}">Test</button>
                        ${canWrite ? `<button type="button" class="btn btn-danger btn-sm" data-drop-fts="${esc(idx.name)}">${esc(t('common.delete'))}</button>` : ''}
                      </td>
                    </tr>
                  `).join('')}
                </tbody>
              </table>
            </div>
          ` : `<div class="empty-state">${esc(t('dbs.ftsNoIndexes'))}</div>`}
        </div>

        ${testRes ? `
          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">${esc(t('dbs.ftsTestTitle'))} <span class="mono text-xs muted">(${esc(this._selectedFts)})</span></h3>
              <button type="button" class="btn btn-sm" id="open-sql-btn">${esc(t('dbs.ftsOpenSql'))}</button>
            </div>
            <div class="panel-body stack gap-md">
              <search>
                <form id="fts-test-form" class="search-bar">
                  <input type="search" id="fts-query-input" value="${esc(this._query)}" placeholder="${esc(t('dbs.ftsTestPlaceholder'))}">
                  <button type="submit" class="btn btn-primary btn-sm">${esc(t('search.searchBtn'))}</button>
                </form>
              </search>
              <div>
                <div class="muted text-xs" style="margin-bottom:4px;">${esc(t('dbs.ftsGeneratedSql'))} · ${testRes.took_ms} ms</div>
                <pre class="code-block">${esc(testRes.sql)}</pre>
              </div>
              ${testRes.rows.length ? `
                <div class="result-list">
                  ${testRes.rows.map(({ row, score }) => `
                    <div class="result-card">
                      <div class="row-between">
                        <span class="mono text-xs muted">id=${esc(row.id)}</span>
                        <span class="badge badge-accent mono">${esc(t('search.score'))} ${score.toFixed(2)}</span>
                      </div>
                      ${row.title ? `<div class="result-title">${mock.highlightText(row.title, this._query)}</div>` : ''}
                      <div class="result-body">${mock.highlightText(row.body || row.content || '', this._query)}</div>
                    </div>
                  `).join('')}
                </div>
              ` : `<div class="empty-state">${esc(t('search.noHits'))}</div>`}
            </div>
          </div>
        ` : ''}
      </div>

      <dialog id="create-fts-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title">${esc(t('dbs.ftsNewIndex'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" id="close-fts-dlg">✕</button>
        </div>
        <form id="create-fts-form">
          <div class="dialog-body stack gap-md">
            <div class="field">
              <label for="fts-name">${esc(t('dbs.ftsIndexName'))}</label>
              <input type="text" id="fts-name" required placeholder="idx_posts_fts">
            </div>
            <div class="field">
              <label for="fts-table">${esc(t('dbs.ftsTable'))}</label>
              <input type="text" id="fts-table" required value="articles" placeholder="articles">
            </div>
            <div class="field">
              <label for="fts-cols">${esc(t('dbs.ftsColumns'))}</label>
              <input type="text" id="fts-cols" required value="title, body" placeholder="title, body">
            </div>
            <div class="field">
              <label for="fts-tok">${esc(t('dbs.ftsTokenizer'))}</label>
              <select id="fts-tok">
                <option value="ngram">ngram (推荐中文/子串)</option>
                <option value="simple">simple (英文标准)</option>
                <option value="whitespace">whitespace (空格切分)</option>
              </select>
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" id="cancel-fts-dlg">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.create'))}</button>
          </div>
        </form>
      </dialog>
    `;

    this._bindEvents(dbId, testRes);
  }

  _bindEvents(dbId, testRes) {
    const root = this.shadowRoot;
    const { t } = getI18n();
    const mock = getMock();

    const newBtn = root.getElementById('new-fts-btn');
    if (newBtn) {
      newBtn.addEventListener('click', () => root.getElementById('create-fts-dialog').showModal());
    }
    ['close-fts-dlg', 'cancel-fts-dlg'].forEach((id) => {
      const btn = root.getElementById(id);
      if (btn) btn.addEventListener('click', () => root.getElementById('create-fts-dialog').close());
    });

    const createForm = root.getElementById('create-fts-form');
    if (createForm) {
      createForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        try {
          const created = mock.createDbFtsIndex(dbId, {
            name: root.getElementById('fts-name').value.trim(),
            table: root.getElementById('fts-table').value.trim(),
            columns: root.getElementById('fts-cols').value.trim(),
            tokenizer: root.getElementById('fts-tok').value
          });
          root.getElementById('create-fts-dialog').close();
          this._selectedFts = created.name;
          emitToast(this, t('dbs.ftsCreated'), 'success');
          this.render();
        } catch (err) {
          emitToast(this, err.message, 'error');
        }
      });
    }

    root.querySelectorAll('[data-pick-fts]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._selectedFts = btn.getAttribute('data-pick-fts');
        this.render();
      });
    });

    root.querySelectorAll('[data-drop-fts]').forEach((btn) => {
      btn.addEventListener('click', () => {
        mock.dropDbFtsIndex(dbId, btn.getAttribute('data-drop-fts'));
        emitToast(this, t('dbs.ftsDropped'), 'success');
        this.render();
      });
    });

    const testForm = root.getElementById('fts-test-form');
    if (testForm) {
      testForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._query = root.getElementById('fts-query-input').value;
        this.render();
      });
    }

    const openSqlBtn = root.getElementById('open-sql-btn');
    if (openSqlBtn && testRes) {
      openSqlBtn.addEventListener('click', () => {
        this.dispatchEvent(new CustomEvent('fp-open-sql', {
          detail: { sql: testRes.sql },
          bubbles: true,
          composed: true
        }));
      });
    }
  }
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
      limit: '100',
      mockFts: true
    };
  }

  set projects(list) {
    this._projects = Array.isArray(list) ? list : [];
    if (this.isConnected) this.render();
  }

  set liveEntries(list) {
    this._liveEntries = Array.isArray(list) ? list : [];
    if (this.isConnected && !this._filters.mockFts) this.render();
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (this._scope.startsWith('project:')) {
      this._filters.project_id = this._scope.slice('project:'.length);
    }
    if (this.isConnected) this.render();
  }

  connectedCallback() {
    this.render();
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
    const mock = getMock();
    const f = this._filters;

    const scopePid = this._scope.startsWith('project:') ? this._scope.slice('project:'.length) : '';
    let entries = [];
    let summaryText = '';
    if (f.mockFts) {
      const res = mock.queryMockLogs(f, this._scope);
      entries = res.entries;
      summaryText = t('logs.ftsHitsSummary', { total: res.total, ms: res.took_ms, index: res.index_name });
    } else {
      const qLower = f.q.trim().toLowerCase();
      entries = this._liveEntries.filter((e) => {
        if (!qLower) return true;
        return String(e.message || '').toLowerCase().includes(qLower) ||
          String(e.meta_json || '').toLowerCase().includes(qLower);
      });
      summaryText = `${entries.length} · ${t('logs.ftsLiveMode')}`;
    }

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
            <div class="log-msg">${mock.highlightText(e.message, f.q)}</div>
            ${e.meta_json ? `
              <details>
                <summary class="muted text-xs" style="cursor:pointer;">${esc(t('logs.meta'))}</summary>
                <pre class="code-block" style="margin-top:6px;">${mock.highlightText(e.meta_json, f.q)}</pre>
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
                <label class="checkbox-pill">
                  <input type="checkbox" id="lf-mock" ${f.mockFts ? 'checked' : ''}>
                  <span>${esc(t('logs.ftsMockMode'))}</span>
                </label>
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
                ${f.mockFts ? `<span class="badge badge-warn">${esc(t('search.mockBadge'))}</span>` : `<span class="badge badge-success">${esc(t('logs.ftsLiveMode'))}</span>`}
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
      mockFts: root.getElementById('lf-mock').checked,
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
    if (form) {
      form.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._syncFiltersFromDom();
        if (!this._filters.mockFts) {
          this.dispatchEvent(new CustomEvent('fp-log-query', {
            detail: { ...this._filters },
            bubbles: true,
            composed: true
          }));
        }
        this.render();
      });
    }
    const mockToggle = root.getElementById('lf-mock');
    if (mockToggle) {
      mockToggle.addEventListener('change', () => {
        this._syncFiltersFromDom();
        if (!this._filters.mockFts) {
          this.dispatchEvent(new CustomEvent('fp-log-query', {
            detail: { ...this._filters },
            bubbles: true,
            composed: true
          }));
        }
        this.render();
      });
    }
  }
}

if (!customElements.get('fp-search-workbench')) {
  customElements.define('fp-search-workbench', FpSearchWorkbench);
}
if (!customElements.get('fp-db-fts-panel')) {
  customElements.define('fp-db-fts-panel', FpDbFtsPanel);
}
if (!customElements.get('fp-log-search-panel')) {
  customElements.define('fp-log-search-panel', FpLogSearchPanel);
}
