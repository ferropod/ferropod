// Native HTML Web Component: <fp-perf-monitor>
// Real-time lightweight telemetry panel (Process RSS, API RTT, Load Average, Uptime & Open FDs)
// with inline SVG sparklines and zero external dependencies.

const perfSheet = new CSSStyleSheet();
perfSheet.replaceSync(`
  *, *::before, *::after { box-sizing: border-box; }
  :host {
    display: block;
    color: var(--text);
    font-family: var(--font-sans);
    font-size: 14px;
    touch-action: manipulation;
    max-width: 100%;
  }
  .mono { font-family: var(--font-mono); }
  .muted { color: var(--muted); }
  .text-xs { font-size: 12px; }
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
  .panel-title {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    height: 30px;
    padding: 0 10px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 0.15s ease, border-color 0.15s ease;
  }
  .btn:hover { background: var(--surface-subtle); border-color: var(--border-strong); }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
  }
  .badge-success { background: var(--success-soft); color: var(--success); }
  .badge-warn { background: var(--warn-soft); color: var(--warn); }
  .badge-neutral { background: var(--surface-subtle); color: var(--text-secondary); border: 1px solid var(--border); }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
    padding: 14px 16px;
  }
  @media (max-width: 1024px) {
    .grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  }
  @media (max-width: 560px) {
    .panel-header { padding: 12px 14px; }
    .grid {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 10px;
      padding: 12px;
    }
    .metric-card { padding: 10px 12px; }
    .metric-value { font-size: 16px; }
  }

  .metric-card {
    background: var(--surface-subtle);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-width: 0;
  }
  .metric-top {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
  }
  .metric-label { font-size: 12px; color: var(--muted); font-weight: 500; }
  .metric-value { font-size: 18px; font-weight: 700; letter-spacing: -0.02em; }
  .metric-sub { font-size: 11.5px; color: var(--muted); display: flex; justify-content: space-between; gap: 6px; }

  .progress {
    width: 100%;
    height: 6px;
    border-radius: 999px;
    background: var(--border);
    overflow: hidden;
  }
  .progress-bar {
    height: 100%;
    border-radius: 999px;
    background: var(--accent);
    transition: width 0.3s ease;
  }
  .progress-bar.ok { background: var(--success); }
  .progress-bar.warn { background: var(--warn); }
  .progress-bar.danger { background: var(--danger); }

  .sparkline {
    width: 100%;
    height: 28px;
    display: block;
  }
  .telemetry-strip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 10px;
    padding: 10px 16px;
    border-top: 1px solid var(--border);
    background: var(--surface-subtle);
    font-size: 12px;
  }
  .telemetry-pills { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
`);

function esc(v) {
  return String(v === null || v === undefined ? '' : v)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;');
}

function getI18n() {
  return window.FerroI18n || {
    t: (k) => k,
    formatBytes: (b) => `${b || 0} B`
  };
}

function formatUptime(secs) {
  const s = Math.max(0, Math.floor(Number(secs) || 0));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const rem = s % 60;
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m ${rem}s`;
  if (m > 0) return `${m}m ${rem}s`;
  return `${rem}s`;
}

function buildSparkline(values, colorVar) {
  if (!values.length) return '';
  const w = 160;
  const h = 28;
  const max = Math.max(...values, 1);
  const min = Math.min(...values, 0);
  const span = Math.max(max - min, 0.001);
  const pts = values.map((v, idx) => {
    const x = values.length === 1 ? w : (idx / (values.length - 1)) * w;
    const y = h - 3 - ((v - min) / span) * (h - 6);
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  }).join(' ');
  const area = `0,${h} ${pts} ${w},${h}`;
  return `
    <svg class="sparkline" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" aria-hidden="true">
      <polygon points="${area}" fill="${colorVar}" fill-opacity="0.14"></polygon>
      <polyline points="${pts}" fill="none" stroke="${colorVar}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></polyline>
    </svg>
  `;
}

class FpPerfMonitor extends HTMLElement {
  constructor() {
    super();
    this.attachShadow({ mode: 'open' });
    this.shadowRoot.adoptedStyleSheets = [perfSheet];
    this._token = '';
    this._paused = false;
    this._timer = null;
    this._latest = null;
    this._error = '';
    this._rssHistory = [];
    this._rttHistory = [];
    this._loadHistory = [];
    this._onLangChange = () => { if (this.isConnected) this.render(); };
  }

  set token(val) {
    this._token = String(val || '');
    if (this.isConnected && !this._latest) {
      this.refresh();
    }
  }

  connectedCallback() {
    window.addEventListener('fp-lang-change', this._onLangChange);
    this.render();
    this.refresh();
    this._startPolling();
  }

  disconnectedCallback() {
    window.removeEventListener('fp-lang-change', this._onLangChange);
    this._stopPolling();
  }

  _startPolling() {
    this._stopPolling();
    this._timer = window.setInterval(() => {
      if (!this._paused && document.visibilityState !== 'hidden') {
        this.refresh();
      }
    }, 5000);
  }

  _stopPolling() {
    if (this._timer) {
      window.clearInterval(this._timer);
      this._timer = null;
    }
  }

  async refresh() {
    const token = this._token || localStorage.getItem('ferropod_token') || sessionStorage.getItem('ferropod_token') || '';
    if (!token) return;
    const started = performance.now();
    try {
      const res = await fetch('/_api/v1/metrics', {
        headers: { Authorization: 'Bearer ' + token }
      });
      const rttMs = Number((performance.now() - started).toFixed(1));
      if (!res.ok) {
        this._error = `HTTP ${res.status}`;
        this.render();
        return;
      }
      const data = await res.json();
      this._latest = { ...data, rtt_ms: rttMs };
      this._error = '';
      this._pushHistory(this._rssHistory, data.rss_bytes / (1024 * 1024));
      this._pushHistory(this._rttHistory, rttMs);
      this._pushHistory(this._loadHistory, data.load_1m || 0);
      this.render();
    } catch (err) {
      this._error = err.message || 'Fetch error';
      this.render();
    }
  }

  _pushHistory(arr, val) {
    arr.push(Number(val) || 0);
    if (arr.length > 20) arr.shift();
  }

  render() {
    const { t, formatBytes } = getI18n();
    const m = this._latest;

    const statusBadge = this._paused
      ? `<span class="badge badge-warn"><span class="dot"></span>${esc(t('perf.paused'))}</span>`
      : `<span class="badge badge-success"><span class="dot"></span>${esc(t('perf.live'))}</span>`;

    let bodyHtml = `<div style="padding:20px;text-align:center;" class="muted text-xs">${esc(this._error || t('common.loading'))}</div>`;

    if (m) {
      const rssPct = m.mem_budget_bytes > 0 ? Math.min(100, (m.rss_bytes / m.mem_budget_bytes) * 100) : 0;
      const rssBarCls = rssPct > 85 ? 'danger' : (rssPct > 65 ? 'warn' : 'ok');

      const hostUsed = m.mem_total_bytes > 0 ? Math.max(0, m.mem_total_bytes - m.mem_avail_bytes) : 0;
      const hostPct = m.mem_total_bytes > 0 ? Math.min(100, (hostUsed / m.mem_total_bytes) * 100) : 0;

      const avgRtt = this._rttHistory.length
        ? (this._rttHistory.reduce((a, b) => a + b, 0) / this._rttHistory.length).toFixed(1)
        : m.rtt_ms;

      bodyHtml = `
        <div class="grid">
          <div class="metric-card">
            <div class="metric-top">
              <span class="metric-label">${esc(t('perf.processRss'))}</span>
              <span class="badge badge-neutral mono">${rssPct.toFixed(1)}%</span>
            </div>
            <div class="metric-value mono">${esc(formatBytes(m.rss_bytes))}</div>
            <div class="progress"><div class="progress-bar ${rssBarCls}" style="width:${rssPct.toFixed(1)}%"></div></div>
            ${buildSparkline(this._rssHistory, 'var(--accent)')}
            <div class="metric-sub">
              <span>${esc(t('perf.budget'))}: ${esc(formatBytes(m.mem_budget_bytes))}</span>
              <span>${esc(m.profile)}</span>
            </div>
          </div>

          <div class="metric-card">
            <div class="metric-top">
              <span class="metric-label">${esc(t('perf.apiLatency'))}</span>
              <span class="badge badge-neutral mono">avg ${avgRtt} ms</span>
            </div>
            <div class="metric-value mono">${m.rtt_ms} ms</div>
            ${buildSparkline(this._rttHistory, 'var(--info)')}
            <div class="metric-sub">
              <span>${esc(t('perf.samples'))}: ${this._rttHistory.length}</span>
              <span>5s interval</span>
            </div>
          </div>

          <div class="metric-card">
            <div class="metric-top">
              <span class="metric-label">${esc(t('perf.systemLoad'))}</span>
              <span class="badge badge-neutral mono">${m.cpu_cores} vCPU</span>
            </div>
            <div class="metric-value mono">${m.load_1m.toFixed(2)}</div>
            ${buildSparkline(this._loadHistory, 'var(--success)')}
            <div class="metric-sub">
              <span class="mono">5m: ${m.load_5m.toFixed(2)} · 15m: ${m.load_15m.toFixed(2)}</span>
              <span>RAM ${hostPct.toFixed(0)}%</span>
            </div>
          </div>

          <div class="metric-card">
            <div class="metric-top">
              <span class="metric-label">${esc(t('perf.uptime'))}</span>
              <span class="badge badge-neutral mono">FD ${m.open_fds}</span>
            </div>
            <div class="metric-value mono">${esc(formatUptime(m.uptime_secs))}</div>
            <div class="progress"><div class="progress-bar ok" style="width:${Math.min(100, hostPct).toFixed(1)}%"></div></div>
            <div class="metric-sub">
              <span>${esc(t('perf.hostRam'))}</span>
              <span class="mono">${esc(formatBytes(hostUsed))} / ${esc(formatBytes(m.mem_total_bytes))}</span>
            </div>
          </div>
        </div>
        <div class="telemetry-strip">
          <div class="telemetry-pills">
            <span class="muted">${esc(t('perf.httpTelemetry'))}:</span>
            <span class="badge badge-success mono">2xx ${esc(m.http_requests_2xx || 0)}</span>
            <span class="badge badge-neutral mono">3xx ${esc(m.http_requests_3xx || 0)}</span>
            <span class="badge badge-warn mono">4xx ${esc(m.http_requests_4xx || 0)}</span>
            <span class="badge ${(m.http_requests_5xx || 0) > 0 ? 'badge-warn' : 'badge-neutral'} mono">5xx ${esc(m.http_requests_5xx || 0)}</span>
            <span class="badge badge-neutral mono">avg ${esc(m.http_avg_latency_ms || 0)} ms</span>
            <span class="badge badge-neutral mono">inflight ${esc(m.active_http_requests || 0)}</span>
            <span class="badge badge-neutral mono">${esc(t('perf.rateLimited'))} ${esc(m.rate_limited_total || 0)}</span>
            <span class="badge badge-neutral mono">${esc(t('perf.workerErrors'))} ${esc(m.worker_errors_total || 0)} / OOM ${esc(m.worker_oom_total || 0)}</span>
          </div>
          <button type="button" class="btn mono" id="perf-copy-prom">/metrics/prometheus</button>
        </div>
      `;
    }

    this.shadowRoot.innerHTML = `
      <section class="panel" aria-label="${esc(t('perf.title'))}">
        <div class="panel-header">
          <h3 class="panel-title">
            <span>${esc(t('perf.title'))}</span>
            ${statusBadge}
          </h3>
          <div class="row">
            <button type="button" class="btn" id="perf-toggle-btn">${esc(this._paused ? t('perf.resume') : t('perf.pause'))}</button>
            <button type="button" class="btn" id="perf-refresh-btn">${esc(t('common.refresh'))}</button>
          </div>
        </div>
        ${bodyHtml}
      </section>
    `;

    const toggleBtn = this.shadowRoot.getElementById('perf-toggle-btn');
    if (toggleBtn) {
      toggleBtn.addEventListener('click', () => {
        this._paused = !this._paused;
        this.render();
      });
    }
    const refreshBtn = this.shadowRoot.getElementById('perf-refresh-btn');
    if (refreshBtn) {
      refreshBtn.addEventListener('click', () => this.refresh());
    }
    const copyPromBtn = this.shadowRoot.getElementById('perf-copy-prom');
    if (copyPromBtn) {
      copyPromBtn.addEventListener('click', () => {
        const url = window.location.origin + '/_api/v1/metrics/prometheus';
        if (navigator.clipboard && navigator.clipboard.writeText) {
          navigator.clipboard.writeText(url).then(() => {
            this.dispatchEvent(new CustomEvent('fp-toast', {
              detail: { message: getI18n().t('common.copied'), kind: 'success' },
              bubbles: true,
              composed: true
            }));
          }).catch(() => {});
        }
      });
    }
  }
}

if (!customElements.get('fp-perf-monitor')) {
  customElements.define('fp-perf-monitor', FpPerfMonitor);
}
