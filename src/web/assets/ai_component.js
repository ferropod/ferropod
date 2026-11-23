// Native HTML Web Component: <fp-ai-gateway>
// Manage AI Gateway upstream providers, token budgets & usage telemetry, and live Chat/Embeddings Playground (with SSE streaming).

const aiSheet = new CSSStyleSheet();
aiSheet.replaceSync(`
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

  .stats-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
  }
  @media (max-width: 900px) {
    .stats-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  .stat-card {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .stat-label { font-size: 12px; color: var(--muted); }
  .stat-value { font-size: 20px; font-weight: 700; }

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
    max-height: 360px;
    overflow-y: auto;
  }
  .empty-state { padding: 28px 16px; text-align: center; color: var(--muted); font-size: 13px; }

  input[type="text"], input[type="password"], input[type="number"], input[type="url"], input:not([type]), select, textarea {
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
  textarea { min-height: 96px; resize: vertical; }
  input:focus, select:focus, textarea:focus { border-color: var(--accent); box-shadow: 0 0 0 3px var(--accent-soft); }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field label { font-size: 12px; font-weight: 600; color: var(--text-secondary); }
  .form-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
  .checkbox-row { display: inline-flex; align-items: center; gap: 8px; font-size: 13px; cursor: pointer; }
  .checkbox-row input { width: auto; }

  dialog.fp-dialog {
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--shadow-md);
    width: min(560px, calc(100vw - 24px));
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
  .dialog-body { padding: 16px; overflow-y: auto; max-height: 70vh; }
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
    input[type="text"], input[type="password"], input[type="number"], input[type="url"], select, textarea { font-size: 16px; min-height: 42px; }
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

function requestConfirm(el, message, onConfirm, opts = {}) {
  const ev = new CustomEvent('fp-confirm', {
    detail: { message, onConfirm, ...opts },
    bubbles: true,
    composed: true,
    cancelable: true
  });
  el.dispatchEvent(ev);
  if (!ev.defaultPrevented && window.confirm(message)) {
    onConfirm();
  }
}

class FpAiGateway extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [aiSheet];
    this._token = '';
    this._scope = 'admin';
    this._tab = 'providers';
    this._providers = [];
    this._loadingProviders = false;
    this._editingProvider = null;
    this._usage = null;
    this._usageFilter = '';
    this._loadingUsage = false;
    this._pg = {
      provider: '',
      mode: 'chat',
      model: '',
      stream: true,
      prompt: 'Hello! Reply in one short sentence.',
      running: false,
      meta: '',
      output: ''
    };
    this._onLangChange = () => { if (this.isConnected) this.render(); };
  }

  set token(val) {
    this._token = String(val || '');
    if (this.isConnected) this.refreshAll();
  }

  set scope(val) {
    this._scope = String(val || 'admin');
    if (this.isConnected) this.render();
  }

  _isAdmin() {
    return this._scope === 'admin';
  }

  _canInvoke() {
    return this._scope !== 'readonly';
  }

  _authHeaders(json = false) {
    const tok = this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
    const h = {};
    if (tok) h.Authorization = 'Bearer ' + tok;
    if (json) h['Content-Type'] = 'application/json';
    return h;
  }

  connectedCallback() {
    window.addEventListener('fp-lang-change', this._onLangChange);
    this.render();
    this.refreshAll();
  }

  disconnectedCallback() {
    window.removeEventListener('fp-lang-change', this._onLangChange);
  }

  async refreshAll() {
    await Promise.all([this.loadProviders(), this.loadUsage()]);
  }

  async loadProviders() {
    this._loadingProviders = true;
    this.render();
    try {
      const res = await fetch('/_api/v1/ai/providers', { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._providers = (await res.json()) || [];
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._loadingProviders = false;
      this.render();
    }
  }

  async loadUsage() {
    this._loadingUsage = true;
    this.render();
    try {
      const qs = this._usageFilter ? `?provider=${encodeURIComponent(this._usageFilter)}&limit=100` : '?limit=100';
      const res = await fetch('/_api/v1/ai/usage' + qs, { headers: this._authHeaders() });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      this._usage = await res.json();
    } catch (err) {
      emitToast(this, err.message, 'error');
    } finally {
      this._loadingUsage = false;
      this.render();
    }
  }

  async saveProvider(payload, existingName) {
    const { t } = getI18n();
    try {
      const url = existingName
        ? '/_api/v1/ai/providers/' + encodeURIComponent(existingName)
        : '/_api/v1/ai/providers';
      const res = await fetch(url, {
        method: existingName ? 'PATCH' : 'POST',
        headers: this._authHeaders(true),
        body: JSON.stringify(payload)
      });
      if (!res.ok) {
        const txt = await res.text();
        throw new Error(txt || `HTTP ${res.status}`);
      }
      emitToast(this, t('ai.providerSaved'), 'success');
      await this.loadProviders();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async toggleProvider(p) {
    await this.saveProvider({ enabled: !p.enabled }, p.name);
  }

  async deleteProvider(name) {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/ai/providers/' + encodeURIComponent(name), {
        method: 'DELETE',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      emitToast(this, t('ai.providerDeleted'), 'success');
      await this.loadProviders();
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async clearCache() {
    const { t } = getI18n();
    try {
      const res = await fetch('/_api/v1/ai/cache/clear', {
        method: 'POST',
        headers: this._authHeaders()
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      emitToast(this, t('ai.cacheCleared', { count: data.evicted || 0 }), 'success');
    } catch (err) {
      emitToast(this, err.message, 'error');
    }
  }

  async runPlayground() {
    if (!this._canInvoke()) return;
    this._pg.running = true;
    this._pg.output = '';
    this._pg.meta = '';
    this.render();

    const subpath = this._pg.mode === 'embeddings' ? '/_ai/v1/embeddings' : '/_ai/v1/chat/completions';
    const body = this._pg.mode === 'embeddings'
      ? { input: this._pg.prompt }
      : {
          messages: [{ role: 'user', content: this._pg.prompt }],
          stream: Boolean(this._pg.stream)
        };
    if (this._pg.model.trim()) {
      body.model = this._pg.model.trim();
    }

    const headers = this._authHeaders(true);
    if (this._pg.provider) {
      headers['x-ferropod-provider'] = this._pg.provider;
    }

    try {
      const res = await fetch(subpath, {
        method: 'POST',
        headers,
        body: JSON.stringify(body)
      });
      const prov = res.headers.get('x-ferropod-provider') || '-';
      const cache = res.headers.get('x-ferropod-cache') || '-';
      this._pg.meta = `HTTP ${res.status} · provider=${prov} · cache=${cache}`;

      const ct = res.headers.get('content-type') || '';
      if (ct.includes('text/event-stream') && res.body) {
        const reader = res.body.getReader();
        const decoder = new TextDecoder();
        let buffer = '';
        let assembled = '';
        while (true) {
          const { value, done } = await reader.read();
          if (done) break;
          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split('\n');
          buffer = lines.pop() || '';
          for (const line of lines) {
            const trimmed = line.trim();
            if (!trimmed.startsWith('data:')) continue;
            const payload = trimmed.slice(5).trim();
            if (!payload || payload === '[DONE]') continue;
            try {
              const parsed = JSON.parse(payload);
              const delta = parsed.choices && parsed.choices[0] && parsed.choices[0].delta && parsed.choices[0].delta.content;
              if (typeof delta === 'string') {
                assembled += delta;
                this._pg.output = assembled;
                const outEl = this.shadowRoot.getElementById('ai-pg-output');
                if (outEl) outEl.textContent = assembled;
              }
            } catch (_) {
              assembled += payload + '\n';
            }
          }
        }
        if (!assembled) {
          this._pg.output = buffer || '(empty stream)';
        }
      } else {
        const txt = await res.text();
        try {
          this._pg.output = JSON.stringify(JSON.parse(txt), null, 2);
        } catch (_) {
          this._pg.output = txt;
        }
      }
      await this.loadUsage();
    } catch (err) {
      this._pg.output = err.message || 'Request failed';
      emitToast(this, this._pg.output, 'error');
    } finally {
      this._pg.running = false;
      this.render();
    }
  }

  _openProviderModal(existing) {
    this._editingProvider = existing || null;
    const dlg = this.shadowRoot.getElementById('ai-provider-dialog');
    if (!dlg) return;
    const { t } = getI18n();
    const form = this.shadowRoot.getElementById('ai-provider-form');
    this.shadowRoot.getElementById('ai-dlg-title').textContent = existing
      ? `${t('common.edit')} · ${existing.name}`
      : t('ai.newProvider');
    const nameEl = this.shadowRoot.getElementById('ap-name');
    nameEl.value = existing ? existing.name : '';
    nameEl.disabled = Boolean(existing);
    this.shadowRoot.getElementById('ap-url').value = existing ? existing.base_url : 'https://api.openai.com/v1';
    this.shadowRoot.getElementById('ap-key').value = '';
    this.shadowRoot.getElementById('ap-key').placeholder = existing && existing.has_api_key ? t('ai.keepKeyHint') : 'sk-...';
    this.shadowRoot.getElementById('ap-model').value = existing && existing.default_model ? existing.default_model : '';
    this.shadowRoot.getElementById('ap-fallback').value = existing && existing.fallback_provider ? existing.fallback_provider : '';
    this.shadowRoot.getElementById('ap-rpm').value = existing && existing.rate_limit_rpm ? existing.rate_limit_rpm : '';
    this.shadowRoot.getElementById('ap-cache').value = existing ? existing.cache_ttl_secs : 0;
    this.shadowRoot.getElementById('ap-budget').value = existing && existing.monthly_token_budget ? existing.monthly_token_budget : '';
    this.shadowRoot.getElementById('ap-timeout').value = existing ? (existing.timeout_secs || 120) : 120;
    this.shadowRoot.getElementById('ap-enabled').checked = existing ? Boolean(existing.enabled) : true;
    form.dataset.editing = existing ? existing.name : '';
    dlg.showModal();
  }

  render() {
    const { t, formatTime } = getI18n();
    const isAdmin = this._isAdmin();
    const canInvoke = this._canInvoke();
    const u = this._usage || { total_requests: 0, cached_requests: 0, total_prompt_tokens: 0, total_completion_tokens: 0, total_tokens: 0, month_tokens: 0, recent: [] };
    const hitRate = u.total_requests > 0 ? ((u.cached_requests / u.total_requests) * 100).toFixed(1) : '0.0';

    let tabBody = '';
    if (this._tab === 'providers') {
      const rows = this._providers.map((p) => `
        <tr>
          <td class="cell-primary">
            <div class="row-between">
              <span class="text-strong mono">${esc(p.name)}</span>
              <span class="badge ${p.enabled ? 'badge-success' : 'badge-neutral'}">${esc(p.enabled ? t('ops.enabled') : t('ops.disabled'))}</span>
            </div>
            <div class="muted text-xs mono break-all">${esc(p.base_url)}</div>
          </td>
          <td data-label="${esc(t('ai.defaultModel'))}" class="mono text-xs">${esc(p.default_model || '-')}</td>
          <td data-label="${esc(t('ai.fallback'))}" class="mono text-xs">${esc(p.fallback_provider || '-')}</td>
          <td data-label="${esc(t('ai.limits'))}" class="mono text-xs">
            ${p.rate_limit_rpm ? `${esc(p.rate_limit_rpm)} rpm` : '-'} · ${esc(p.cache_ttl_secs)}s cache · ${esc(p.timeout_secs || 120)}s
          </td>
          <td data-label="${esc(t('ai.monthlyBudget'))}" class="mono text-xs">${p.monthly_token_budget ? esc(p.monthly_token_budget.toLocaleString()) : t('common.never')}</td>
          <td data-label="${esc(t('ai.apiKey'))}">${p.has_api_key ? `<span class="badge badge-accent">${esc(t('ai.configured'))}</span>` : `<span class="badge badge-neutral">-</span>`}</td>
          ${isAdmin ? `
            <td class="cell-actions-td"><div class="cell-actions">
              <button type="button" class="btn btn-sm" data-edit-prov="${esc(p.name)}">${esc(t('common.edit'))}</button>
              <button type="button" class="btn btn-sm" data-toggle-prov="${esc(p.name)}">${esc(p.enabled ? t('perf.pause') : t('perf.resume'))}</button>
              <button type="button" class="btn btn-danger btn-sm" data-del-prov="${esc(p.name)}">${esc(t('common.delete'))}</button>
            </div></td>
          ` : ''}
        </tr>
      `).join('');

      tabBody = `
        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">
              <span>${esc(t('ai.providersList'))}</span>
              <span class="badge badge-neutral">${this._providers.length}</span>
            </h3>
            <div class="row gap-xs">
              <button type="button" class="btn btn-sm" id="ai-refresh-btn">${esc(t('common.refresh'))}</button>
              ${isAdmin ? `<button type="button" class="btn btn-sm" id="ai-clear-cache-btn">${esc(t('ai.clearCache'))}</button>` : ''}
              ${isAdmin ? `<button type="button" class="btn btn-primary btn-sm" id="ai-new-prov-btn">${esc(t('ai.newProvider'))}</button>` : ''}
            </div>
          </div>
          ${this._loadingProviders && !this._providers.length
            ? `<div class="empty-state">${esc(t('common.loading'))}</div>`
            : (rows
              ? `<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr>
                  <th>${esc(t('common.name'))}</th>
                  <th>${esc(t('ai.defaultModel'))}</th>
                  <th>${esc(t('ai.fallback'))}</th>
                  <th>${esc(t('ai.limits'))}</th>
                  <th>${esc(t('ai.monthlyBudget'))}</th>
                  <th>${esc(t('ai.apiKey'))}</th>
                  ${isAdmin ? `<th>${esc(t('common.actions'))}</th>` : ''}
                </tr></thead><tbody>${rows}</tbody></table></div>`
              : `<div class="empty-state">${esc(t('ai.noProviders'))}</div>`)
          }
        </div>
      `;
    } else if (this._tab === 'usage') {
      const uRows = (u.recent || []).map((r) => `
        <tr>
          <td class="cell-primary mono text-strong">${esc(r.provider_name)}</td>
          <td data-label="${esc(t('ai.defaultModel'))}" class="mono text-xs">${esc(r.model || '-')}</td>
          <td data-label="Tokens" class="mono text-xs">${esc(r.prompt_tokens)} + ${esc(r.completion_tokens)} = <strong>${esc(r.total_tokens)}</strong></td>
          <td data-label="Cache">${r.cached ? '<span class="badge badge-success">HIT</span>' : '<span class="badge badge-neutral">MISS</span>'}</td>
          <td data-label="${esc(t('common.status'))}"><span class="badge ${r.status >= 200 && r.status < 300 ? 'badge-success' : 'badge-danger'} mono">${esc(r.status)}</span></td>
          <td data-label="Latency" class="mono text-xs">${esc(r.latency_ms)} ms</td>
          <td data-label="${esc(t('common.createdAt'))}" class="muted text-xs">${esc(formatTime(r.created_at))}</td>
        </tr>
      `).join('');

      tabBody = `
        <div class="stack gap-md">
          <div class="stats-grid">
            <div class="stat-card"><span class="stat-label">${esc(t('ai.totalRequests'))}</span><span class="stat-value">${esc(u.total_requests)}</span></div>
            <div class="stat-card"><span class="stat-label">${esc(t('ai.cacheHits'))}</span><span class="stat-value">${esc(u.cached_requests)} <span class="text-xs muted">(${hitRate}%)</span></span></div>
            <div class="stat-card"><span class="stat-label">${esc(t('ai.totalTokens'))}</span><span class="stat-value">${esc(u.total_tokens.toLocaleString())}</span></div>
            <div class="stat-card"><span class="stat-label">${esc(t('ai.monthTokens'))}</span><span class="stat-value">${esc((u.month_tokens || 0).toLocaleString())}</span></div>
          </div>

          <div class="panel">
            <div class="panel-header">
              <h3 class="panel-title">${esc(t('ai.recentUsage'))}</h3>
              <div class="row gap-xs">
                <select id="ai-usage-filter" style="width:auto;">
                  <option value="">${esc(t('common.all'))}</option>
                  ${this._providers.map((p) => `<option value="${esc(p.name)}" ${this._usageFilter === p.name ? 'selected' : ''}>${esc(p.name)}</option>`).join('')}
                </select>
                <button type="button" class="btn btn-sm" id="ai-usage-refresh">${esc(t('common.refresh'))}</button>
              </div>
            </div>
            ${uRows
              ? `<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr>
                  <th>${esc(t('ai.provider'))}</th>
                  <th>${esc(t('ai.defaultModel'))}</th>
                  <th>Tokens</th>
                  <th>Cache</th>
                  <th>${esc(t('common.status'))}</th>
                  <th>Latency</th>
                  <th>${esc(t('common.createdAt'))}</th>
                </tr></thead><tbody>${uRows}</tbody></table></div>`
              : `<div class="empty-state">${esc(t('ai.noUsage'))}</div>`
            }
          </div>
        </div>
      `;
    } else {
      tabBody = `
        <div class="panel">
          <div class="panel-header">
            <h3 class="panel-title">${esc(t('ai.playgroundTitle'))}</h3>
            <span class="mono text-xs muted">POST /_ai/v1/${this._pg.mode === 'embeddings' ? 'embeddings' : 'chat/completions'}</span>
          </div>
          <div class="panel-body stack gap-md">
            ${!canInvoke ? `<div class="badge badge-warn">${esc(t('ai.readonlyDisabled'))}</div>` : ''}
            <form id="ai-pg-form" class="stack gap-md">
              <div class="form-grid">
                <div class="field">
                  <label>${esc(t('ai.provider'))}</label>
                  <select id="ai-pg-provider">
                    <option value="">${esc(t('ai.autoProvider'))}</option>
                    ${this._providers.filter((p) => p.enabled).map((p) => `<option value="${esc(p.name)}" ${this._pg.provider === p.name ? 'selected' : ''}>${esc(p.name)}</option>`).join('')}
                  </select>
                </div>
                <div class="field">
                  <label>${esc(t('ai.mode'))}</label>
                  <select id="ai-pg-mode">
                    <option value="chat" ${this._pg.mode === 'chat' ? 'selected' : ''}>Chat Completions</option>
                    <option value="embeddings" ${this._pg.mode === 'embeddings' ? 'selected' : ''}>Embeddings</option>
                  </select>
                </div>
                <div class="field">
                  <label>${esc(t('ai.modelOverride'))}</label>
                  <input type="text" id="ai-pg-model" value="${esc(this._pg.model)}" placeholder="gpt-4o-mini">
                </div>
                <div class="field" style="justify-content:flex-end;">
                  <label class="checkbox-row">
                    <input type="checkbox" id="ai-pg-stream" ${this._pg.stream ? 'checked' : ''} ${this._pg.mode === 'embeddings' ? 'disabled' : ''}>
                    <span>${esc(t('ai.streamSse'))}</span>
                  </label>
                </div>
              </div>
              <div class="field">
                <label>${esc(t('ai.prompt'))}</label>
                <textarea id="ai-pg-prompt" rows="4" required>${esc(this._pg.prompt)}</textarea>
              </div>
              <div class="row">
                <button type="submit" class="btn btn-primary" ${!canInvoke || this._pg.running ? 'disabled' : ''}>${esc(this._pg.running ? t('common.loading') : t('ai.sendRequest'))}</button>
                ${this._pg.meta ? `<span class="badge badge-info mono">${esc(this._pg.meta)}</span>` : ''}
              </div>
            </form>
            ${this._pg.output || this._pg.running ? `<pre class="code-block" id="ai-pg-output">${esc(this._pg.output)}</pre>` : ''}
          </div>
        </div>
      `;
    }

    this.shadowRoot.innerHTML = `
      <div class="page-head">
        <div>
          <h2 class="page-title">${esc(t('ai.title'))}</h2>
          <p class="page-subtitle">${esc(t('ai.subtitle'))}</p>
        </div>
      </div>
      <div class="tabs">
        <button type="button" class="tab-btn ${this._tab === 'providers' ? 'active' : ''}" data-ai-tab="providers">${esc(t('ai.tabProviders'))}</button>
        <button type="button" class="tab-btn ${this._tab === 'usage' ? 'active' : ''}" data-ai-tab="usage">${esc(t('ai.tabUsage'))}</button>
        <button type="button" class="tab-btn ${this._tab === 'playground' ? 'active' : ''}" data-ai-tab="playground">${esc(t('ai.tabPlayground'))}</button>
      </div>
      ${tabBody}

      <dialog id="ai-provider-dialog" class="fp-dialog">
        <div class="dialog-head">
          <h3 class="dialog-title" id="ai-dlg-title">${esc(t('ai.newProvider'))}</h3>
          <button type="button" class="btn btn-ghost btn-sm" id="ai-dlg-close">✕</button>
        </div>
        <form id="ai-provider-form">
          <div class="dialog-body stack gap-md">
            <div class="form-grid">
              <div class="field">
                <label>${esc(t('common.name'))}</label>
                <input type="text" id="ap-name" required placeholder="openai">
              </div>
              <div class="field">
                <label>${esc(t('ai.defaultModel'))}</label>
                <input type="text" id="ap-model" placeholder="gpt-4o-mini">
              </div>
            </div>
            <div class="field">
              <label>Base URL</label>
              <input type="url" id="ap-url" required placeholder="https://api.openai.com/v1">
            </div>
            <div class="field">
              <label>${esc(t('ai.apiKey'))}</label>
              <input type="password" id="ap-key" placeholder="sk-...">
            </div>
            <div class="form-grid">
              <div class="field">
                <label>${esc(t('ai.fallback'))}</label>
                <input type="text" id="ap-fallback" placeholder="openrouter">
              </div>
              <div class="field">
                <label>${esc(t('ai.rpmLimit'))}</label>
                <input type="number" id="ap-rpm" min="0" placeholder="60">
              </div>
              <div class="field">
                <label>${esc(t('ai.cacheTtl'))}</label>
                <input type="number" id="ap-cache" min="0" value="0">
              </div>
              <div class="field">
                <label>${esc(t('ai.monthlyBudget'))}</label>
                <input type="number" id="ap-budget" min="0" placeholder="1000000">
              </div>
              <div class="field">
                <label>${esc(t('ai.timeoutSecs'))}</label>
                <input type="number" id="ap-timeout" min="5" max="600" value="120">
              </div>
              <div class="field" style="justify-content:flex-end;">
                <label class="checkbox-row">
                  <input type="checkbox" id="ap-enabled" checked>
                  <span>${esc(t('ops.enabled'))}</span>
                </label>
              </div>
            </div>
          </div>
          <div class="dialog-foot">
            <button type="button" class="btn" id="ai-dlg-cancel">${esc(t('common.cancel'))}</button>
            <button type="submit" class="btn btn-primary">${esc(t('common.save'))}</button>
          </div>
        </form>
      </dialog>
    `;

    this._bindEvents();
  }

  _bindEvents() {
    const sr = this.shadowRoot;
    const { t } = getI18n();

    sr.querySelectorAll('[data-ai-tab]').forEach((btn) => {
      btn.addEventListener('click', () => {
        this._tab = btn.getAttribute('data-ai-tab');
        this.render();
      });
    });

    const refreshBtn = sr.getElementById('ai-refresh-btn');
    if (refreshBtn) refreshBtn.addEventListener('click', () => this.loadProviders());

    const clearCacheBtn = sr.getElementById('ai-clear-cache-btn');
    if (clearCacheBtn) clearCacheBtn.addEventListener('click', () => this.clearCache());

    const newBtn = sr.getElementById('ai-new-prov-btn');
    if (newBtn) newBtn.addEventListener('click', () => this._openProviderModal(null));

    sr.querySelectorAll('[data-edit-prov]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const name = btn.getAttribute('data-edit-prov');
        const p = this._providers.find((x) => x.name === name);
        if (p) this._openProviderModal(p);
      });
    });

    sr.querySelectorAll('[data-toggle-prov]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const name = btn.getAttribute('data-toggle-prov');
        const p = this._providers.find((x) => x.name === name);
        if (p) this.toggleProvider(p);
      });
    });

    sr.querySelectorAll('[data-del-prov]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const name = btn.getAttribute('data-del-prov');
        requestConfirm(this, t('ai.deleteProviderConfirm', { name }), () => {
          this.deleteProvider(name);
        });
      });
    });

    const usageFilter = sr.getElementById('ai-usage-filter');
    if (usageFilter) {
      usageFilter.addEventListener('change', () => {
        this._usageFilter = usageFilter.value;
        this.loadUsage();
      });
    }
    const usageRefresh = sr.getElementById('ai-usage-refresh');
    if (usageRefresh) usageRefresh.addEventListener('click', () => this.loadUsage());

    const dlg = sr.getElementById('ai-provider-dialog');
    const closeDlg = () => { if (dlg && dlg.open) dlg.close(); };
    if (sr.getElementById('ai-dlg-close')) sr.getElementById('ai-dlg-close').addEventListener('click', closeDlg);
    if (sr.getElementById('ai-dlg-cancel')) sr.getElementById('ai-dlg-cancel').addEventListener('click', closeDlg);
    if (dlg) {
      dlg.addEventListener('click', (ev) => {
        if (ev.target === dlg) closeDlg();
      });
    }

    const pForm = sr.getElementById('ai-provider-form');
    if (pForm) {
      pForm.addEventListener('submit', async (ev) => {
        ev.preventDefault();
        const editing = pForm.dataset.editing || '';
        const keyVal = sr.getElementById('ap-key').value.trim();
        const rpmRaw = sr.getElementById('ap-rpm').value.trim();
        const budgetRaw = sr.getElementById('ap-budget').value.trim();
        const payload = {
          base_url: sr.getElementById('ap-url').value.trim(),
          default_model: sr.getElementById('ap-model').value.trim(),
          fallback_provider: sr.getElementById('ap-fallback').value.trim(),
          rate_limit_rpm: rpmRaw ? Number(rpmRaw) : 0,
          cache_ttl_secs: Number(sr.getElementById('ap-cache').value || 0),
          monthly_token_budget: budgetRaw ? Number(budgetRaw) : 0,
          timeout_secs: Number(sr.getElementById('ap-timeout').value || 120),
          enabled: sr.getElementById('ap-enabled').checked
        };
        if (!editing) payload.name = sr.getElementById('ap-name').value.trim();
        if (keyVal) payload.api_key = keyVal;
        closeDlg();
        await this.saveProvider(payload, editing || null);
      });
    }

    const pgForm = sr.getElementById('ai-pg-form');
    if (pgForm) {
      const modeSel = sr.getElementById('ai-pg-mode');
      modeSel.addEventListener('change', () => {
        this._pg.mode = modeSel.value;
        this._pg.provider = sr.getElementById('ai-pg-provider').value;
        this._pg.model = sr.getElementById('ai-pg-model').value;
        this._pg.prompt = sr.getElementById('ai-pg-prompt').value;
        this.render();
      });
      pgForm.addEventListener('submit', (ev) => {
        ev.preventDefault();
        this._pg.provider = sr.getElementById('ai-pg-provider').value;
        this._pg.mode = sr.getElementById('ai-pg-mode').value;
        this._pg.model = sr.getElementById('ai-pg-model').value;
        this._pg.stream = sr.getElementById('ai-pg-stream').checked;
        this._pg.prompt = sr.getElementById('ai-pg-prompt').value;
        this.runPlayground();
      });
    }
  }
}

if (!customElements.get('fp-ai-gateway')) {
  customElements.define('fp-ai-gateway', FpAiGateway);
}
