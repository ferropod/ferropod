(function () {
  'use strict';

  var I18n = window.FerroI18n;
  var t = I18n.t;
  var formatTime = I18n.formatTime;
  var formatBytes = I18n.formatBytes;

  var TOKEN_STORAGE = 'ferropod_token';
  var THEME_STORAGE = 'ferropod_theme';

  var ROUTES = [
    'overview',
    'apps',
    'kv',
    'queues',
    'blobs',
    'dbs',
    'search',
    'ai',
    'tokens',
    'logs',
    'ops',
    'updates',
    'settings'
  ];

  var state = {
    token: '',
    status: null,
    projects: [],
    routes: [],
    kvNamespaces: [],
    queues: [],
    buckets: [],
    dbs: [],
    tokens: [],
    currentRoute: 'overview',
    projectId: null,
    projectTab: 'deployments',
    envDeploymentId: null,
    selectedKvNs: '',
    kvKeys: [],
    kvPrefix: '',
    selectedQueue: '',
    queueStats: null,
    queueJobs: [],
    queueInspectStatus: 'dead',
    selectedBucket: '',
    blobObjects: [],
    blobPrefix: '',
    selectedDbId: '',
    dbTab: 'console',
    dbSchema: '',
    dbResult: null,
    dbSql: 'SELECT name, type FROM sqlite_master ORDER BY type, name;',
    dbParams: '[]',
    logsTab: 'live',
    liveLogs: [],
    liveFilterLevel: '',
    liveFilterSource: '',
    liveFilterSearch: '',
    sseSource: null,
    sseStatus: 'disconnected', // disconnected | connected | paused
    historyLogs: [],
    historyFilters: {
      source: '',
      level: '',
      project_id: '',
      request_id: '',
      windowSecs: '86400',
      limit: '100'
    }
  };

  var viewRequestSeq = 0;
  var dbFtsRequestSeq = 0;
  var searchQuerySeq = 0;

  function beginViewRender(expectedRoute, extraCheck) {
    var seq = ++viewRequestSeq;
    var reqToken = state.token;
    return function () {
      return (
        state.token === reqToken &&
        viewRequestSeq === seq &&
        state.currentRoute === expectedRoute &&
        (!extraCheck || extraCheck())
      );
    };
  }

  function $(id) {
    return document.getElementById(id);
  }

  function esc(val) {
    if (val === null || val === undefined) return '';
    return String(val)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function enc(seg) {
    return encodeURIComponent(String(seg));
  }

  function encPath(key) {
    return String(key)
      .replace(/^\/+/, '')
      .split('/')
      .map(encodeURIComponent)
      .join('/');
  }

  function getScope() {
    return state.status && state.status.scope ? state.status.scope : '';
  }

  function isAdmin() {
    return getScope() === 'admin';
  }

  function isReadonly() {
    return getScope() === 'readonly';
  }

  function canWriteProject(pid) {
    var s = getScope();
    return s === 'admin' || (pid && s === 'project:' + pid);
  }

  function canWriteResource(ownerPid) {
    var s = getScope();
    return s === 'admin' || (ownerPid && s === 'project:' + ownerPid);
  }

  function projectNameById(pid) {
    if (!pid) return '';
    for (var i = 0; i < state.projects.length; i++) {
      if (state.projects[i].id === pid) return state.projects[i].name;
    }
    return pid;
  }

  function decodePayloadBytes(bytes) {
    if (typeof bytes === 'string') return bytes;
    if (!Array.isArray(bytes)) return '';
    try {
      var raw = new TextDecoder().decode(new Uint8Array(bytes));
      try {
        return JSON.stringify(JSON.parse(raw), null, 2);
      } catch (_) {
        return raw;
      }
    } catch (_) {
      return JSON.stringify(bytes);
    }
  }

  // --- Theme & Language ---

  function detectTheme() {
    try {
      var saved = localStorage.getItem(THEME_STORAGE);
      if (saved === 'light' || saved === 'dark') return saved;
    } catch (_) {}
    if (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches) {
      return 'dark';
    }
    return 'light';
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute('data-theme', theme);
    try {
      localStorage.setItem(THEME_STORAGE, theme);
    } catch (_) {}
    var sun = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>';
    var moon = '<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>';
    var icon = theme === 'dark' ? sun : moon;
    var label = theme === 'dark' ? t('theme.light') : t('theme.dark');
    ['login-theme-btn', 'topbar-theme-btn'].forEach(function (id) {
      var el = $(id);
      if (el) {
        el.innerHTML = icon;
        el.title = label + ' (' + t('theme.toggle') + ')';
      }
    });
  }

  function toggleTheme() {
    var cur = document.documentElement.getAttribute('data-theme') || 'light';
    applyTheme(cur === 'dark' ? 'light' : 'dark');
  }

  function populateLangSelects() {
    var locales = I18n.LOCALES || [
      { code: 'en', label: 'English' },
      { code: 'zh-CN', label: '简体中文' }
    ];
    document.querySelectorAll('[data-action="select-lang"]').forEach(function (sel) {
      sel.innerHTML = '';
      locales.forEach(function (loc) {
        var opt = document.createElement('option');
        opt.value = loc.code;
        opt.textContent = loc.label;
        sel.appendChild(opt);
      });
    });
  }

  function applyDocumentTitle() {
    if (typeof document === 'undefined') return;
    var baseTitle = t('app.pageTitle');
    var inst = state.status && state.status.instance_name ? String(state.status.instance_name).trim() : '';
    document.title = inst ? (inst + ' · ' + baseTitle) : baseTitle;
  }

  function applyStaticI18n() {
    applyDocumentTitle();
    var nodes = document.querySelectorAll('[data-i18n]');
    nodes.forEach(function (el) {
      el.textContent = t(el.getAttribute('data-i18n'));
    });
    var phs = document.querySelectorAll('[data-i18n-placeholder]');
    phs.forEach(function (el) {
      el.setAttribute('placeholder', t(el.getAttribute('data-i18n-placeholder')));
    });
    var curLocale = I18n.getLocale();
    document.querySelectorAll('[data-action="select-lang"]').forEach(function (sel) {
      sel.value = curLocale;
      sel.setAttribute('aria-label', t('lang.label'));
    });
    var tokBtn = $('toggle-token-vis');
    var tokInp = $('token-input');
    if (tokBtn && tokInp) {
      tokBtn.textContent = tokInp.type === 'text' ? t('auth.hideToken') : t('auth.showToken');
    }
    applyTheme(document.documentElement.getAttribute('data-theme') || detectTheme());
  }

  async function selectLang(lang) {
    var applied = I18n.changeLocale ? await I18n.changeLocale(lang) : I18n.setLocale(lang);
    applyStaticI18n();
    if (state.status) {
      renderCurrentRoute(false);
    }
    return applied;
  }

  // --- Toast & Modal ---

  function toast(message, kind) {
    var root = $('toast-root');
    if (!root) return;
    var el = document.createElement('div');
    el.className = 'toast' + (kind ? ' toast-' + kind : '');
    el.textContent = message;
    root.appendChild(el);
    setTimeout(function () {
      if (el.parentNode) el.parentNode.removeChild(el);
    }, 3400);
  }

  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(
        function () { toast(t('common.copied'), 'success'); },
        function () { fallbackCopy(text); }
      );
    } else {
      fallbackCopy(text);
    }
  }

  function fallbackCopy(text) {
    var ta = document.createElement('textarea');
    ta.value = text;
    ta.style.position = 'fixed';
    ta.style.left = '-9999px';
    document.body.appendChild(ta);
    ta.select();
    try {
      document.execCommand('copy');
      toast(t('common.copied'), 'success');
    } catch (_) {}
    document.body.removeChild(ta);
  }

  function openModal(opts) {
    var root = $('modal-root');
    root.innerHTML =
      '<div class="modal-card" role="dialog" aria-modal="true">' +
      '<div class="modal-head">' +
      '<h3 class="modal-title">' + esc(opts.title) + '</h3>' +
      '<button type="button" class="btn btn-ghost btn-icon" id="modal-close-x" aria-label="Close">' +
      '<svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M18 6L6 18M6 6l12 12"/></svg>' +
      '</button></div>' +
      '<div class="modal-body" id="modal-body-slot">' + opts.body + '</div>' +
      (opts.footer ? '<div class="modal-foot" id="modal-foot-slot">' + opts.footer + '</div>' : '') +
      '</div>';
    root.classList.remove('hidden');
    root.setAttribute('aria-hidden', 'false');

    $('modal-close-x').addEventListener('click', closeModal);
    root.onclick = function (ev) {
      if (ev.target === root) closeModal();
    };
    if (opts.onMount) {
      opts.onMount(root);
    }
  }

  function closeModal() {
    var root = $('modal-root');
    if (!root) return;
    root.classList.add('hidden');
    root.setAttribute('aria-hidden', 'true');
    root.innerHTML = '';
  }

  function confirmAction(message, onConfirm) {
    openModal({
      title: t('common.confirmDeleteTitle'),
      body: '<p style="margin:0;">' + esc(message) + '</p>',
      footer:
        '<button type="button" class="btn" id="confirm-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="button" class="btn btn-danger" id="confirm-ok">' + esc(t('common.confirm')) + '</button>',
      onMount: function () {
        $('confirm-cancel').addEventListener('click', closeModal);
        $('confirm-ok').addEventListener('click', function () {
          closeModal();
          onConfirm();
        });
      }
    });
  }

  // --- Token Persistence Helpers ---

  function readStoredToken() {
    var tok = '';
    try { tok = localStorage.getItem(TOKEN_STORAGE) || ''; } catch (_) {}
    if (!tok) {
      try { tok = sessionStorage.getItem(TOKEN_STORAGE) || ''; } catch (_) {}
    }
    return tok;
  }

  function saveStoredToken(tok) {
    try { localStorage.setItem(TOKEN_STORAGE, tok); } catch (_) {}
    try { sessionStorage.setItem(TOKEN_STORAGE, tok); } catch (_) {}
  }

  function clearStoredToken() {
    try { localStorage.removeItem(TOKEN_STORAGE); } catch (_) {}
    try { sessionStorage.removeItem(TOKEN_STORAGE); } catch (_) {}
  }

  // --- API Client ---

  async function api(path, opts) {
    opts = opts || {};
    var headers = Object.assign({}, opts.headers || {});
    var tok = state.token || readStoredToken();
    if (tok) {
      state.token = tok;
      headers.Authorization = 'Bearer ' + tok;
    }
    var fetchOpts = {
      method: opts.method || 'GET',
      headers: headers
    };
    if (opts.json !== undefined) {
      headers['Content-Type'] = 'application/json';
      fetchOpts.body = JSON.stringify(opts.json);
    } else if (opts.body !== undefined) {
      fetchOpts.body = opts.body;
    }

    var res = await fetch(path, fetchOpts);
    var text = await res.text();
    var parsedErr = '';
    if (!res.ok && text) {
      try {
        var parsedObj = JSON.parse(text);
        if (parsedObj && parsedObj.error) parsedErr = parsedObj.error;
      } catch (_) {
        parsedErr = text;
      }
    }

    if (res.status === 401 && !opts.skipAuthRedirect) {
      // Distinguish genuine invalid/expired token from scope/permission 401 on a specific resource
      var stillValid = false;
      if (state.status && path !== '/_api/v1/status') {
        try {
          var probe = await fetch('/_api/v1/status', {
            headers: state.token ? { Authorization: 'Bearer ' + state.token } : {}
          });
          stillValid = probe.ok;
        } catch (_) {}
      }
      if (!stillValid) {
        disconnect(true);
        throw new Error(parsedErr || t('auth.invalidToken'));
      }
      throw new Error(parsedErr || ('401 ' + res.statusText));
    }

    if (!res.ok) {
      throw new Error(parsedErr || (res.status + ' ' + res.statusText));
    }
    if (!text || res.status === 204) return null;
    try {
      return JSON.parse(text);
    } catch (_) {
      return text;
    }
  }

  // --- Drawer & Navigation ---

  function openDrawer() {
    $('sidebar').classList.add('open');
    $('drawer-backdrop').classList.add('open');
  }

  function closeDrawer() {
    $('sidebar').classList.remove('open');
    $('drawer-backdrop').classList.remove('open');
  }

  function parseHash() {
    var raw = (window.location.hash || '#/overview').replace(/^#\/?/, '');
    var parts = raw.split('/').filter(Boolean);
    var route = parts[0] || 'overview';
    if (route === 'projects' || route === 'runtime') {
      var suffix = parts[1] ? '/' + parts[1] + (parts[2] ? '/' + parts[2] : '') : '';
      history.replaceState(null, '', '#/apps' + suffix);
      route = 'apps';
    } else if (route === 'routes') {
      history.replaceState(null, '', '#/apps');
      route = 'apps';
      parts = ['apps'];
    }
    if (ROUTES.indexOf(route) === -1) route = 'overview';
    if (route === 'tokens' && !isAdmin()) route = 'overview';
    if (route === 'settings' && !isAdmin() && !isReadonly()) route = 'overview';
    state.currentRoute = route;
    var runtimeId = route === 'apps' && parts[1] ? decodeURIComponent(parts[1]) : null;
    if (runtimeId !== state.projectId) {
      state.projectTab = 'deployments';
      state.envDeploymentId = null;
    }
    var hashTab = route === 'apps' && parts[2] ? decodeURIComponent(parts[2]) : null;
    if (['deployments', 'bindings', 'secrets', 'routes'].indexOf(hashTab) !== -1) {
      state.projectTab = hashTab;
    }
    if (state.pendingRuntimeTab) state.projectTab = state.pendingRuntimeTab;
    state.pendingRuntimeTab = null;
    state.projectId = runtimeId;
  }

  function navigate(hash) {
    viewRequestSeq++;
    if (window.location.hash !== hash) {
      try {
        history.pushState(null, '', hash);
      } catch (_) {
        window.location.hash = hash;
      }
    }
    parseHash();
    closeDrawer();
    if (state.status) {
      renderCurrentRoute(true);
    }
  }

  function updateNavActive() {
    var navTok = $('nav-tokens');
    if (navTok) {
      navTok.classList.toggle('hidden', !isAdmin());
    }
    var navSet = $('nav-settings');
    if (navSet) {
      navSet.classList.toggle('hidden', !isAdmin() && !isReadonly());
    }
    document.querySelectorAll('[data-nav]').forEach(function (el) {
      el.classList.toggle('active', el.getAttribute('data-nav') === state.currentRoute);
    });
    var titleKey = 'nav.' + state.currentRoute;
    $('topbar-title').textContent = t(titleKey);
  }

  // --- Session & Auth ---

  async function connectWithToken(token, silentBoot) {
    var clean = (token || '').trim();
    if (!clean) {
      showLoginError(t('auth.tokenRequired'));
      return;
    }
    var btn = $('login-submit');
    btn.disabled = true;
    btn.textContent = t('auth.connecting');
    hideLoginError();

    if (silentBoot) {
      $('login-view').classList.add('hidden');
      $('app-shell').classList.remove('hidden');
      var vr = $('view-root');
      if (vr) vr.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    }

    state.token = clean;
    try {
      var st = await api('/_api/v1/status', { skipAuthRedirect: true });
      state.status = st;
      saveStoredToken(clean);
      enterApp();
    } catch (err) {
      state.token = '';
      state.status = null;
      if (silentBoot) {
        $('app-shell').classList.add('hidden');
        $('login-view').classList.remove('hidden');
      }
      showLoginError(err.message || t('auth.invalidToken'));
    } finally {
      btn.disabled = false;
      btn.textContent = t('auth.connect');
    }
  }

  function showLoginError(msg) {
    var el = $('login-error');
    el.textContent = msg;
    el.classList.remove('hidden');
  }

  function hideLoginError() {
    var el = $('login-error');
    el.textContent = '';
    el.classList.add('hidden');
  }

  function enterApp() {
    $('login-view').classList.add('hidden');
    $('app-shell').classList.remove('hidden');
    $('session-actor').textContent = state.status.actor || '-';
    $('session-scope').textContent = state.status.scope || '-';
    if ($('session-version')) {
      $('session-version').textContent = state.status.version ? 'v' + state.status.version : 'v-';
    }
    applyDocumentTitle();
    parseHash();
    renderCurrentRoute(true);
  }

  function disconnect(keepError) {
    stopSse();
    state.token = '';
    state.status = null;
    clearStoredToken();
    applyDocumentTitle();
    $('app-shell').classList.add('hidden');
    $('login-view').classList.remove('hidden');
    if (!keepError) hideLoginError();
  }

  // --- Render Helpers ---

  function renderBadgesForEnv(env) {
    if (env === 'prod' || env === 'active') return '<span class="badge badge-success">' + esc(env) + '</span>';
    if (env === 'preview') return '<span class="badge badge-warn">' + esc(env) + '</span>';
    return '<span class="badge badge-neutral">' + esc(env || 'ready') + '</span>';
  }

  function resourceGroupField(presetValue) {
    return '<div class="field"><label for="resource-group">' + esc(t('groups.label')) + '</label>' +
      '<input id="resource-group" list="resource-group-options" maxlength="80" value="' + esc(presetValue || '') + '" placeholder="' +
      esc(t('groups.placeholder')) + '"><datalist id="resource-group-options"></datalist></div>';
  }

  function loadResourceGroupOptions() {
    var options = $('resource-group-options');
    api('/_api/v1/resource-groups').then(function (groups) {
      if (options.isConnected) options.innerHTML = (groups || []).map(function (name) {
        return '<option value="' + esc(name) + '"></option>';
      }).join('');
    }).catch(function (e) { toast(e.message, 'error'); });
  }

  function selectedResourceGroup() {
    return $('resource-group').value.trim() || null;
  }

  function groupedMenu(items, renderItem) {
    var groups = new Map();
    items.forEach(function (item) {
      var name = item.group || '';
      if (!groups.has(name)) groups.set(name, []);
      groups.get(name).push(item);
    });
    if (groups.size === 1 && groups.has('')) return items.map(renderItem).join('');
    return Array.from(groups.keys()).sort().map(function (name) {
      return '<div class="nav-group-label">' + esc(name || t('groups.ungrouped')) + '</div>' +
        groups.get(name).map(renderItem).join('');
    }).join('');
  }

  function renderProjectSelectOptions(selectedId, includeGlobal) {
    var html = includeGlobal
      ? '<option value="">' + esc(t('common.allProjects')) + '</option>'
      : '';
    state.projects.forEach(function (p) {
      html += '<option value="' + esc(p.id) + '"' + (p.id === selectedId ? ' selected' : '') + '>' + esc(p.name) + ' (' + esc(p.id) + ')</option>';
    });
    return html;
  }

  function renderCurrentRoute(fetchData) {
    updateNavActive();
    if (state.currentRoute === 'overview') return renderOverview(fetchData);
    if (state.currentRoute === 'apps') {
      return state.projectId ? renderProjectDetail(state.projectId, fetchData) : renderRuntime();
    }
    if (state.currentRoute === 'kv') return renderKv(fetchData);
    if (state.currentRoute === 'queues') return renderQueues(fetchData);
    if (state.currentRoute === 'blobs') return renderBlobs(fetchData);
    if (state.currentRoute === 'dbs') return renderDbs(fetchData);
    if (state.currentRoute === 'search') return renderSearch(fetchData);
    if (state.currentRoute === 'ai') return renderAi(fetchData);
    if (state.currentRoute === 'tokens') return renderTokens(fetchData);
    if (state.currentRoute === 'logs') return renderLogs(fetchData);
    if (state.currentRoute === 'ops') return renderOps(fetchData);
    if (state.currentRoute === 'updates') return renderUpdates(fetchData);
    if (state.currentRoute === 'settings') return renderSettings();
  }

  // --- Lazy Native ESM Component Loaders ---

  var perfModulePromise = null;
  function ensurePerfComponent() {
    if (!perfModulePromise) {
      perfModulePromise = import('/_panel/assets/perf_component.js').catch(function (err) {
        perfModulePromise = null;
        throw err;
      });
    }
    return perfModulePromise;
  }

  var searchBundlePromise = null;
  function ensureSearchBundle() {
    if (!searchBundlePromise) {
      searchBundlePromise = import('/_panel/assets/search_components.js').catch(function (err) {
        searchBundlePromise = null;
        throw err;
      });
    }
    return searchBundlePromise;
  }

  var opsModulePromise = null;
  function ensureOpsComponent() {
    if (!opsModulePromise) {
      opsModulePromise = import('/_panel/assets/ops_component.js').catch(function (err) {
        opsModulePromise = null;
        throw err;
      });
    }
    return opsModulePromise;
  }

  var runtimeModulePromise = null;
  function ensureRuntimeComponent() {
    if (!runtimeModulePromise) {
      runtimeModulePromise = import('/_panel/assets/runtime_component.js').catch(function (err) {
        runtimeModulePromise = null;
        throw err;
      });
    }
    return runtimeModulePromise;
  }

  var updateModulePromise = null;
  function ensureUpdateComponent() {
    if (!updateModulePromise) {
      updateModulePromise = import('/_panel/assets/update_component.js').catch(function (err) {
        updateModulePromise = null;
        throw err;
      });
    }
    return updateModulePromise;
  }

  var settingsModulePromise = null;
  function ensureSettingsComponent() {
    if (!settingsModulePromise) {
      settingsModulePromise = import('/_panel/assets/settings_component.js').catch(function (err) {
        settingsModulePromise = null;
        throw err;
      });
    }
    return settingsModulePromise;
  }

  var aiModulePromise = null;
  function ensureAiComponent() {
    if (!aiModulePromise) {
      aiModulePromise = import('/_panel/assets/ai_component.js').catch(function (err) {
        aiModulePromise = null;
        throw err;
      });
    }
    return aiModulePromise;
  }

  // --- 1. Overview ---

  async function renderOverview(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('overview');
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var results = await Promise.all([
          api('/_api/v1/status'),
          api('/_healthz'),
          api('/_api/v1/apps'),
          api('/_api/v1/routes')
        ]);
        if (!isCurrent()) return;
        state.status = results[0];
        state.healthOk = results[1] && results[1].status === 'ok';
        state.projects = results[2] || [];
        state.routes = results[3] || [];
        $('session-actor').textContent = state.status.actor || '-';
        $('session-scope').textContent = state.status.scope || '-';
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;
    var st = state.status || {};
    var healthBadge = state.healthOk !== false
      ? '<span class="badge badge-success"><span class="dot"></span>' + esc(t('overview.healthy')) + '</span>'
      : '<span class="badge badge-danger"><span class="dot"></span>' + esc(t('overview.unhealthy')) + '</span>';

    var projRows = state.projects.slice(0, 5).map(function (p) {
      return '<tr>' +
        '<td data-label="' + esc(t('common.name')) + '"><a href="#/apps/' + enc(p.id) + '" class="text-strong">' + esc(p.name) + '</a></td>' +
        '<td data-label="' + esc(t('projects.activeDeployment')) + '" class="mono text-xs">' + esc(p.active_deployment_id || '-') + '</td>' +
        '<td data-label="' + esc(t('projects.previewDeployment')) + '" class="mono text-xs">' + esc(p.preview_deployment_id || '-') + '</td>' +
        '</tr>';
    }).join('');

    var routeRows = state.routes.slice(0, 5).map(function (r) {
      var appHref = '#/apps/' + enc(r.project_id);
      return '<tr>' +
        '<td data-label="' + esc(t('routes.host')) + '" class="mono text-xs"><a href="' + appHref + '" data-runtime-tab="routes" class="text-strong">' + esc(r.host) + '</a></td>' +
        '<td data-label="' + esc(t('routes.project')) + '"><a href="' + appHref + '" data-runtime-tab="routes">' + esc(projectNameById(r.project_id)) + '</a></td>' +
        '<td data-label="' + esc(t('routes.targetEnv')) + '">' + renderBadgesForEnv(r.target_env) + '</td>' +
        '</tr>';
    }).join('');

    root.innerHTML =
      '<div class="page-head">' +
      '<div><h2 class="page-title">' + esc(t('overview.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('overview.subtitle')) + '</p></div>' +
      '<div class="row">' + healthBadge + '</div></div>' +

      '<div class="panel"><div class="panel-body row-between">' +
      '<div class="row gap-lg">' +
      '<div><div class="muted text-xs">' + esc(t('updates.currentVersion')) + '</div><div class="text-strong mono"><a href="#/updates">v' + esc(st.version || '-') + '</a></div></div>' +
      '<div><div class="muted text-xs">' + esc(t('overview.profile')) + '</div><div class="text-strong">' + esc(st.profile || '-') + '</div></div>' +
      '<div><div class="muted text-xs">' + esc(t('overview.actor')) + '</div><div class="text-strong">' + esc(st.actor || '-') + '</div></div>' +
      '<div><div class="muted text-xs">' + esc(t('overview.scope')) + '</div><div><span class="badge badge-accent">' + esc(st.scope || '-') + '</span></div></div>' +
      '</div>' +
      '<div class="row">' +
      '<a href="#/apps" class="btn btn-sm">' + esc(t('nav.apps')) + '</a>' +
      '<a href="#/logs" class="btn btn-sm">' + esc(t('nav.logs')) + '</a>' +
      '<a href="#/updates" class="btn btn-sm">' + esc(t('nav.updates')) + '</a>' +
      '</div></div></div>' +

      '<div class="stats-grid">' +
      statCard(t('overview.projects'), st.projects, '#/apps') +
      statCard(t('overview.routes'), st.routes, '#/apps') +
      statCard(t('overview.kvNamespaces'), st.kv_namespaces, '#/kv') +
      statCard(t('overview.queues'), st.queues, '#/queues') +
      statCard(t('overview.buckets'), st.buckets, '#/blobs') +
      statCard(t('overview.dbs'), st.dbs, '#/dbs') +
      statCard(t('overview.searchIndexes'), st.search_indexes || 0, '#/search') +
      statCard(t('overview.aiProviders'), st.ai_providers || 0, '#/ai') +
      '</div>' +

      '<fp-perf-monitor id="overview-perf-monitor"></fp-perf-monitor>' +

      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('overview.endpointsTitle')) + '</h3><span class="badge badge-info">AI &amp; API</span></div>' +
      '<div class="panel-body row gap-xs">' +
      [
        { label: 'MCP', path: '/_mcp' },
        { label: 'AI Chat', path: '/_ai/v1/chat/completions' },
        { label: 'AI Models', path: '/_ai/v1/models' },
        { label: 'Realtime SSE', path: '/_realtime/{channel}/sse' },
        { label: 'OpenAPI', path: '/_api/v1/openapi.json' },
        { label: 'llms.txt', path: '/_api/v1/llms.txt' },
        { label: 'Prometheus', path: '/_api/v1/metrics/prometheus' }
      ].map(function (ep) {
        return '<button type="button" class="btn btn-sm" data-copy-endpoint="' + esc(ep.path) + '">' + esc(ep.label) + ': <code class="mono">' + esc(ep.path) + '</code></button>';
      }).join('') +
      '</div></div>' +

      '<div class="grid-2">' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('overview.recentProjects')) + '</h3><a href="#/apps" class="btn btn-ghost btn-sm">' + esc(t('common.view')) + '</a></div>' +
      (projRows
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('common.name')) + '</th><th>' + esc(t('projects.activeDeployment')) + '</th><th>' + esc(t('projects.previewDeployment')) + '</th></tr></thead><tbody>' + projRows + '</tbody></table></div>'
        : '<div class="empty-state">' + esc(t('common.empty')) + '</div>') +
      '</div>' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('overview.recentRoutes')) + '</h3><a href="#/apps" class="btn btn-ghost btn-sm">' + esc(t('common.view')) + '</a></div>' +
      (routeRows
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('routes.host')) + '</th><th>' + esc(t('routes.project')) + '</th><th>' + esc(t('routes.targetEnv')) + '</th></tr></thead><tbody>' + routeRows + '</tbody></table></div>'
        : '<div class="empty-state">' + esc(t('common.empty')) + '</div>') +
      '</div></div>';

    root.querySelectorAll('[data-copy-endpoint]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var path = btn.getAttribute('data-copy-endpoint');
        copyText(window.location.origin + path);
      });
    });

    if ($('overview-perf-monitor')) {
      var perfEl = $('overview-perf-monitor');
      ensurePerfComponent().then(function () {
        perfEl.token = state.token || readStoredToken();
      }).catch(function () {});
    }
  }

  function statCard(label, value, hash) {
    return '<a href="' + hash + '" class="stat-card clickable">' +
      '<span class="stat-label">' + esc(label) + '</span>' +
      '<span class="stat-value">' + esc(value !== undefined ? value : 0) + '</span>' +
      '</a>';
  }

  // --- 2. Runtime details ---

  function openCreateRuntimeModal() {
    openModal({
      title: t('runtime.newRuntime'),
      body:
        '<form id="create-proj-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('runtime.runtimeName')) + '</label>' +
        '<input type="text" id="cp-name" required placeholder="' + esc(t('projects.namePlaceholder')) + '"></div>' +
        resourceGroupField() +
        '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="field"><label>' + esc(t('common.description')) + '</label>' +
        '<input type="text" id="cp-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('projects.healthCheckPath')) + '</label>' +
        '<input type="text" id="cp-hc" placeholder="' + esc(t('projects.healthCheckPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('projects.maxConcurrency')) + '</label>' +
        '<input type="number" id="cp-conc" min="1" value="64"></div>' +
        '<div class="field"><label>' + esc(t('projects.timeoutMs')) + '</label>' +
        '<input type="number" id="cp-timeout" min="10" placeholder="' + esc(t('projects.timeoutPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('projects.wasmMaxMemoryMb')) + '</label>' +
        '<input type="number" id="cp-mem" min="1" placeholder="' + esc(t('projects.wasmMaxMemoryPlaceholder')) + '"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="cp-priv"> ' + esc(t('projects.allowPrivateOutbound')) + '</label>' +
        '</div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="cp-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="create-proj-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('cp-cancel').addEventListener('click', closeModal);
        $('create-proj-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var name = $('cp-name').value.trim();
          var hc = $('cp-hc').value.trim();
          var desc = $('cp-desc').value.trim();
          var concRaw = $('cp-conc').value.trim();
          var timeoutRaw = $('cp-timeout').value.trim();
          var memRaw = $('cp-mem').value.trim();
          try {
            var created = await api('/_api/v1/apps', {
              method: 'POST',
              json: {
                name: name,
                group: selectedResourceGroup(),
                description: desc || null,
                health_check_path: hc || null,
                max_concurrency: concRaw ? Number(concRaw) : 64,
                timeout_ms: timeoutRaw ? Number(timeoutRaw) : null,
                wasm_max_memory_mb: memRaw ? Number(memRaw) : null,
                allow_private_outbound: $('cp-priv').checked ? true : null
              }
            });
            closeModal();
            toast(t('runtime.created'), 'success');
            navigate('#/apps/' + enc(created.id));
          } catch (e) {
            toast(e.message, 'error');
          }
        });
      }
    });
  }

  function openEditRuntimeModal(project) {
    openModal({
      title: t('common.editConfig') + ' · ' + project.name,
      body:
        '<form id="edit-proj-form" class="stack gap-md">' +
        resourceGroupField(project.group || '') +
        '<div class="field"><label>' + esc(t('common.description')) + '</label>' +
        '<input type="text" id="ep-desc" value="' + esc(project.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('projects.healthCheckPath')) + '</label>' +
        '<input type="text" id="ep-hc" value="' + esc(project.health_check_path || '/_health') + '"></div>' +
        '<div class="field"><label>' + esc(t('projects.maxConcurrency')) + '</label>' +
        '<input type="number" id="ep-conc" min="1" value="' + esc(project.max_concurrency || 64) + '"></div>' +
        '<div class="field"><label>' + esc(t('projects.timeoutMs')) + '</label>' +
        '<input type="number" id="ep-timeout" min="0" value="' + esc(project.timeout_ms || '') + '" placeholder="' + esc(t('projects.timeoutPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('projects.wasmMaxMemoryMb')) + '</label>' +
        '<input type="number" id="ep-mem" min="0" value="' + esc(project.wasm_max_memory_mb || '') + '" placeholder="' + esc(t('projects.wasmMaxMemoryPlaceholder')) + '"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="ep-priv"' + (project.allow_private_outbound ? ' checked' : '') + '> ' + esc(t('projects.allowPrivateOutbound')) + '</label>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="ep-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="edit-proj-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('ep-cancel').addEventListener('click', closeModal);
        $('edit-proj-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var timeoutRaw = $('ep-timeout').value.trim();
          var memRaw = $('ep-mem').value.trim();
          try {
            await api('/_api/v1/apps/' + enc(project.id), {
              method: 'PATCH',
              json: {
                group: $('resource-group').value.trim(),
                description: $('ep-desc').value.trim(),
                health_check_path: $('ep-hc').value.trim() || '/_health',
                max_concurrency: Number($('ep-conc').value) || 64,
                timeout_ms: timeoutRaw ? Number(timeoutRaw) : 0,
                wasm_max_memory_mb: memRaw ? Number(memRaw) : 0,
                allow_private_outbound: $('ep-priv').checked
              }
            });
            closeModal();
            toast(t('projects.updated'), 'success');
            renderProjectDetail(project.id, true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  async function renderProjectDetail(id, fetchData) {
    var root = $('view-root');
    var isCurrentDetail = beginViewRender('apps', function () {
      return state.projectId === id;
    });
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var reqs = [
          api('/_api/v1/apps/' + enc(id)),
          api('/_api/v1/apps/' + enc(id) + '/deployments'),
          api('/_api/v1/routes'),
          api('/_api/v1/buckets'),
          state.projects.length
            ? Promise.resolve(state.projects)
            : api('/_api/v1/apps').catch(function () { return []; })
        ];
        var res = await Promise.all(reqs);
        if (!isCurrentDetail()) return;
        var secrets = [];
        if (canWriteProject(res[0].id)) {
          try {
            secrets = (await api('/_api/v1/apps/' + enc(id) + '/secrets')) || [];
          } catch (_) {}
        }
        if (!isCurrentDetail()) return;
        state.detailProject = res[0];
        state.detailDeployments = res[1] || [];
        state.routes = res[2] || [];
        state.buckets = res[3] || [];
        if (Array.isArray(res[4]) && res[4].length) state.projects = res[4];
        state.detailSecrets = secrets;
      } catch (e) {
        if (isCurrentDetail()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }

    if (!isCurrentDetail()) return;
    var p = state.detailProject;
    if (!p) return;
    var writable = canWriteProject(p.id);
    var deps = state.detailDeployments || [];
    var projRoutes = state.routes.filter(function (r) { return r.project_id === p.id; });
    var secrets = state.detailSecrets || [];

    var headerHtml =
      '<div class="page-head">' +
      '<div><div class="row gap-xs" style="margin-bottom:6px;"><a href="#/apps" class="btn btn-sm">' + esc(t('common.back')) + '</a></div>' +
      '<h2 class="page-title">' + esc(p.name) + '</h2>' +
      '<p class="page-subtitle mono">' + esc(p.id) + (p.group ? ' · ' + esc(p.group) : '') + ' · ' + esc(t('projects.healthCheckPath')) + ': ' + esc(p.health_check_path) + (p.description ? ' · ' + esc(p.description) : '') + '</p></div>' +
      (writable
        ? '<div class="row">' +
          '<label class="checkbox-row" style="font-size:12px;"><input type="checkbox" id="pd-verify-health"> ' + esc(t('projectDetail.verifyHealth')) + '</label>' +
          (p.preview_deployment_id ? '<button type="button" class="btn" id="pd-promote-preview">' + esc(t('projectDetail.promotePreview')) + '</button>' : '') +
          (deps.length > 1 ? '<button type="button" class="btn" id="pd-rollback">' + esc(t('projectDetail.rollbackProd')) + '</button>' : '') +
          '<button type="button" class="btn" id="pd-edit-runtime">' + esc(t('common.editConfig')) + '</button>' +
          '<button type="button" class="btn btn-primary" id="pd-new-dep">' + esc(t('projectDetail.newDeployment')) + '</button>' +
          (isAdmin() ? '<button type="button" class="btn btn-danger" id="pd-delete-runtime">' + esc(t('common.delete')) + '</button>' : '') +
          '</div>'
        : '') +
      '</div>';

    var summaryHtml =
      '<div class="stats-grid">' +
      '<div class="stat-card"><span class="stat-label">' + esc(t('projects.activeDeployment')) + '</span><span class="mono text-sm truncate">' + esc(p.active_deployment_id || '-') + '</span></div>' +
      '<div class="stat-card"><span class="stat-label">' + esc(t('projects.previewDeployment')) + '</span><span class="mono text-sm truncate">' + esc(p.preview_deployment_id || '-') + '</span></div>' +
      '<div class="stat-card"><span class="stat-label">' + esc(t('projects.maxConcurrency')) + '</span><span class="stat-value">' + esc(p.max_concurrency) + '</span></div>' +
      '<div class="stat-card"><span class="stat-label">' + esc(t('common.createdAt')) + '</span><span class="text-sm">' + esc(formatTime(p.created_at)) + '</span></div>' +
      '</div>';

    var tabsHtml =
      '<div class="tabs">' +
      '<button type="button" class="tab-btn' + (state.projectTab === 'deployments' ? ' active' : '') + '" data-pd-tab="deployments">' + esc(t('projectDetail.tabDeployments')) + ' (' + deps.length + ')</button>' +
      '<button type="button" class="tab-btn' + (state.projectTab === 'bindings' ? ' active' : '') + '" data-pd-tab="bindings">' + esc(t('projectDetail.tabBindings')) + '</button>' +
      '<button type="button" class="tab-btn' + (state.projectTab === 'secrets' ? ' active' : '') + '" data-pd-tab="secrets">' + esc(t('projectDetail.tabSecrets')) + '</button>' +
      '<button type="button" class="tab-btn' + (state.projectTab === 'routes' ? ' active' : '') + '" data-pd-tab="routes">' + esc(t('projectDetail.tabRoutes')) + ' (' + projRoutes.length + ')</button>' +
      '</div>';

    var bodyHtml = '';
    if (state.projectTab === 'bindings') {
      bodyHtml = '<fp-project-bindings id="pd-bindings-panel"></fp-project-bindings>';
    } else if (state.projectTab === 'deployments') {
      var depRows = deps.map(function (d) {
        var bucketSrc = d.assets_bucket
          ? 'bucket://' + d.assets_bucket + (d.assets_prefix ? '/' + d.assets_prefix : '')
          : '';
        var src = bucketSrc && d.artifact_path
          ? bucketSrc + ' + ' + d.artifact_path
          : (bucketSrc || d.artifact_path || '-');
        var isCurrent = p.active_deployment_id === d.id;
        return '<tr>' +
          '<td data-label="' + esc(t('projectDetail.deployId')) + '" class="mono text-xs">' + esc(d.id) + '</td>' +
          '<td data-label="' + esc(t('common.status')) + '">' + renderBadgesForEnv(d.status) + '</td>' +
          '<td data-label="' + esc(t('projectDetail.source')) + '" class="mono text-xs break-all">' + esc(src) + '</td>' +
          '<td data-label="' + esc(t('projectDetail.spaFallback')) + '">' + esc(d.spa_fallback ? t('common.yes') : t('common.no')) + '</td>' +
          '<td data-label="' + esc(t('common.createdAt')) + '" class="text-xs muted">' + esc(formatTime(d.created_at)) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          (writable && !isCurrent
            ? '<button type="button" class="btn btn-sm" data-promote-dep="' + esc(d.id) + '">' + esc(t('projectDetail.promote')) + '</button>' +
              '<button type="button" class="btn btn-sm" data-rollback-dep="' + esc(d.id) + '">' + esc(t('projectDetail.rollback')) + '</button>'
            : '') +
          '</div></td></tr>';
      }).join('');
      bodyHtml =
        '<div class="panel">' +
        (depRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('projectDetail.deployId')) + '</th><th>' + esc(t('common.status')) + '</th><th>' + esc(t('projectDetail.source')) + '</th><th>' + esc(t('projectDetail.spaFallback')) + '</th><th>' + esc(t('common.createdAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + depRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('projectDetail.noDeployments')) + '</div>') +
        '</div>';
    } else if (state.projectTab === 'secrets') {
      var envHtml = renderDeploymentEnv(p, deps, writable);
      if (!writable) {
        bodyHtml = '<div class="alert alert-info">' + esc(t('projectDetail.secretsWriteRequired')) + '</div>';
      } else {
        var secRows = secrets.map(function (k) {
          return '<tr>' +
            '<td data-label="' + esc(t('projectDetail.secretKey')) + '" class="mono text-strong">' + esc(k) + '</td>' +
            '<td class="cell-actions-td"><div class="cell-actions">' +
            '<button type="button" class="btn btn-sm" data-edit-secret="' + esc(k) + '">' + esc(t('common.edit')) + '</button>' +
            '<button type="button" class="btn btn-danger btn-sm" data-del-secret="' + esc(k) + '">' + esc(t('common.delete')) + '</button>' +
            '</div></td></tr>';
        }).join('');
        bodyHtml =
          '<div class="panel">' +
          '<div class="panel-header"><div><h3 class="panel-title">' + esc(t('projectDetail.secretsTitle')) + '</h3><div class="muted text-xs">' + esc(t('projectDetail.secretsSubtitle')) + '</div></div></div>' +
          '<div class="panel-body">' +
          '<form id="secret-form" class="form-grid" style="margin-bottom:14px;">' +
          '<div class="field"><label>' + esc(t('projectDetail.secretKey')) + '</label><input type="text" id="sec-key" required placeholder="OPENAI_API_KEY"></div>' +
          '<div class="field"><label>' + esc(t('projectDetail.secretValue')) + '</label><input type="password" id="sec-val" required placeholder="••••••••"></div>' +
          '<div class="span-2 row"><button type="submit" class="btn btn-primary">' + esc(t('projectDetail.setSecret')) + '</button></div>' +
          '</form>' +
          (secRows
            ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('projectDetail.secretKey')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + secRows + '</tbody></table></div>'
            : '<div class="empty-state">' + esc(t('projectDetail.noSecrets')) + '</div>') +
          '</div></div>';
      }
      bodyHtml = '<div class="stack gap-md">' + envHtml + bodyHtml + '</div>';
    } else {
      var rRows = projRoutes.map(function (r) {
        return '<tr>' +
          '<td data-label="' + esc(t('routes.host')) + '" class="mono text-strong">' + esc(r.host) + '</td>' +
          '<td data-label="' + esc(t('routes.targetEnv')) + '">' + renderBadgesForEnv(r.target_env) + '</td>' +
          '<td data-label="' + esc(t('routes.policies')) + '">' + renderRoutePolicyBadges(r) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          (writable
            ? '<button type="button" class="btn btn-sm" data-edit-route="' + esc(r.host) + '">' + esc(t('common.edit')) + '</button>' +
              '<button type="button" class="btn btn-danger btn-sm" data-del-route="' + esc(r.host) + '">' + esc(t('common.delete')) + '</button>'
            : '') +
          '</div></td></tr>';
      }).join('');
      bodyHtml =
        '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('projectDetail.tabRoutes')) + '</h3>' +
        (writable ? '<button type="button" class="btn btn-sm btn-primary" id="pd-add-route">' + esc(t('projectDetail.addRoute')) + '</button>' : '') +
        '</div>' +
        (rRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('routes.host')) + '</th><th>' + esc(t('routes.targetEnv')) + '</th><th>' + esc(t('routes.policies')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + rRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('routes.empty')) + '</div>') +
        '</div>';
    }

    root.innerHTML = headerHtml + summaryHtml + tabsHtml + bodyHtml;

    root.querySelectorAll('[data-pd-tab]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        state.projectTab = btn.getAttribute('data-pd-tab');
        renderProjectDetail(p.id, false);
      });
    });

    if ($('pd-bindings-panel')) {
      var bp = $('pd-bindings-panel');
      var activeDep = deps.find(function (d) { return d.id === p.active_deployment_id; }) || deps[0] || null;
      Promise.all([
        ensureRuntimeComponent(),
        api('/_api/v1/dbs').catch(function () { return []; }),
        api('/_api/v1/kv').catch(function () { return []; }),
        api('/_api/v1/queues').catch(function () { return []; }),
        api('/_api/v1/buckets').catch(function () { return []; }),
        api('/_api/v1/search').catch(function () { return []; })
      ]).then(function (res) {
        bp.token = state.token || readStoredToken();
        bp.canWrite = writable;
        bp.project = p;
        bp.activeDeployment = activeDep;
        bp.resources = {
          dbs: res[1] || [],
          kvNamespaces: res[2] || [],
          queues: res[3] || [],
          buckets: res[4] || [],
          searchIndexes: res[5] || []
        };
      }).catch(function (e) { toast(e.message, 'error'); });
      bp.addEventListener('fp-bindings-saved', function () {
        renderProjectDetail(p.id, true);
      });
    }

    if ($('pd-new-dep')) $('pd-new-dep').addEventListener('click', function () { openNewDeploymentModal(p); });
    if ($('pd-edit-runtime')) $('pd-edit-runtime').addEventListener('click', function () { openEditRuntimeModal(p); });
    if ($('pd-delete-runtime')) {
      $('pd-delete-runtime').addEventListener('click', function () {
        confirmAction(t('projects.deleteConfirm', { name: p.name }), async function () {
          try {
            await api('/_api/v1/apps/' + enc(p.id), { method: 'DELETE' });
            toast(t('projects.deleted'), 'success');
            navigate('#/apps');
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    if ($('pd-promote-preview')) {
      $('pd-promote-preview').addEventListener('click', async function () {
        try {
          var verifyHealth = Boolean($('pd-verify-health') && $('pd-verify-health').checked);
          var dep = await api('/_api/v1/apps/' + enc(p.id) + '/promote', {
            method: 'POST',
            json: { verify_health: verifyHealth }
          });
          toast(t('projectDetail.promoteDone', { id: dep.id }), 'success');
          renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('pd-rollback')) {
      $('pd-rollback').addEventListener('click', async function () {
        try {
          var dep = await api('/_api/v1/apps/' + enc(p.id) + '/rollback', { method: 'POST', json: {} });
          toast(t('projectDetail.rollbackDone', { id: dep.id }), 'success');
          renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
      });
    }

    root.querySelectorAll('[data-promote-dep]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        var depId = btn.getAttribute('data-promote-dep');
        try {
          var verifyHealth = Boolean($('pd-verify-health') && $('pd-verify-health').checked);
          var dep = await api('/_api/v1/apps/' + enc(p.id) + '/promote', {
            method: 'POST',
            json: { deployment_id: depId, verify_health: verifyHealth }
          });
          toast(t('projectDetail.promoteDone', { id: dep.id }), 'success');
          renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
      });
    });
    root.querySelectorAll('[data-rollback-dep]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        var depId = btn.getAttribute('data-rollback-dep');
        try {
          var dep = await api('/_api/v1/apps/' + enc(p.id) + '/rollback', {
            method: 'POST',
            json: { deployment_id: depId }
          });
          toast(t('projectDetail.rollbackDone', { id: dep.id }), 'success');
          renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
      });
    });

    if ($('env-deployment')) {
      $('env-deployment').addEventListener('change', function (ev) {
        state.envDeploymentId = ev.target.value;
        renderProjectDetail(p.id, false);
      });
    }
    if ($('env-form')) {
      $('env-form').addEventListener('submit', async function (ev) {
        ev.preventDefault();
        var env;
        try {
          env = JSON.parse($('runtime-env-json').value);
          if (!env || Array.isArray(env) || typeof env !== 'object' ||
              Object.values(env).some(function (value) { return typeof value !== 'string'; })) {
            throw new Error();
          }
        } catch (_) {
          toast(t('projectDetail.invalidEnvJson'), 'error');
          return;
        }
        var submit = $('env-save');
        submit.disabled = true;
        try {
          var saved = await api('/_api/v1/apps/' + enc(p.id) + '/deployments/' +
            enc($('env-deployment').value) + '/env', { method: 'POST', json: { env: env } });
          state.envDeploymentId = saved.id;
          toast(t('projectDetail.envSaved'), 'success');
          await renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
        finally { submit.disabled = false; }
      });
    }

    if ($('secret-form')) {
      $('secret-form').addEventListener('submit', async function (ev) {
        ev.preventDefault();
        var key = $('sec-key').value.trim();
        var value = $('sec-val').value;
        try {
          await api('/_api/v1/apps/' + enc(p.id) + '/secrets', {
            method: 'POST',
            json: { key: key, value: value }
          });
          toast(t('projectDetail.secretSaved'), 'success');
          renderProjectDetail(p.id, true);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    root.querySelectorAll('[data-edit-secret]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        if ($('sec-key')) {
          $('sec-key').value = btn.getAttribute('data-edit-secret');
          $('sec-val').focus();
        }
      });
    });
    root.querySelectorAll('[data-del-secret]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var key = btn.getAttribute('data-del-secret');
        confirmAction(t('projectDetail.secretDeleteConfirm', { key: key }), async function () {
          try {
            await api('/_api/v1/apps/' + enc(p.id) + '/secrets/' + enc(key), { method: 'DELETE' });
            toast(t('projectDetail.secretDeleted'), 'success');
            renderProjectDetail(p.id, true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });

    if ($('pd-add-route')) {
      $('pd-add-route').addEventListener('click', function () {
        openAppRouteModal(p, deps, null, function () { renderProjectDetail(p.id, true); });
      });
    }
    root.querySelectorAll('[data-edit-route]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var host = btn.getAttribute('data-edit-route');
        var routeObj = projRoutes.find(function (r) { return r.host === host; }) || null;
        if (routeObj) {
          openAppRouteModal(p, deps, routeObj, function () { renderProjectDetail(p.id, true); });
        }
      });
    });
    root.querySelectorAll('[data-del-route]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var host = btn.getAttribute('data-del-route');
        confirmAction(t('routes.deleteConfirm', { host: host }), async function () {
          try {
            await api('/_api/v1/routes/' + enc(host), { method: 'DELETE' });
            toast(t('routes.deleted'), 'success');
            renderProjectDetail(p.id, true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });
  }

  function renderDeploymentEnv(runtime, deployments, writable) {
    var selected = deployments.find(function (item) { return item.id === state.envDeploymentId; }) ||
      deployments.find(function (item) { return item.id === runtime.active_deployment_id; }) ||
      deployments[0];
    var content = '<div class="empty-state">' + esc(t('projectDetail.noDeployments')) + '</div>';
    if (selected) {
      var options = deployments.map(function (item) {
        return '<option value="' + esc(item.id) + '"' + (item.id === selected.id ? ' selected' : '') +
          '>' + esc(item.id + ' · ' + item.status) + '</option>';
      }).join('');
      var env = (selected.bindings && selected.bindings.env) || {};
      content = '<form id="env-form" class="panel-body stack gap-sm">' +
        '<div class="field"><label for="env-deployment">' + esc(t('projectDetail.envSource')) + '</label>' +
        '<select id="env-deployment">' + options + '</select></div>' +
        '<div class="field"><label for="runtime-env-json">' + esc(t('projectDetail.envValues')) + '</label>' +
        '<textarea id="runtime-env-json" class="mono" rows="7" spellcheck="false"' +
        (writable ? '' : ' readonly') + '>' + esc(JSON.stringify(env, null, 2)) + '</textarea></div>' +
        (writable ? '<div><button type="submit" class="btn btn-primary" id="env-save">' +
          esc(t('projectDetail.saveEnvVersion')) + '</button></div>' : '') + '</form>';
    }
    return '<div class="panel"><div class="panel-header"><div><h3 class="panel-title">' +
      esc(t('bindings.envTitle')) + '</h3><div class="muted text-xs">' +
      esc(t('projectDetail.envVersionHint')) + '</div></div></div>' + content + '</div>';
  }

  function openNewDeploymentModal(project) {
    var base =
      state.detailDeployments.find(function (d) { return d.id === project.active_deployment_id; }) ||
      state.detailDeployments.find(function (d) { return d.id === project.preview_deployment_id; });
    var availableBuckets = state.buckets.filter(function (b) {
      return !b.project_id || b.project_id === project.id;
    });
    var bktOpts = '<option value="">' + esc(t('projectDetail.assetsBucketNone')) + '</option>' +
      availableBuckets.map(function (b) {
        return '<option value="' + esc(b.name) + '"' + (base && base.assets_bucket === b.name ? ' selected' : '') + '>' + esc(b.name) + '</option>';
      }).join('');
    if (base && base.assets_bucket && !availableBuckets.some(function (b) { return b.name === base.assets_bucket; })) {
      bktOpts += '<option value="' + esc(base.assets_bucket) + '" selected>' + esc(base.assets_bucket) + '</option>';
    }

    openModal({
      title: t('projectDetail.newDeployment'),
      body:
        '<form id="new-dep-form" class="form-grid">' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.targetEnv')) + '</label>' +
        '<select id="dep-env"><option value="prod">' + esc(t('projectDetail.envProd')) + '</option>' +
        '<option value="preview">' + esc(t('projectDetail.envPreview')) + '</option>' +
        '<option value="">' + esc(t('projectDetail.envReady')) + '</option></select></div>' +
        '<div class="field"><label>' + esc(t('projectDetail.assetsBucket')) + '</label><select id="dep-bucket">' + bktOpts + '</select></div>' +
        '<div class="field"><label>' + esc(t('projectDetail.assetsPrefix')) + '</label><input type="text" id="dep-prefix" placeholder="dist"></div>' +
        '<div class="field span-2"><label for="dep-file">' + esc(t('projectDetail.artifactFile')) + '</label><input type="file" id="dep-file" accept=".wasm,.wat"><span class="muted text-xs">' + esc(t('projectDetail.artifactFileHint')) + '</span></div>' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.artifactPath')) + ' (' + esc(t('common.optional')) + ')</label><input type="text" id="dep-artifact" placeholder="worker.wasm"></div>' +
        '<div class="span-2"><label class="checkbox-row"><input type="checkbox" id="dep-spa" checked> ' + esc(t('projectDetail.spaFallback')) + '</label></div>' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.crons')) + '</label><textarea id="dep-crons" placeholder="' + esc(t('projectDetail.cronsPlaceholder')) + '"></textarea></div>' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.bindings')) + '</label><textarea id="dep-bindings" class="mono" placeholder="' + esc(t('projectDetail.bindingsPlaceholder')) + '"></textarea></div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="dep-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="new-dep-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        if (base) {
          $('dep-prefix').value = base.assets_prefix || '';
          $('dep-artifact').value = base.artifact_path || '';
          $('dep-spa').checked = base.spa_fallback;
          $('dep-crons').value = (base.crons || []).join('\n');
          $('dep-bindings').value = JSON.stringify(base.bindings || {}, null, 2);
        }
        $('dep-cancel').addEventListener('click', closeModal);
        $('new-dep-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var bindings = {};
          var rawBind = $('dep-bindings').value.trim();
          if (rawBind) {
            try {
              bindings = JSON.parse(rawBind);
              if (!bindings || typeof bindings !== 'object' || Array.isArray(bindings)) {
                throw new Error('bad');
              }
            } catch (_) {
              toast(t('projectDetail.invalidBindingsJson'), 'error');
              return;
            }
          }
          var crons = $('dep-crons').value.split('\n').map(function (s) { return s.trim(); }).filter(Boolean);
          var env = $('dep-env').value || null;
          var bucket = $('dep-bucket').value || null;
          var prefix = $('dep-prefix').value.trim() || null;
          var artifact = $('dep-artifact').value.trim() || null;
          var file = $('dep-file').files[0];
          if (file && (file.size === 0 || file.size > 64 * 1024 * 1024 || !/\.(wasm|wat)$/i.test(file.name))) {
            toast(t('projectDetail.invalidArtifactFile'), 'error');
            return;
          }
          var submit = document.querySelector('button[form="new-dep-form"]');
          submit.disabled = true;

          try {
            if (file) {
              var nonce = new Uint8Array(16);
              crypto.getRandomValues(nonce);
              var filename = 'upload-' + Array.from(nonce, function (byte) { return byte.toString(16).padStart(2, '0'); }).join('') + '.wasm';
              var uploaded = await api('/_api/v1/apps/' + enc(project.id) + '/artifacts/' + filename, {
                method: 'PUT', body: file, headers: { 'Content-Type': 'application/octet-stream' }
              });
              artifact = uploaded.artifact_path;
              $('dep-file').value = '';
              $('dep-artifact').value = artifact;
            }
            await api('/_api/v1/apps/' + enc(project.id) + '/deployments', {
              method: 'POST',
              json: {
                artifact_path: artifact,
                assets_bucket: bucket,
                assets_prefix: prefix,
                spa_fallback: $('dep-spa').checked,
                bindings: bindings,
                crons: crons,
                target_env: env
              }
            });
            closeModal();
            toast(t('projectDetail.deployCreated'), 'success');
            renderProjectDetail(project.id, true);
          } catch (e) { toast(e.message, 'error'); }
          finally { submit.disabled = false; }
        });
      }
    });
  }

  // --- 3. App Route Helpers & Modal ---

  function renderRoutePolicyBadges(r) {
    var badges = [];
    if (r.proxy_upstream) {
      var pfx = r.proxy_prefix || '/api';
      badges.push(
        '<span class="badge badge-info mono" title="' + esc(pfx + ' -> ' + r.proxy_upstream) + '">Proxy ' + esc(pfx) + '</span>'
      );
    }
    if (r.rate_limit_rpm) {
      badges.push(
        '<span class="badge badge-neutral mono">' + esc(r.rate_limit_rpm) + ' req/m</span>'
      );
    }
    if (r.basic_auth_enabled) {
      badges.push(
        '<span class="badge badge-warn"' +
        (r.basic_auth_user ? ' title="' + esc(r.basic_auth_user) + '"' : '') +
        '>Basic Auth</span>'
      );
    }
    if (r.canary_deployment_id && r.canary_weight > 0) {
      badges.push(
        '<span class="badge badge-accent mono" title="' + esc(r.canary_deployment_id) + '">Canary ' + esc(r.canary_weight) + '%</span>'
      );
    }
    return badges.length
      ? '<div class="route-policies">' + badges.join('') + '</div>'
      : '<span class="muted text-xs">-</span>';
  }

  function normalizeRouteHost(raw) {
    var trimmed = String(raw || '').trim();
    var withoutPort = trimmed;
    if (trimmed.charAt(0) === '[') {
      var closeIdx = trimmed.indexOf(']');
      if (closeIdx !== -1) {
        withoutPort = trimmed.slice(0, closeIdx + 1);
      }
    } else {
      var colonIdx = trimmed.lastIndexOf(':');
      if (colonIdx > 0) {
        var host = trimmed.slice(0, colonIdx);
        var port = trimmed.slice(colonIdx + 1);
        if (/^\d*$/.test(port)) {
          withoutPort = host;
        }
      }
    }
    return withoutPort.replace(/\.+$/, '').toLowerCase();
  }

  function openAppRouteModal(project, deployments, existingRoute, onDone) {
    var isEdit = Boolean(existingRoute);
    var depList = Array.isArray(deployments) ? deployments : [];
    var hasAdvPolicies = isEdit && Boolean(
      existingRoute.proxy_upstream ||
      existingRoute.rate_limit_rpm ||
      existingRoute.basic_auth_enabled ||
      existingRoute.canary_deployment_id
    );
    var canaryId = isEdit ? (existingRoute.canary_deployment_id || '') : '';
    var canaryOpts = '<option value="">' + esc(t('routes.canaryNone')) + '</option>' +
      depList.map(function (d) {
        return '<option value="' + esc(d.id) + '"' + (d.id === canaryId ? ' selected' : '') + '>' + esc(d.id + ' · ' + d.status) + '</option>';
      }).join('');
    if (canaryId && !depList.some(function (d) { return d.id === canaryId; })) {
      canaryOpts += '<option value="' + esc(canaryId) + '" selected>' + esc(canaryId) + '</option>';
    }
    var currentEnv = isEdit ? existingRoute.target_env : 'prod';
    var baseDomain = state.status && state.status.base_domain ? String(state.status.base_domain).trim() : '';
    function suggestHostForEnv(env) {
      if (!baseDomain) return '';
      var slug = String(project.name || project.id || 'app')
        .toLowerCase()
        .replace(/[^a-z0-9-]+/g, '-')
        .replace(/^-+|-+$/g, '') || 'app';
      return (env === 'preview' ? (slug + '-preview') : slug) + '.' + baseDomain;
    }

    openModal({
      title: isEdit ? (t('routes.edit') + ' · ' + existingRoute.host) : t('projectDetail.addRoute'),
      body:
        '<form id="route-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('routes.project')) + '</label>' +
        '<input type="text" disabled value="' + esc(project.name + ' (' + project.id + ')') + '"></div>' +
        '<div class="field"><label for="rt-host">' + esc(t('routes.host')) + '</label>' +
        '<input type="text" id="rt-host" required value="' + esc(isEdit ? existingRoute.host : '') + '"' +
        (isEdit ? ' readonly' : ' placeholder="' + esc(t('routes.hostPlaceholder')) + '"') + '>' +
        (!isEdit && baseDomain
          ? '<div class="row-between text-xs muted" style="margin-top:4px;">' +
            '<span>' + esc(t('routes.suggestedHost')) + ': <code id="rt-suggest-val">' + esc(suggestHostForEnv(currentEnv)) + '</code></span>' +
            '<button type="button" class="btn btn-sm" id="rt-use-suggest">' + esc(t('routes.useSuggested')) + '</button>' +
            '</div>'
          : '') +
        '</div>' +
        (!isEdit
          ? '<div id="rt-conflict-warn" class="alert alert-warn hidden">' +
            '<div id="rt-conflict-msg"></div>' +
            '<label class="checkbox-row" style="margin-top:8px;"><input type="checkbox" id="rt-conflict-confirm"> ' +
            '<span>' + esc(t('routes.conflictConfirm')) + '</span></label>' +
            '</div>'
          : '') +
        '<div class="field"><label for="rt-env">' + esc(t('routes.targetEnv')) + '</label>' +
        '<select id="rt-env"><option value="prod"' + (currentEnv === 'preview' ? '' : ' selected') + '>prod</option>' +
        '<option value="preview"' + (currentEnv === 'preview' ? ' selected' : '') + '>preview</option></select></div>' +
        '<details class="adv-options"' + (hasAdvPolicies ? ' open' : '') + '>' +
        '<summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="form-grid">' +
        '<div class="field"><label for="rt-proxy-prefix">' + esc(t('routes.proxyPrefix')) + '</label>' +
        '<input type="text" id="rt-proxy-prefix" value="' + esc(isEdit && existingRoute.proxy_prefix ? existingRoute.proxy_prefix : '') + '" placeholder="/api"></div>' +
        '<div class="field"><label for="rt-proxy-upstream">' + esc(t('routes.proxyUpstream')) + '</label>' +
        '<input type="text" id="rt-proxy-upstream" value="' + esc(isEdit && existingRoute.proxy_upstream ? existingRoute.proxy_upstream : '') + '" placeholder="http://127.0.0.1:8080"></div>' +
        '</div>' +
        '<div class="field"><label for="rt-rate-limit">' + esc(t('routes.rateLimitRpm')) + '</label>' +
        '<input type="number" id="rt-rate-limit" min="0" step="1" value="' + esc(isEdit && existingRoute.rate_limit_rpm ? existingRoute.rate_limit_rpm : '') + '" placeholder="' + esc(t('routes.rateLimitPlaceholder')) + '"></div>' +
        (isEdit && existingRoute.basic_auth_enabled
          ? '<div class="muted text-xs">' + esc(t('routes.basicAuthEnabledHint', { user: existingRoute.basic_auth_user || '-' })) + '</div>' +
            '<label class="checkbox-row"><input type="checkbox" id="rt-clear-auth"> ' + esc(t('routes.clearBasicAuth')) + '</label>'
          : '') +
        '<div class="form-grid">' +
        '<div class="field"><label for="rt-auth-user">' + esc(t('routes.basicAuthUser')) + '</label>' +
        '<input type="text" id="rt-auth-user" autocomplete="off" value="' + esc(isEdit && existingRoute.basic_auth_user ? existingRoute.basic_auth_user : '') + '"></div>' +
        '<div class="field"><label for="rt-auth-pass">' + esc(t('routes.basicAuthPassword')) + '</label>' +
        '<input type="password" id="rt-auth-pass" autocomplete="new-password" placeholder="' + esc(isEdit && existingRoute.basic_auth_enabled ? t('routes.basicAuthKeepHint') : '••••••••') + '"></div>' +
        '</div>' +
        '<div class="form-grid">' +
        '<div class="field"><label for="rt-canary-dep">' + esc(t('routes.canaryDeployment')) + '</label>' +
        '<select id="rt-canary-dep">' + canaryOpts + '</select></div>' +
        '<div class="field"><label for="rt-canary-weight">' + esc(t('routes.canaryWeight')) + '</label>' +
        '<input type="number" id="rt-canary-weight" min="0" max="100" step="1" value="' + esc(isEdit ? (existingRoute.canary_weight || 0) : 0) + '"></div>' +
        '</div>' +
        '</div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="rt-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="route-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        $('rt-cancel').addEventListener('click', closeModal);
        var lastConflictHost = '';
        function updateConflictWarning() {
          if (isEdit) return null;
          var norm = normalizeRouteHost($('rt-host').value);
          var conflict = norm
            ? state.routes.find(function (r) { return normalizeRouteHost(r.host) === norm; })
            : null;
          var warnEl = $('rt-conflict-warn');
          var msgEl = $('rt-conflict-msg');
          var chkEl = $('rt-conflict-confirm');
          if (!warnEl || !msgEl || !chkEl) return conflict;
          if (conflict) {
            msgEl.textContent = conflict.project_id === project.id
              ? t('routes.conflictSameApp', { host: conflict.host })
              : t('routes.conflictOtherApp', { host: conflict.host, app: projectNameById(conflict.project_id) });
            if (lastConflictHost !== conflict.host) {
              chkEl.checked = false;
              lastConflictHost = conflict.host;
            }
            warnEl.classList.remove('hidden');
          } else {
            lastConflictHost = '';
            chkEl.checked = false;
            warnEl.classList.add('hidden');
          }
          return conflict;
        }
        if (!isEdit && $('rt-host')) {
          $('rt-host').addEventListener('input', updateConflictWarning);
        }
        if (!isEdit && baseDomain && $('rt-use-suggest') && $('rt-suggest-val')) {
          $('rt-use-suggest').addEventListener('click', function () {
            $('rt-host').value = $('rt-suggest-val').textContent;
            updateConflictWarning();
          });
          if ($('rt-env')) {
            $('rt-env').addEventListener('change', function () {
              $('rt-suggest-val').textContent = suggestHostForEnv($('rt-env').value);
            });
          }
        }
        if ($('rt-clear-auth')) {
          $('rt-clear-auth').addEventListener('change', function () {
            var disabled = $('rt-clear-auth').checked;
            $('rt-auth-user').disabled = disabled;
            $('rt-auth-pass').disabled = disabled;
          });
        }
        $('route-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var host = isEdit ? existingRoute.host : $('rt-host').value.trim();
          if (!host) return;
          if (!isEdit) {
            var conflict = updateConflictWarning();
            if (conflict && !$('rt-conflict-confirm').checked) {
              toast(t('routes.conflictConfirmRequired'), 'error');
              return;
            }
          }
          var targetEnv = $('rt-env').value;
          var proxyPrefix = $('rt-proxy-prefix').value.trim();
          var proxyUpstream = $('rt-proxy-upstream').value.trim();
          if (proxyPrefix && !proxyUpstream) {
            toast(t('routes.proxyUpstreamRequired'), 'error');
            return;
          }
          var rateRaw = $('rt-rate-limit').value.trim();
          var rateLimitRpm = 0;
          if (rateRaw !== '') {
            var rateVal = Number(rateRaw);
            if (!Number.isInteger(rateVal) || rateVal < 0) {
              toast(t('routes.invalidRateLimit'), 'error');
              return;
            }
            rateLimitRpm = rateVal;
          }
          var clearAuth = Boolean($('rt-clear-auth') && $('rt-clear-auth').checked);
          var authUser = $('rt-auth-user').value.trim();
          var authPass = $('rt-auth-pass').value;
          if (!clearAuth) {
            var keepingExistingAuth = isEdit &&
              existingRoute.basic_auth_enabled &&
              authUser === (existingRoute.basic_auth_user || '');
            if ((authPass && !authUser) || (authUser && !authPass && !keepingExistingAuth)) {
              toast(t('routes.basicAuthPairRequired'), 'error');
              return;
            }
          }
          var canaryDepId = $('rt-canary-dep').value;
          var canaryWeightRaw = $('rt-canary-weight').value.trim();
          var canaryWeight = canaryWeightRaw === '' ? 0 : Number(canaryWeightRaw);
          if (!Number.isInteger(canaryWeight) || canaryWeight < 0 || canaryWeight > 100) {
            toast(t('routes.canaryWeightInvalid'), 'error');
            return;
          }
          if (!canaryDepId && canaryWeight > 0) {
            toast(t('routes.canaryDepRequired'), 'error');
            return;
          }

          var submit = document.querySelector('button[form="route-form"]');
          submit.disabled = true;
          try {
            if (isEdit) {
              await api('/_api/v1/routes/' + enc(existingRoute.host), {
                method: 'PATCH',
                json: {
                  target_env: targetEnv,
                  proxy_upstream: proxyUpstream,
                  proxy_prefix: proxyUpstream ? (proxyPrefix || '/api') : null,
                  rate_limit_rpm: rateLimitRpm,
                  clear_basic_auth: clearAuth ? true : null,
                  basic_auth_user: (!clearAuth && authUser && authPass) ? authUser : null,
                  basic_auth_password: (!clearAuth && authUser && authPass) ? authPass : null,
                  canary_deployment_id: canaryDepId,
                  canary_weight: canaryDepId ? canaryWeight : 0
                }
              });
            } else {
              await api('/_api/v1/routes', {
                method: 'POST',
                json: {
                  host: host,
                  project: project.id,
                  target_env: targetEnv,
                  proxy_upstream: proxyUpstream || null,
                  proxy_prefix: proxyUpstream ? (proxyPrefix || '/api') : null,
                  rate_limit_rpm: rateLimitRpm > 0 ? rateLimitRpm : null,
                  basic_auth_user: (authUser && authPass) ? authUser : null,
                  basic_auth_password: (authUser && authPass) ? authPass : null,
                  canary_deployment_id: canaryDepId || null,
                  canary_weight: canaryDepId ? canaryWeight : 0
                }
              });
            }
            closeModal();
            toast(t('routes.saved'), 'success');
            onDone();
          } catch (e) { toast(e.message, 'error'); }
          finally { submit.disabled = false; }
        });
      }
    });
  }

  // --- 4. KV Store ---

  async function renderKv(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('kv');
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var res = await Promise.all([api('/_api/v1/kv'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.kvNamespaces = res[0] || [];
        state.projects = res[1] || [];
        if (!state.selectedKvNs && state.kvNamespaces.length) {
          state.selectedKvNs = state.kvNamespaces[0].name;
        }
        if (state.selectedKvNs) {
          await loadKvKeys();
          if (!isCurrent()) return;
        }
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;

    var nsMeta = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; });
    var canWriteNs = !isReadonly() && (isAdmin() || (nsMeta && canWriteResource(nsMeta.project_id)));

    var nsMenu = state.kvNamespaces.length
      ? groupedMenu(state.kvNamespaces, function (n) {
          return '<button type="button" class="list-menu-item' + (n.name === state.selectedKvNs ? ' active' : '') + '" data-select-ns="' + esc(n.name) + '">' +
            '<span class="truncate">' + esc(n.name) + '</span>' +
            (n.project_id ? '<span class="badge badge-neutral text-xs">' + esc(projectNameById(n.project_id)) + '</span>' : '') +
            '</button>';
        })
      : '<div class="empty-state" style="padding:20px 10px;">' + esc(t('kv.noNamespaces')) + '</div>';

    var keyRows = state.kvKeys.map(function (k) {
      return '<tr>' +
        '<td data-label="' + esc(t('kv.key')) + '" class="mono text-strong break-all">' + esc(k.key) + '</td>' +
        '<td data-label="' + esc(t('kv.size')) + '">' + esc(formatBytes(k.size)) + '</td>' +
        '<td data-label="' + esc(t('common.expiresAt')) + '" class="text-xs muted">' + esc(k.expires_at ? formatTime(k.expires_at) : t('common.never')) + '</td>' +
        '<td data-label="' + esc(t('common.updatedAt')) + '" class="text-xs muted">' + esc(formatTime(k.updated_at)) + '</td>' +
        '<td class="cell-actions-td"><div class="cell-actions">' +
        '<button type="button" class="btn btn-sm" data-view-kv="' + esc(k.key) + '">' + esc(t('common.view')) + '</button>' +
        (canWriteNs ? '<button type="button" class="btn btn-sm" data-incr-kv="' + esc(k.key) + '">' + esc(t('kv.incr')) + '</button>' : '') +
        (canWriteNs ? '<button type="button" class="btn btn-danger btn-sm" data-del-kv="' + esc(k.key) + '">' + esc(t('common.delete')) + '</button>' : '') +
        '</div></td></tr>';
    }).join('');

    var rightHtml = !state.selectedKvNs
      ? '<div class="panel"><div class="empty-state">' + esc(t('kv.selectNamespace')) + '</div></div>'
      : '<div class="panel">' +
        '<div class="panel-header"><div class="toolbar">' +
        '<div><h3 class="panel-title mono">' + esc(state.selectedKvNs) + '</h3>' +
        (nsMeta ? '<div class="muted text-xs">TTL: ' + esc(nsMeta.default_ttl_secs || 604800) + 's' +
          (nsMeta.max_keys ? ' · Max Keys: ' + esc(nsMeta.max_keys) : '') +
          (nsMeta.max_value_bytes ? ' · Max Value: ' + esc(formatBytes(nsMeta.max_value_bytes)) : '') +
          (nsMeta.description ? ' · ' + esc(nsMeta.description) : '') + '</div>' : '') +
        '</div>' +
        '<div class="row">' +
        '<input type="search" id="kv-prefix-input" style="width:190px;" value="' + esc(state.kvPrefix) + '" placeholder="' + esc(t('kv.prefixPlaceholder')) + '">' +
        '<button type="button" class="btn btn-sm" id="kv-filter-btn">' + esc(t('common.filter')) + '</button>' +
        (canWriteNs ? '<button type="button" class="btn btn-primary btn-sm" id="kv-put-btn">' + esc(t('kv.putKey')) + '</button>' : '') +
        (canWriteNs && nsMeta ? '<button type="button" class="btn btn-sm" id="kv-edit-ns-btn">' + esc(t('common.editConfig')) + '</button>' : '') +
        (canWriteNs && nsMeta ? '<button type="button" class="btn btn-danger btn-sm" id="kv-del-ns-btn">' + esc(t('kv.deleteNamespace')) + '</button>' : '') +
        '</div></div></div>' +
        (keyRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('kv.key')) + '</th><th>' + esc(t('kv.size')) + '</th><th>' + esc(t('common.expiresAt')) + '</th><th>' + esc(t('common.updatedAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + keyRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('kv.noKeys')) + '</div>') +
        '</div>';

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('kv.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('kv.subtitle')) + '</p></div>' +
      (isAdmin() ? '<button type="button" class="btn btn-primary" id="new-kv-ns-btn">' + esc(t('kv.newNamespace')) + '</button>' : '') +
      '</div>' +
      '<div class="split-layout">' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('kv.namespaces')) + '</h3></div>' +
      '<div class="panel-body" style="padding:10px 10px 4px;"><form id="kv-manual-form" class="input-group">' +
      '<input type="text" id="kv-manual-ns" placeholder="' + esc(t('kv.manualNamespace')) + '">' +
      '<button type="submit" class="btn btn-sm">' + esc(t('kv.openNamespace')) + '</button></form></div>' +
      '<div class="list-menu">' + nsMenu + '</div></div>' +
      rightHtml + '</div>';

    if ($('new-kv-ns-btn')) $('new-kv-ns-btn').addEventListener('click', openCreateKvNamespaceModal);
    $('kv-manual-form').addEventListener('submit', async function (ev) {
      ev.preventDefault();
      var ns = $('kv-manual-ns').value.trim();
      if (!ns) return;
      state.selectedKvNs = ns;
      try {
        await loadKvKeys();
        renderKv(false);
      } catch (e) { toast(e.message, 'error'); }
    });
    root.querySelectorAll('[data-select-ns]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.selectedKvNs = btn.getAttribute('data-select-ns');
        try {
          await loadKvKeys();
          renderKv(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    });
    if ($('kv-filter-btn')) {
      $('kv-filter-btn').addEventListener('click', async function () {
        state.kvPrefix = $('kv-prefix-input').value.trim();
        try {
          await loadKvKeys();
          renderKv(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('kv-put-btn')) {
      $('kv-put-btn').addEventListener('click', function () {
        openKvPutModal(null);
      });
    }
    if ($('kv-edit-ns-btn') && nsMeta) {
      $('kv-edit-ns-btn').addEventListener('click', function () {
        openEditKvNamespaceModal(nsMeta);
      });
    }
    if ($('kv-del-ns-btn') && nsMeta) {
      $('kv-del-ns-btn').addEventListener('click', function () {
        var nsName = nsMeta.name;
        confirmAction(t('kv.deleteNamespaceConfirm', { name: nsName }), async function () {
          try {
            await api('/_api/v1/kv/' + enc(nsName), { method: 'DELETE' });
            state.selectedKvNs = '';
            state.kvKeys = [];
            toast(t('kv.namespaceDeleted'), 'success');
            renderKv(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    root.querySelectorAll('[data-view-kv]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        var key = btn.getAttribute('data-view-kv');
        try {
          var item = await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key));
          openKvValueModal(key, item, canWriteNs);
        } catch (e) { toast(e.message, 'error'); }
      });
    });
    root.querySelectorAll('[data-incr-kv]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        openKvIncrModal(btn.getAttribute('data-incr-kv'));
      });
    });
    root.querySelectorAll('[data-del-kv]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var key = btn.getAttribute('data-del-kv');
        confirmAction(t('kv.deleteConfirm', { key: key }), async function () {
          try {
            await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key), { method: 'DELETE' });
            toast(t('kv.deleted'), 'success');
            await loadKvKeys();
            renderKv(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });
  }

  async function loadKvKeys() {
    var qs = '?limit=200' + (state.kvPrefix ? '&prefix=' + enc(state.kvPrefix) : '');
    state.kvKeys = (await api('/_api/v1/kv/' + enc(state.selectedKvNs) + qs)) || [];
  }

  function openCreateKvNamespaceModal() {
    openModal({
      title: t('kv.newNamespace'),
      body:
        '<form id="kv-ns-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('kv.namespaceName')) + '</label><input type="text" id="kns-name" required placeholder="' + esc(t('kv.namespacePlaceholder')) + '"></div>' +
        resourceGroupField() +
        '<div class="field"><label>' + esc(t('common.project')) + '</label><select id="kns-proj">' + renderProjectSelectOptions('', true) + '</select></div>' +
        '<div class="field"><label>' + esc(t('kv.defaultTtlSecs')) + '</label>' +
        '<input type="number" id="kns-ttl" min="1" required value="604800">' +
        '<div class="preset-chips">' +
        '<button type="button" class="preset-chip-btn" data-kv-ttl="3600">' + esc(t('kv.ttlPreset1h')) + ' (3600s)</button>' +
        '<button type="button" class="preset-chip-btn" data-kv-ttl="86400">' + esc(t('kv.ttlPreset1d')) + ' (86400s)</button>' +
        '<button type="button" class="preset-chip-btn" data-kv-ttl="604800">' + esc(t('kv.ttlPreset7d')) + ' (604800s)</button>' +
        '<button type="button" class="preset-chip-btn" data-kv-ttl="2592000">' + esc(t('kv.ttlPreset30d')) + ' (2592000s)</button>' +
        '</div>' +
        '<div class="muted text-xs">' + esc(t('kv.defaultTtlHint')) + '</div></div>' +
        '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="kns-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('kv.maxValueBytes')) + '</label><input type="number" id="kns-max-val" min="1" placeholder="1048576"></div>' +
        '<div class="field"><label>' + esc(t('kv.maxKeys')) + '</label><input type="number" id="kns-max-keys" min="1" placeholder="100000"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="kns-eph"> ' + esc(t('kv.defaultEphemeral')) + '</label>' +
        '</div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="kns-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="kv-ns-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('kns-cancel').addEventListener('click', closeModal);
        document.querySelectorAll('[data-kv-ttl]').forEach(function (chip) {
          chip.addEventListener('click', function () {
            $('kns-ttl').value = chip.getAttribute('data-kv-ttl');
          });
        });
        $('kv-ns-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var name = $('kns-name').value.trim();
          var pid = $('kns-proj').value || null;
          var ttl = Number($('kns-ttl').value || 604800);
          var desc = $('kns-desc').value.trim();
          var maxValRaw = $('kns-max-val').value.trim();
          var maxKeysRaw = $('kns-max-keys').value.trim();
          try {
            await api('/_api/v1/kv', {
              method: 'POST',
              json: {
                name: name,
                group: selectedResourceGroup(),
                project_id: pid,
                description: desc || null,
                default_ttl_secs: ttl,
                default_ephemeral: $('kns-eph').checked,
                max_value_bytes: maxValRaw ? Number(maxValRaw) : null,
                max_keys: maxKeysRaw ? Number(maxKeysRaw) : null
              }
            });
            closeModal();
            state.selectedKvNs = name;
            toast(t('kv.namespaceCreated'), 'success');
            renderKv(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openEditKvNamespaceModal(nsMeta) {
    openModal({
      title: t('common.editConfig') + ' · ' + nsMeta.name,
      body:
        '<form id="edit-kv-ns-form" class="stack gap-md">' +
        resourceGroupField(nsMeta.group || '') +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="ekns-desc" value="' + esc(nsMeta.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('kv.defaultTtlSecs')) + '</label>' +
        '<input type="number" id="ekns-ttl" min="1" required value="' + esc(nsMeta.default_ttl_secs || 604800) + '">' +
        '<div class="preset-chips">' +
        '<button type="button" class="preset-chip-btn" data-ekv-ttl="3600">' + esc(t('kv.ttlPreset1h')) + '</button>' +
        '<button type="button" class="preset-chip-btn" data-ekv-ttl="86400">' + esc(t('kv.ttlPreset1d')) + '</button>' +
        '<button type="button" class="preset-chip-btn" data-ekv-ttl="604800">' + esc(t('kv.ttlPreset7d')) + '</button>' +
        '<button type="button" class="preset-chip-btn" data-ekv-ttl="2592000">' + esc(t('kv.ttlPreset30d')) + '</button>' +
        '</div></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('kv.maxValueBytes')) + '</label><input type="number" id="ekns-max-val" min="0" value="' + esc(nsMeta.max_value_bytes || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '<div class="field"><label>' + esc(t('kv.maxKeys')) + '</label><input type="number" id="ekns-max-keys" min="0" value="' + esc(nsMeta.max_keys || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="ekns-eph"' + (nsMeta.default_ephemeral ? ' checked' : '') + '> ' + esc(t('kv.defaultEphemeral')) + '</label>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="ekns-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="edit-kv-ns-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('ekns-cancel').addEventListener('click', closeModal);
        document.querySelectorAll('[data-ekv-ttl]').forEach(function (chip) {
          chip.addEventListener('click', function () {
            $('ekns-ttl').value = chip.getAttribute('data-ekv-ttl');
          });
        });
        $('edit-kv-ns-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var maxValRaw = $('ekns-max-val').value.trim();
          var maxKeysRaw = $('ekns-max-keys').value.trim();
          try {
            await api('/_api/v1/kv/' + enc(nsMeta.name), {
              method: 'PATCH',
              json: {
                group: $('resource-group').value.trim(),
                description: $('ekns-desc').value.trim(),
                default_ttl_secs: Number($('ekns-ttl').value || 604800),
                default_ephemeral: $('ekns-eph').checked,
                max_value_bytes: maxValRaw ? Number(maxValRaw) : 0,
                max_keys: maxKeysRaw ? Number(maxKeysRaw) : 0
              }
            });
            closeModal();
            toast(t('kv.namespaceUpdated'), 'success');
            renderKv(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openKvValueModal(key, item, canEdit) {
    openModal({
      title: t('kv.valueModalTitle') + ': ' + key,
      body:
        '<div class="stack gap-sm">' +
        '<div class="muted text-xs">' + esc(t('common.updatedAt')) + ': ' + esc(formatTime(item.updated_at)) +
        ' · ' + esc(t('common.expiresAt')) + ': ' + esc(item.expires_at ? formatTime(item.expires_at) : t('common.never')) + '</div>' +
        '<div class="field"><label>' + esc(t('kv.value')) + '</label><pre class="code-block">' + esc(item.value) + '</pre></div>' +
        (item.metadata ? '<div class="field"><label>' + esc(t('kv.metadata')) + '</label><pre class="code-block">' + esc(item.metadata) + '</pre></div>' : '') +
        '</div>',
      footer:
        '<button type="button" class="btn" id="kv-val-close">' + esc(t('common.close')) + '</button>' +
        (canEdit ? '<button type="button" class="btn btn-primary" id="kv-val-edit">' + esc(t('common.edit')) + '</button>' : ''),
      onMount: function () {
        $('kv-val-close').addEventListener('click', closeModal);
        if ($('kv-val-edit')) {
          $('kv-val-edit').addEventListener('click', function () {
            openKvPutModal({ key: key, value: item.value, metadata: item.metadata || '' });
          });
        }
      }
    });
  }

  function openKvPutModal(preset) {
    var nsMeta = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; });
    var defaultTtl = (nsMeta && nsMeta.default_ttl_secs) ? nsMeta.default_ttl_secs : 604800;
    var defaultEph = Boolean(nsMeta && nsMeta.default_ephemeral);
    openModal({
      title: t('kv.putKey'),
      body:
        '<form id="kv-put-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('kv.key')) + '</label><input type="text" id="kvp-key" required value="' + esc(preset ? preset.key : '') + '" placeholder="' + esc(t('kv.keyPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('kv.value')) + '</label><textarea id="kvp-val" placeholder="' + esc(t('kv.valuePlaceholder')) + '">' + esc(preset ? preset.value : '') + '</textarea></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('kv.ttlSecs')) + '</label><input type="number" id="kvp-ttl" min="1" required value="' + esc(defaultTtl) + '" placeholder="' + esc(t('kv.ttlPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('kv.metadata')) + '</label><input type="text" id="kvp-meta" value="' + esc(preset ? preset.metadata : '') + '" placeholder="' + esc(t('kv.metadataPlaceholder')) + '"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="kvp-eph"' + (defaultEph ? ' checked' : '') + '> ' + esc(t('kv.ephemeral')) + '</label>' +
        '<div class="muted text-xs" style="margin-top:-8px;">' + esc(t('kv.ephemeralHint')) + '</div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="kvp-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="kv-put-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        $('kvp-cancel').addEventListener('click', closeModal);
        $('kv-put-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var key = $('kvp-key').value.trim();
          var ttlRaw = $('kvp-ttl').value.trim();
          var metaRaw = $('kvp-meta').value.trim();
          try {
            await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key), {
              method: 'PUT',
              json: {
                value: $('kvp-val').value,
                metadata: metaRaw || null,
                ttl_secs: ttlRaw ? Number(ttlRaw) : defaultTtl,
                ephemeral: $('kvp-eph').checked
              }
            });
            closeModal();
            toast(t('kv.saved'), 'success');
            await loadKvKeys();
            renderKv(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openKvIncrModal(key) {
    var nsMeta = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; });
    var defaultTtl = (nsMeta && nsMeta.default_ttl_secs) ? nsMeta.default_ttl_secs : 604800;
    openModal({
      title: t('kv.incr') + ': ' + key,
      body:
        '<form id="kv-incr-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('kv.incrDelta')) + '</label><input type="number" id="kvi-delta" value="1" required></div>' +
        '<div class="field"><label>' + esc(t('kv.ttlSecs')) + '</label><input type="number" id="kvi-ttl" min="1" required value="' + esc(defaultTtl) + '" placeholder="' + esc(t('kv.ttlPlaceholder')) + '"></div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="kvi-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="kv-incr-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        $('kvi-cancel').addEventListener('click', closeModal);
        $('kv-incr-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var ttlRaw = $('kvi-ttl').value.trim();
          try {
            var res = await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key) + '/incr', {
              method: 'POST',
              json: {
                delta: Number($('kvi-delta').value || 1),
                ttl_secs: ttlRaw ? Number(ttlRaw) : defaultTtl
              }
            });
            closeModal();
            toast(t('kv.incrDone', { val: res.value }), 'success');
            await loadKvKeys();
            renderKv(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  // --- 5. Queues ---

  async function renderQueues(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('queues');
    if (state.currentRoute !== 'queues') return;
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var res = await Promise.all([api('/_api/v1/queues'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.queues = res[0] || [];
        state.projects = res[1] || [];
        if (!state.selectedQueue && state.queues.length) {
          state.selectedQueue = state.queues[0].name;
        }
        if (state.selectedQueue) {
          await loadSelectedQueue();
          if (!isCurrent()) return;
        }
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;

    var qCfg = state.queues.find(function (q) { return q.name === state.selectedQueue; });
    var canWriteQ = qCfg && canWriteResource(qCfg.project_id);

    var qMenu = state.queues.length
      ? groupedMenu(state.queues, function (q) {
          return '<button type="button" class="list-menu-item' + (q.name === state.selectedQueue ? ' active' : '') + '" data-select-queue="' + esc(q.name) + '">' +
            '<span class="truncate">' + esc(q.name) + '</span>' +
            '<span class="row gap-xs" style="flex-wrap:nowrap;">' +
            (q.ephemeral ? '<span class="badge badge-info text-xs">' + esc(t('queues.ephemeralBadge')) + '</span>' : '') +
            '<span class="badge badge-neutral text-xs">c:' + esc(q.max_concurrency) + '</span></span></button>';
        })
      : '<div class="empty-state" style="padding:20px 10px;">' + esc(t('queues.empty')) + '</div>';

    var detailHtml = '';
    if (!qCfg) {
      detailHtml = '<div class="panel"><div class="empty-state">' + esc(t('queues.selectQueue')) + '</div></div>';
    } else {
      var st = state.queueStats || { pending: 0, running: 0, dead: 0 };
      var jobs = state.queueJobs || [];
      var jobRows = jobs.map(function (j) {
        var decoded = decodePayloadBytes(j.payload);
        return '<tr>' +
          '<td data-label="' + esc(t('queues.jobId')) + '" class="mono">#' + esc(j.id) + '</td>' +
          '<td data-label="' + esc(t('queues.payload')) + '"><pre class="code-block" style="max-height:90px;">' + esc(decoded) + '</pre></td>' +
          '<td data-label="' + esc(t('queues.attempts')) + '">' + esc(j.attempts + ' / ' + j.max_attempts) + '</td>' +
          '<td data-label="' + esc(t('queues.lastError')) + '" class="text-xs muted break-all">' + esc(j.last_error || '-') + '</td>' +
          '<td data-label="' + esc(t('queues.runAt')) + '" class="text-xs muted">' + esc(formatTime(j.run_at)) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          (canWriteQ && state.queueInspectStatus === 'dead'
            ? '<button type="button" class="btn btn-sm" data-replay-job="' + esc(j.id) + '">' + esc(t('queues.replaySelected')) + '</button>'
            : '') +
          '</div></td></tr>';
      }).join('');

      detailHtml =
        '<div class="stack gap-md">' +
        '<div class="stats-grid">' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsPending')) + '</span><span class="stat-value">' + esc(st.pending) + '</span></div>' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsRunning')) + '</span><span class="stat-value">' + esc(st.running) + '</span></div>' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsDead')) + '</span><span class="stat-value" style="color:' + (st.dead > 0 ? 'var(--danger)' : 'inherit') + ';">' + esc(st.dead) + '</span></div>' +
        '</div>' +
        '<div class="panel"><div class="panel-header"><div class="toolbar">' +
        '<div class="row">' +
        '<h3 class="panel-title mono">' + esc(qCfg.name) + '</h3>' +
        (qCfg.ephemeral ? '<span class="badge badge-info">' + esc(t('queues.ephemeralBadge')) + '</span>' : '') +
        (qCfg.max_pending_jobs ? '<span class="badge badge-neutral text-xs">max:' + esc(qCfg.max_pending_jobs) + '</span>' : '') +
        '<select id="queue-inspect-status" style="width:auto;min-width:150px;">' +
        '<option value="dead"' + (state.queueInspectStatus === 'dead' ? ' selected' : '') + '>' + esc(t('queues.statusDead')) + '</option>' +
        '<option value="pending"' + (state.queueInspectStatus === 'pending' ? ' selected' : '') + '>' + esc(t('queues.statusPending')) + '</option>' +
        '<option value="running"' + (state.queueInspectStatus === 'running' ? ' selected' : '') + '>' + esc(t('queues.statusRunning')) + '</option>' +
        '</select>' +
        '</div>' +
        (canWriteQ
          ? '<div class="row">' +
            '<button type="button" class="btn btn-primary btn-sm" id="q-enqueue-btn">' + esc(t('queues.enqueue')) + '</button>' +
            '<button type="button" class="btn btn-sm" id="q-edit-btn">' + esc(t('queues.editConfig')) + '</button>' +
            (st.dead > 0 ? '<button type="button" class="btn btn-sm" id="q-replay-all-btn">' + esc(t('queues.replayAll')) + '</button>' : '') +
            '<button type="button" class="btn btn-danger btn-sm" id="q-purge-btn">' + esc(t('queues.purge')) + '</button>' +
            '<button type="button" class="btn btn-danger btn-sm" id="q-delete-btn">' + esc(t('queues.deleteQueue')) + '</button>' +
            '</div>'
          : '') +
        '</div></div>' +
        (jobRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('queues.jobId')) + '</th><th>' + esc(t('queues.payload')) + '</th><th>' + esc(t('queues.attempts')) + '</th><th>' + esc(t('queues.lastError')) + '</th><th>' + esc(t('queues.runAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + jobRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('queues.noJobs')) + '</div>') +
        '</div></div>';
    }

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('queues.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('queues.subtitle')) + '</p></div>' +
      (isAdmin() ? '<button type="button" class="btn btn-primary" id="new-queue-btn">' + esc(t('queues.new')) + '</button>' : '') +
      '</div>' +
      '<div class="split-layout">' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('nav.queues')) + '</h3></div><div class="list-menu">' + qMenu + '</div></div>' +
      detailHtml + '</div>';

    if ($('new-queue-btn')) $('new-queue-btn').addEventListener('click', openCreateQueueModal);
    root.querySelectorAll('[data-select-queue]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.selectedQueue = btn.getAttribute('data-select-queue');
        try {
          await loadSelectedQueue();
          renderQueues(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    });
    if ($('queue-inspect-status')) {
      $('queue-inspect-status').addEventListener('change', async function (e) {
        state.queueInspectStatus = e.target.value;
        try {
          await loadSelectedQueue();
          renderQueues(false);
        } catch (err) { toast(err.message, 'error'); }
      });
    }
    if ($('q-enqueue-btn')) $('q-enqueue-btn').addEventListener('click', function () { openEnqueueModal(qCfg.name); });
    if ($('q-edit-btn')) $('q-edit-btn').addEventListener('click', function () { openEditQueueModal(qCfg); });
    if ($('q-replay-all-btn')) {
      $('q-replay-all-btn').addEventListener('click', async function () {
        try {
          var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/dlq/replay', { method: 'POST', json: {} });
          toast(t('queues.replayed', { count: res.replayed }), 'success');
          await loadSelectedQueue();
          renderQueues(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('q-purge-btn')) {
      $('q-purge-btn').addEventListener('click', function () {
        confirmAction(t('queues.purgeConfirm', { name: qCfg.name }), async function () {
          try {
            var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/purge', { method: 'POST' });
            toast(t('queues.purged', { count: res.purged }), 'success');
            await loadSelectedQueue();
            renderQueues(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    if ($('q-delete-btn')) {
      $('q-delete-btn').addEventListener('click', function () {
        var qName = qCfg.name;
        confirmAction(t('queues.deleteConfirm', { name: qName }), async function () {
          try {
            await api('/_api/v1/queues/' + enc(qName), { method: 'DELETE' });
            state.selectedQueue = '';
            state.queueStats = null;
            state.queueJobs = [];
            toast(t('queues.deleted'), 'success');
            renderQueues(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    root.querySelectorAll('[data-replay-job]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        var jobId = Number(btn.getAttribute('data-replay-job'));
        try {
          var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/dlq/replay', {
            method: 'POST',
            json: { job_ids: [jobId] }
          });
          toast(t('queues.replayed', { count: res.replayed }), 'success');
          await loadSelectedQueue();
          renderQueues(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    });
  }

  function openEditQueueModal(qCfg) {
    openModal({
      title: t('queues.editConfig') + ' · ' + qCfg.name,
      body:
        '<form id="edit-queue-form" class="stack gap-md">' +
        resourceGroupField(qCfg.group || '') +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="eq-desc" value="' + esc(qCfg.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.consumerProject')) + '</label><select id="eq-cproj">' + renderProjectSelectOptions(qCfg.consumer_project_id || '', true) + '</select></div>' +
        '<div class="grid-2">' +
        '<div class="field"><label>' + esc(t('queues.maxConcurrency')) + '</label><input type="number" id="eq-conc" min="1" value="' + esc(qCfg.max_concurrency) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.rateLimit')) + '</label><input type="number" id="eq-rate" min="0" value="' + esc(qCfg.rate_limit_per_sec || '') + '" placeholder="unlimited"></div>' +
        '</div>' +
        '<div class="grid-2">' +
        '<div class="field"><label>' + esc(t('queues.batchSize')) + '</label><input type="number" id="eq-batch" min="1" value="' + esc(qCfg.batch_size) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.leaseSecs')) + '</label><input type="number" id="eq-lease" min="1" value="' + esc(qCfg.lease_secs || 60) + '"></div>' +
        '</div>' +
        '<div class="grid-2">' +
        '<div class="field"><label>' + esc(t('queues.maxAttempts')) + '</label><input type="number" id="eq-att" min="1" value="' + esc(qCfg.max_attempts) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.retryBackoffSecs')) + '</label><input type="number" id="eq-backoff" min="1" value="' + esc(qCfg.retry_backoff_secs || 5) + '"></div>' +
        '</div>' +
        '<div class="grid-2">' +
        '<div class="field"><label>' + esc(t('queues.dlqRetentionSecs')) + '</label><input type="number" id="eq-dlq" min="0" value="' + esc(qCfg.dlq_retention_secs || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.maxPendingJobs')) + '</label><input type="number" id="eq-max-pend" min="0" value="' + esc(qCfg.max_pending_jobs || '') + '" placeholder="10000"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="eq-eph"' + (qCfg.ephemeral ? ' checked' : '') + '> ' + esc(t('queues.ephemeral')) + '</label>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="eq-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="edit-queue-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('eq-cancel').addEventListener('click', closeModal);
        $('edit-queue-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var rateRaw = $('eq-rate').value.trim();
          var dlqRaw = $('eq-dlq').value.trim();
          var maxPendRaw = $('eq-max-pend').value.trim();
          var cproj = $('eq-cproj').value;
          var body = {
            group: $('resource-group').value.trim(),
            description: $('eq-desc').value.trim(),
            max_concurrency: Number($('eq-conc').value) || 4,
            batch_size: Number($('eq-batch').value) || 20,
            lease_secs: Number($('eq-lease').value) || 60,
            max_attempts: Number($('eq-att').value) || 5,
            retry_backoff_secs: Number($('eq-backoff').value) || 5,
            ephemeral: $('eq-eph').checked
          };
          if (!cproj) {
            body.clear_consumer_project = true;
          } else {
            body.consumer_project_id = cproj;
          }
          if (rateRaw === '' || Number(rateRaw) <= 0) {
            body.clear_rate_limit = true;
          } else {
            body.rate_limit_per_sec = Number(rateRaw);
          }
          if (dlqRaw === '' || Number(dlqRaw) <= 0) {
            body.clear_dlq_retention = true;
          } else {
            body.dlq_retention_secs = Number(dlqRaw);
          }
          if (maxPendRaw === '' || Number(maxPendRaw) <= 0) {
            body.clear_max_pending_jobs = true;
          } else {
            body.max_pending_jobs = Number(maxPendRaw);
          }
          try {
            await api('/_api/v1/queues/' + enc(qCfg.name), { method: 'PATCH', json: body });
            closeModal();
            toast(t('queues.updated'), 'success');
            renderQueues(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  async function loadSelectedQueue() {
    var name = state.selectedQueue;
    var status = state.queueInspectStatus;
    var res = await Promise.all([
      api('/_api/v1/queues/' + enc(name) + '/stats'),
      api('/_api/v1/queues/' + enc(name) + '/inspect?status=' + enc(status) + '&limit=50')
    ]);
    if (state.selectedQueue !== name || state.queueInspectStatus !== status) return;
    state.queueStats = res[0];
    state.queueJobs = res[1] || [];
  }

  function openCreateQueueModal() {
    openModal({
      title: t('queues.new'),
      body:
        '<form id="create-q-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('queues.name')) + '</label><input type="text" id="cq-name" required placeholder="' + esc(t('queues.namePlaceholder')) + '"></div>' +
        resourceGroupField() +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('queues.producerProject')) + '</label><select id="cq-proj">' + renderProjectSelectOptions('', true) + '</select></div>' +
        '<div class="field"><label>' + esc(t('queues.consumerProject')) + '</label><select id="cq-cproj">' + renderProjectSelectOptions('', true) + '</select></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="cq-eph"> ' + esc(t('queues.ephemeral')) + '</label>' +
        '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="cq-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('queues.maxConcurrency')) + '</label><input type="number" id="cq-conc" value="4" min="1"></div>' +
        '<div class="field"><label>' + esc(t('queues.rateLimit')) + '</label><input type="number" id="cq-rate" min="1" placeholder="unlimited"></div>' +
        '<div class="field"><label>' + esc(t('queues.batchSize')) + '</label><input type="number" id="cq-batch" value="20" min="1"></div>' +
        '<div class="field"><label>' + esc(t('queues.leaseSecs')) + '</label><input type="number" id="cq-lease" value="60" min="1"></div>' +
        '<div class="field"><label>' + esc(t('queues.maxAttempts')) + '</label><input type="number" id="cq-attempts" value="5" min="1"></div>' +
        '<div class="field"><label>' + esc(t('queues.retryBackoffSecs')) + '</label><input type="number" id="cq-backoff" value="5" min="1"></div>' +
        '<div class="field"><label>' + esc(t('queues.dlqRetentionSecs')) + '</label><input type="number" id="cq-dlq" min="1" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '<div class="field"><label>' + esc(t('queues.maxPendingJobs')) + '</label><input type="number" id="cq-max-pend" min="1" placeholder="10000"></div>' +
        '</div></div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="cq-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="create-q-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('cq-cancel').addEventListener('click', closeModal);
        $('create-q-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var name = $('cq-name').value.trim();
          var rateRaw = $('cq-rate').value.trim();
          var dlqRaw = $('cq-dlq').value.trim();
          var maxPendRaw = $('cq-max-pend').value.trim();
          var descRaw = $('cq-desc').value.trim();
          try {
            await api('/_api/v1/queues', {
              method: 'POST',
              json: {
                name: name,
                group: selectedResourceGroup(),
                description: descRaw || null,
                project_id: $('cq-proj').value || null,
                consumer_project_id: $('cq-cproj').value || null,
                max_concurrency: Number($('cq-conc').value || 4),
                rate_limit_per_sec: rateRaw ? Number(rateRaw) : null,
                batch_size: Number($('cq-batch').value || 20),
                lease_secs: Number($('cq-lease').value || 60),
                max_attempts: Number($('cq-attempts').value || 5),
                retry_backoff_secs: Number($('cq-backoff').value || 5),
                dlq_retention_secs: dlqRaw ? Number(dlqRaw) : null,
                ephemeral: $('cq-eph').checked,
                max_pending_jobs: maxPendRaw ? Number(maxPendRaw) : null
              }
            });
            closeModal();
            state.selectedQueue = name;
            toast(t('queues.created'), 'success');
            renderQueues(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openEnqueueModal(queueName) {
    openModal({
      title: t('queues.enqueue') + ' (' + queueName + ')',
      body:
        '<form id="enqueue-form" class="form-grid">' +
        '<div class="field span-2"><label>' + esc(t('queues.payload')) + '</label><textarea id="enq-payload" class="mono" placeholder=\'' + esc(t('queues.payloadPlaceholder')) + '\'>{"hello":"ferropod"}</textarea></div>' +
        '<div class="field"><label>' + esc(t('queues.delaySecs')) + '</label><input type="number" id="enq-delay" min="0" placeholder="0"></div>' +
        '<div class="field"><label>' + esc(t('queues.priority')) + '</label><input type="number" id="enq-prio" placeholder="0"></div>' +
        '<div class="field"><label>' + esc(t('queues.dedupKey')) + '</label><input type="text" id="enq-dedup"></div>' +
        '<div class="field"><label>' + esc(t('queues.maxAttempts')) + '</label><input type="number" id="enq-attempts" min="1"></div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="enq-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="enqueue-form" class="btn btn-primary">' + esc(t('queues.enqueue')) + '</button>',
      onMount: function () {
        $('enq-cancel').addEventListener('click', closeModal);
        $('enqueue-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var payload;
          try {
            payload = JSON.parse($('enq-payload').value);
          } catch (_) {
            toast(t('queues.invalidPayloadJson'), 'error');
            return;
          }
          var delay = $('enq-delay').value.trim();
          var prio = $('enq-prio').value.trim();
          var dedup = $('enq-dedup').value.trim();
          var attempts = $('enq-attempts').value.trim();
          try {
            var res = await api('/_api/v1/queues/' + enc(queueName) + '/enqueue', {
              method: 'POST',
              json: {
                payload: payload,
                delay_secs: delay ? Number(delay) : null,
                priority: prio ? Number(prio) : null,
                dedup_key: dedup || null,
                max_attempts: attempts ? Number(attempts) : null
              }
            });
            closeModal();
            toast(t('queues.enqueued', { id: res.job_id }), 'success');
            await loadSelectedQueue();
            renderQueues(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  // --- 6. Blobs / Object Storage ---

  async function renderBlobs(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('blobs');
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var res = await Promise.all([api('/_api/v1/buckets'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.buckets = res[0] || [];
        state.projects = res[1] || [];
        if (!state.selectedBucket && state.buckets.length) {
          state.selectedBucket = state.buckets[0].name;
        }
        if (state.selectedBucket) {
          await loadBucketObjects();
          if (!isCurrent()) return;
        }
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;

    var bCfg = state.buckets.find(function (b) { return b.name === state.selectedBucket; });
    var canWriteBkt = bCfg && canWriteResource(bCfg.project_id);

    var bMenu = state.buckets.length
      ? groupedMenu(state.buckets, function (b) {
          var badge = b.public_read
            ? '<span class="badge badge-success">' + esc(t('blobs.public')) + '</span>'
            : '<span class="badge badge-neutral">' + esc(t('blobs.private')) + '</span>';
          return '<button type="button" class="list-menu-item' + (b.name === state.selectedBucket ? ' active' : '') + '" data-select-bucket="' + esc(b.name) + '">' +
            '<span class="truncate">' + esc(b.name) + '</span>' + badge + '</button>';
        })
      : '<div class="empty-state" style="padding:20px 10px;">' + esc(t('blobs.noBuckets')) + '</div>';

    var rightHtml = '';
    if (!bCfg) {
      rightHtml = '<div class="panel"><div class="empty-state">' + esc(t('blobs.selectBucket')) + '</div></div>';
    } else {
      var objRows = state.blobObjects.map(function (o) {
        return '<tr>' +
          '<td data-label="' + esc(t('blobs.objectKey')) + '" class="mono text-strong break-all">' + esc(o.key) + '</td>' +
          '<td data-label="' + esc(t('kv.size')) + '">' + esc(formatBytes(o.size)) + '</td>' +
          '<td data-label="' + esc(t('blobs.contentType')) + '" class="text-xs muted">' + esc(o.content_type) + '</td>' +
          '<td data-label="' + esc(t('common.updatedAt')) + '" class="text-xs muted">' + esc(formatTime(o.updated_at)) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          '<button type="button" class="btn btn-sm" data-dl-obj="' + esc(o.key) + '">' + esc(t('common.download')) + '</button>' +
          '<button type="button" class="btn btn-sm" data-presign-obj="' + esc(o.key) + '">' + esc(t('blobs.presign')) + '</button>' +
          (canWriteBkt ? '<button type="button" class="btn btn-danger btn-sm" data-del-obj="' + esc(o.key) + '">' + esc(t('common.delete')) + '</button>' : '') +
          '</div></td></tr>';
      }).join('');

      rightHtml =
        '<div class="stack gap-md">' +
        '<div class="panel"><div class="panel-header"><div class="toolbar">' +
        '<div class="row"><h3 class="panel-title mono">' + esc(bCfg.name) + '</h3>' +
        (bCfg.public_read
          ? '<span class="badge badge-success">' + esc(t('blobs.public')) + '</span>'
          : '<span class="badge badge-neutral">' + esc(t('blobs.private')) + '</span>') +
        '</div>' +
        '<div class="row">' +
        '<input type="search" id="blob-prefix-input" style="width:180px;" value="' + esc(state.blobPrefix) + '" placeholder="' + esc(t('blobs.prefixPlaceholder')) + '">' +
        '<button type="button" class="btn btn-sm" id="blob-filter-btn">' + esc(t('common.filter')) + '</button>' +
        (canWriteBkt ? '<button type="button" class="btn btn-sm" id="blob-edit-bkt-btn">' + esc(t('common.editConfig')) + '</button>' : '') +
        (isAdmin() ? '<button type="button" class="btn btn-sm" id="blob-toggle-pub">' + esc(t('blobs.togglePublic')) + '</button>' : '') +
        (canWriteBkt ? '<button type="button" class="btn btn-danger btn-sm" id="blob-del-bkt-btn">' + esc(t('blobs.deleteBucket')) + '</button>' : '') +
        '</div></div></div>' +
        (canWriteBkt
          ? '<div class="panel-body stack gap-sm">' +
            '<div class="field" style="max-width:320px;"><label>' + esc(t('blobs.uploadPrefixLabel')) + '</label>' +
            '<input type="text" id="blob-upload-prefix" placeholder="' + esc(t('blobs.uploadPrefixPlaceholder')) + '"></div>' +
            '<div id="blob-dropzone" class="dropzone">' +
            '<input type="file" id="blob-file-input" multiple class="hidden">' +
            '<div id="blob-dropzone-label">' + esc(t('blobs.uploadDropHint')) + '</div></div></div>'
          : '') +
        (objRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('blobs.objectKey')) + '</th><th>' + esc(t('kv.size')) + '</th><th>' + esc(t('blobs.contentType')) + '</th><th>' + esc(t('common.updatedAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + objRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('blobs.noObjects')) + '</div>') +
        '</div></div>';
    }

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('blobs.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('blobs.subtitle')) + '</p></div>' +
      (isAdmin() ? '<button type="button" class="btn btn-primary" id="new-bucket-btn">' + esc(t('blobs.newBucket')) + '</button>' : '') +
      '</div>' +
      '<div class="split-layout">' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('blobs.buckets')) + '</h3></div><div class="list-menu">' + bMenu + '</div></div>' +
      rightHtml + '</div>';

    if ($('new-bucket-btn')) $('new-bucket-btn').addEventListener('click', openCreateBucketModal);
    root.querySelectorAll('[data-select-bucket]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.selectedBucket = btn.getAttribute('data-select-bucket');
        try {
          await loadBucketObjects();
          renderBlobs(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    });

    if ($('blob-filter-btn')) {
      $('blob-filter-btn').addEventListener('click', async function () {
        state.blobPrefix = $('blob-prefix-input').value.trim();
        try {
          await loadBucketObjects();
          renderBlobs(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('blob-toggle-pub')) {
      $('blob-toggle-pub').addEventListener('click', async function () {
        try {
          await api('/_api/v1/buckets/' + enc(bCfg.name), {
            method: 'PATCH',
            json: { public_read: !bCfg.public_read }
          });
          toast(t('blobs.bucketUpdated'), 'success');
          renderBlobs(true);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('blob-edit-bkt-btn') && bCfg) {
      $('blob-edit-bkt-btn').addEventListener('click', function () {
        openEditBucketModal(bCfg);
      });
    }
    if ($('blob-del-bkt-btn') && bCfg) {
      $('blob-del-bkt-btn').addEventListener('click', function () {
        var bName = bCfg.name;
        confirmAction(t('blobs.deleteBucketConfirm', { name: bName }), async function () {
          try {
            await api('/_api/v1/buckets/' + enc(bName), { method: 'DELETE' });
            state.selectedBucket = '';
            state.blobObjects = [];
            toast(t('blobs.bucketDeleted'), 'success');
            renderBlobs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    if ($('blob-dropzone')) {
      var dz = $('blob-dropzone');
      var fi = $('blob-file-input');
      dz.addEventListener('click', function () { fi.click(); });
      fi.addEventListener('change', function () {
        if (fi.files && fi.files.length) uploadBlobFiles(bCfg.name, fi.files);
      });
      dz.addEventListener('dragover', function (e) { e.preventDefault(); dz.classList.add('dragover'); });
      dz.addEventListener('dragleave', function () { dz.classList.remove('dragover'); });
      dz.addEventListener('drop', function (e) {
        e.preventDefault();
        dz.classList.remove('dragover');
        if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length) {
          uploadBlobFiles(bCfg.name, e.dataTransfer.files);
        }
      });
    }

    root.querySelectorAll('[data-dl-obj]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        downloadBlobObject(bCfg.name, btn.getAttribute('data-dl-obj'));
      });
    });
    root.querySelectorAll('[data-presign-obj]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        openPresignModal(bCfg.name, btn.getAttribute('data-presign-obj'), canWriteBkt);
      });
    });
    root.querySelectorAll('[data-del-obj]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var key = btn.getAttribute('data-del-obj');
        confirmAction(t('blobs.deleteConfirm', { key: key }), async function () {
          try {
            await api('/_blob/' + enc(bCfg.name) + '/' + encPath(key), { method: 'DELETE' });
            toast(t('blobs.deleted'), 'success');
            await loadBucketObjects();
            renderBlobs(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });
  }

  async function loadBucketObjects() {
    var qs = '?limit=200' + (state.blobPrefix ? '&prefix=' + enc(state.blobPrefix) : '');
    state.blobObjects = (await api('/_blob/' + enc(state.selectedBucket) + qs)) || [];
  }

  async function uploadBlobFiles(bucket, fileList) {
    var prefix = ($('blob-upload-prefix') ? $('blob-upload-prefix').value : '').trim().replace(/^\/+|\/+$/g, '');
    var labelEl = $('blob-dropzone-label');
    var total = fileList.length;
    var done = 0;
    try {
      for (var i = 0; i < total; i++) {
        var file = fileList[i];
        var key = prefix ? prefix + '/' + file.name : file.name;
        if (labelEl) labelEl.textContent = t('blobs.uploading', { done: done, total: total });
        await api('/_blob/' + enc(bucket) + '/' + encPath(key), {
          method: 'PUT',
          headers: { 'Content-Type': file.type || 'application/octet-stream' },
          body: file
        });
        done += 1;
      }
      toast(t('blobs.uploaded', { count: done }), 'success');
      await loadBucketObjects();
      renderBlobs(false);
    } catch (e) {
      toast(e.message, 'error');
      if (labelEl) labelEl.textContent = t('blobs.uploadDropHint');
    }
  }

  async function downloadBlobObject(bucket, key) {
    try {
      var res = await fetch('/_blob/' + enc(bucket) + '/' + encPath(key), {
        headers: state.token ? { Authorization: 'Bearer ' + state.token } : {}
      });
      if (!res.ok) throw new Error(res.status + ' ' + res.statusText);
      var blob = await res.blob();
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = key.split('/').pop() || 'download';
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      setTimeout(function () { URL.revokeObjectURL(url); }, 1000);
    } catch (e) {
      toast(e.message, 'error');
    }
  }

  function openCreateBucketModal() {
    openModal({
      title: t('blobs.newBucket'),
      body:
        '<form id="create-bkt-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('blobs.bucketName')) + '</label><input type="text" id="cb-name" required placeholder="' + esc(t('blobs.bucketPlaceholder')) + '"></div>' +
        resourceGroupField() +
        '<div class="field"><label>' + esc(t('common.project')) + '</label><select id="cb-proj">' + renderProjectSelectOptions('', true) + '</select></div>' +
        '<label class="checkbox-row"><input type="checkbox" id="cb-pub"> ' + esc(t('blobs.publicRead')) + '</label>' +
        '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="cb-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('blobs.maxObjectSizeMb')) + '</label><input type="number" id="cb-max-obj" min="1" placeholder="64"></div>' +
        '<div class="field"><label>' + esc(t('blobs.quotaMb')) + '</label><input type="number" id="cb-quota" min="1" placeholder="1024"></div>' +
        '</div>' +
        '<div class="field"><label>' + esc(t('blobs.allowedContentTypes')) + '</label><input type="text" id="cb-types" placeholder="image/*,application/pdf,text/*"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('blobs.cacheControl')) + '</label><input type="text" id="cb-cache" placeholder="public, max-age=86400"></div>' +
        '<div class="field"><label>' + esc(t('blobs.corsOrigins')) + '</label><input type="text" id="cb-cors" placeholder="*"></div>' +
        '</div>' +
        '</div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="cb-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="create-bkt-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('cb-cancel').addEventListener('click', closeModal);
        $('create-bkt-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var name = $('cb-name').value.trim();
          var maxObjRaw = $('cb-max-obj').value.trim();
          var quotaRaw = $('cb-quota').value.trim();
          try {
            await api('/_api/v1/buckets', {
              method: 'POST',
              json: {
                name: name,
                group: selectedResourceGroup(),
                project_id: $('cb-proj').value || null,
                public_read: $('cb-pub').checked,
                description: $('cb-desc').value.trim() || null,
                max_object_size_mb: maxObjRaw ? Number(maxObjRaw) : null,
                quota_mb: quotaRaw ? Number(quotaRaw) : null,
                allowed_content_types: $('cb-types').value.trim() || null,
                cache_control: $('cb-cache').value.trim() || null,
                cors_origins: $('cb-cors').value.trim() || null
              }
            });
            closeModal();
            state.selectedBucket = name;
            toast(t('blobs.bucketCreated'), 'success');
            renderBlobs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openEditBucketModal(bCfg) {
    openModal({
      title: t('common.editConfig') + ' · ' + bCfg.name,
      body:
        '<form id="edit-bkt-form" class="stack gap-md">' +
        resourceGroupField(bCfg.group || '') +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="eb-desc" value="' + esc(bCfg.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        (isAdmin() ? '<label class="checkbox-row"><input type="checkbox" id="eb-pub"' + (bCfg.public_read ? ' checked' : '') + '> ' + esc(t('blobs.publicRead')) + '</label>' : '') +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('blobs.maxObjectSizeMb')) + '</label><input type="number" id="eb-max-obj" min="0" value="' + esc(bCfg.max_object_size_mb || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '<div class="field"><label>' + esc(t('blobs.quotaMb')) + '</label><input type="number" id="eb-quota" min="0" value="' + esc(bCfg.quota_mb || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '</div>' +
        '<div class="field"><label>' + esc(t('blobs.allowedContentTypes')) + '</label><input type="text" id="eb-types" value="' + esc(bCfg.allowed_content_types || '') + '" placeholder="image/*,application/pdf,text/*"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('blobs.cacheControl')) + '</label><input type="text" id="eb-cache" value="' + esc(bCfg.cache_control || '') + '" placeholder="public, max-age=86400"></div>' +
        '<div class="field"><label>' + esc(t('blobs.corsOrigins')) + '</label><input type="text" id="eb-cors" value="' + esc(bCfg.cors_origins || '') + '" placeholder="*"></div>' +
        '</div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="eb-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="edit-bkt-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('eb-cancel').addEventListener('click', closeModal);
        $('edit-bkt-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var maxObjRaw = $('eb-max-obj').value.trim();
          var quotaRaw = $('eb-quota').value.trim();
          var body = {
            group: $('resource-group').value.trim(),
            description: $('eb-desc').value.trim(),
            max_object_size_mb: maxObjRaw ? Number(maxObjRaw) : 0,
            quota_mb: quotaRaw ? Number(quotaRaw) : 0,
            allowed_content_types: $('eb-types').value.trim(),
            cache_control: $('eb-cache').value.trim(),
            cors_origins: $('eb-cors').value.trim()
          };
          if ($('eb-pub')) body.public_read = $('eb-pub').checked;
          try {
            await api('/_api/v1/buckets/' + enc(bCfg.name), { method: 'PATCH', json: body });
            closeModal();
            toast(t('blobs.bucketUpdated'), 'success');
            renderBlobs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openPresignModal(bucket, key, allowPut) {
    openModal({
      title: t('blobs.presign') + ': ' + key,
      body:
        '<form id="presign-form" class="form-grid">' +
        '<div class="field"><label>' + esc(t('blobs.presignOp')) + '</label><select id="ps-op">' +
        '<option value="get">' + esc(t('blobs.presignGet')) + '</option>' +
        (allowPut ? '<option value="put">' + esc(t('blobs.presignPut')) + '</option>' : '') +
        '</select></div>' +
        '<div class="field"><label>' + esc(t('blobs.presignExpires')) + '</label><input type="number" id="ps-ttl" value="300" min="1"></div>' +
        '</form>' +
        '<div id="ps-result-box" class="hidden stack gap-xs">' +
        '<div class="text-xs muted">' + esc(t('blobs.presignResult')) + '</div>' +
        '<pre id="ps-url-pre" class="code-block"></pre>' +
        '<div class="row"><button type="button" class="btn btn-sm" id="ps-copy-btn">' + esc(t('common.copy')) + '</button></div>' +
        '</div>',
      footer:
        '<button type="button" class="btn" id="ps-cancel">' + esc(t('common.close')) + '</button>' +
        '<button type="submit" form="presign-form" class="btn btn-primary">' + esc(t('blobs.presignGenerate')) + '</button>',
      onMount: function () {
        $('ps-cancel').addEventListener('click', closeModal);
        $('presign-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          try {
            var res = await api('/_api/v1/buckets/' + enc(bucket) + '/presign', {
              method: 'POST',
              json: {
                key: key,
                op: $('ps-op').value,
                expires_secs: Number($('ps-ttl').value || 300)
              }
            });
            var fullUrl = window.location.origin + res.url;
            $('ps-url-pre').textContent = fullUrl;
            $('ps-result-box').classList.remove('hidden');
            $('ps-copy-btn').onclick = function () { copyText(fullUrl); };
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  // --- 7. Databases (Turso User DBs) ---

  async function renderDbs(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('dbs');
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var res = await Promise.all([api('/_api/v1/dbs'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.dbs = res[0] || [];
        state.projects = res[1] || [];
        if (!state.selectedDbId && state.dbs.length) {
          state.selectedDbId = state.dbs[0].id;
        }
        if (state.selectedDbId && state.dbTab === 'schema') {
          await loadDbSchema();
          if (!isCurrent()) return;
        }
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;

    var dbMeta = state.dbs.find(function (d) { return d.id === state.selectedDbId; });
    var canWriteDb = dbMeta && canWriteResource(dbMeta.project_id);

    var dbMenu = state.dbs.length
      ? groupedMenu(state.dbs, function (d) {
          return '<button type="button" class="list-menu-item' + (d.id === state.selectedDbId ? ' active' : '') + '" data-select-db="' + esc(d.id) + '">' +
            '<span class="truncate">' + esc(d.name) + '</span>' +
            (d.project_id ? '<span class="badge badge-neutral text-xs">' + esc(projectNameById(d.project_id)) + '</span>' : '') +
            '</button>';
        })
      : '<div class="empty-state" style="padding:20px 10px;">' + esc(t('dbs.noDbs')) + '</div>';

    var rightHtml = '';
    if (!dbMeta) {
      rightHtml = '<div class="panel"><div class="empty-state">' + esc(t('dbs.selectDb')) + '</div></div>';
    } else {
      var tabsHtml =
        '<div class="tabs">' +
        '<button type="button" class="tab-btn' + (state.dbTab === 'console' ? ' active' : '') + '" data-db-tab="console">' + esc(t('dbs.tabConsole')) + '</button>' +
        '<button type="button" class="tab-btn' + (state.dbTab === 'schema' ? ' active' : '') + '" data-db-tab="schema">' + esc(t('dbs.tabSchema')) + '</button>' +
        '<button type="button" class="tab-btn' + (state.dbTab === 'fts' ? ' active' : '') + '" data-db-tab="fts">' + esc(t('dbs.tabFts')) + '</button>' +
        '</div>';

      var bodyHtml = '';
      if (state.dbTab === 'fts') {
        bodyHtml = '<div id="db-fts-panel" class="panel"></div>';
      } else if (state.dbTab === 'schema') {
        bodyHtml =
          '<div class="panel"><div class="panel-header"><h3 class="panel-title mono">' + esc(dbMeta.name) + '</h3>' +
          '<button type="button" class="btn btn-sm" id="db-reload-schema">' + esc(t('dbs.reloadSchema')) + '</button></div>' +
          '<div class="panel-body">' +
          (state.dbSchema
            ? '<pre class="code-block">' + esc(state.dbSchema) + '</pre>'
            : '<div class="empty-state">' + esc(t('dbs.emptySchema')) + '</div>') +
          '</div></div>';
      } else {
        var resultHtml = '';
        if (state.dbResult) {
          if (state.dbResult.type === 'execute') {
            resultHtml = '<div class="alert alert-info">' + esc(t('dbs.rowsAffected', { count: state.dbResult.rows_affected })) + '</div>';
          } else {
            var cols = state.dbResult.columns || [];
            var rows = state.dbResult.rows || [];
            var thead = '<tr>' + cols.map(function (c) { return '<th>' + esc(c) + '</th>'; }).join('') + '</tr>';
            var tbody = rows.map(function (r) {
              return '<tr>' + r.map(function (cell) {
                var txt = cell === null ? 'NULL' : (typeof cell === 'object' ? JSON.stringify(cell) : String(cell));
                return '<td class="mono text-xs">' + esc(txt) + '</td>';
              }).join('') + '</tr>';
            }).join('');
            resultHtml =
              '<div class="muted text-xs">' + esc(t('dbs.rowsReturned', { count: rows.length })) + '</div>' +
              (rows.length
                ? '<div class="table-wrap"><table class="data-table"><thead>' + thead + '</thead><tbody>' + tbody + '</tbody></table></div>'
                : '<div class="empty-state">' + esc(t('dbs.noRows')) + '</div>');
          }
        }
        bodyHtml =
          '<div class="panel"><div class="panel-header"><div class="toolbar">' +
          '<div><h3 class="panel-title mono">' + esc(dbMeta.name) + '</h3><div class="muted text-xs mono">' + esc(dbMeta.id) +
          (dbMeta.read_only ? ' · Read-Only' : '') +
          (dbMeta.max_rows ? ' · Max Rows: ' + esc(dbMeta.max_rows) : '') +
          (dbMeta.max_size_mb ? ' · Max Size: ' + esc(dbMeta.max_size_mb) + ' MiB' : '') +
          (dbMeta.description ? ' · ' + esc(dbMeta.description) : '') +
          '</div></div>' +
          '<div class="row">' +
          '<button type="button" class="btn btn-sm" id="db-sample-btn">' + esc(t('dbs.sampleSelect')) + '</button>' +
          (canWriteDb ? '<button type="button" class="btn btn-sm" id="db-edit-btn">' + esc(t('common.editConfig')) + '</button>' : '') +
          (canWriteDb ? '<button type="button" class="btn btn-danger btn-sm" id="db-delete-btn">' + esc(t('dbs.deleteDb')) + '</button>' : '') +
          '</div>' +
          '</div></div><div class="panel-body stack gap-md">' +
          '<div class="field"><label>' + esc(t('dbs.sqlLabel')) + '</label><textarea id="db-sql-input" class="mono" placeholder="' + esc(t('dbs.sqlPlaceholder')) + '">' + esc(state.dbSql) + '</textarea></div>' +
          '<div class="field"><label>' + esc(t('dbs.paramsLabel')) + '</label><input type="text" id="db-params-input" class="mono" value="' + esc(state.dbParams) + '" placeholder="' + esc(t('dbs.paramsPlaceholder')) + '"></div>' +
          '<div class="row-between">' +
          '<div class="muted text-xs">' + esc(t('dbs.shortcutHint')) + '</div>' +
          '<div class="row">' +
          '<button type="button" class="btn btn-primary" id="db-query-btn">' + esc(t('dbs.runQuery')) + '</button>' +
          (canWriteDb ? '<button type="button" class="btn" id="db-exec-btn">' + esc(t('dbs.runExecute')) + '</button>' : '') +
          '</div></div>' +
          resultHtml +
          '</div></div>';
      }
      rightHtml = '<div class="stack gap-md">' + tabsHtml + bodyHtml + '</div>';
    }

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('dbs.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('dbs.subtitle')) + '</p></div>' +
      (isAdmin() ? '<button type="button" class="btn btn-primary" id="new-db-btn">' + esc(t('dbs.newDb')) + '</button>' : '') +
      '</div>' +
      '<div class="split-layout">' +
      '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('dbs.list')) + '</h3></div><div class="list-menu">' + dbMenu + '</div></div>' +
      rightHtml + '</div>';

    if ($('new-db-btn')) $('new-db-btn').addEventListener('click', openCreateDbModal);
    root.querySelectorAll('[data-select-db]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.selectedDbId = btn.getAttribute('data-select-db');
        state.dbResult = null;
        if (state.dbTab === 'schema') {
          try { await loadDbSchema(); } catch (e) { toast(e.message, 'error'); }
        }
        renderDbs(false);
      });
    });
    root.querySelectorAll('[data-db-tab]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.dbTab = btn.getAttribute('data-db-tab');
        if (state.dbTab === 'schema' && dbMeta) {
          try { await loadDbSchema(); } catch (e) { toast(e.message, 'error'); }
        }
        renderDbs(false);
      });
    });
    if ($('db-reload-schema')) {
      $('db-reload-schema').addEventListener('click', async function () {
        try {
          await loadDbSchema();
          renderDbs(false);
        } catch (e) { toast(e.message, 'error'); }
      });
    }
    if ($('db-sample-btn')) {
      $('db-sample-btn').addEventListener('click', function () {
        state.dbSql = t('dbs.sampleTables');
        state.dbParams = '[]';
        if ($('db-sql-input')) $('db-sql-input').value = state.dbSql;
        if ($('db-params-input')) $('db-params-input').value = state.dbParams;
      });
    }
    if ($('db-sql-input')) {
      $('db-sql-input').addEventListener('input', function (e) { state.dbSql = e.target.value; });
      $('db-sql-input').addEventListener('keydown', function (e) {
        if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') {
          e.preventDefault();
          runDbQuery(dbMeta.id);
        }
      });
    }
    if ($('db-params-input')) {
      $('db-params-input').addEventListener('input', function (e) { state.dbParams = e.target.value; });
    }
    if ($('db-query-btn')) $('db-query-btn').addEventListener('click', function () { runDbQuery(dbMeta.id); });
    if ($('db-exec-btn')) $('db-exec-btn').addEventListener('click', function () { runDbExecute(dbMeta.id); });
    if ($('db-edit-btn') && dbMeta) $('db-edit-btn').addEventListener('click', function () { openEditDbModal(dbMeta); });
    if ($('db-delete-btn') && dbMeta) {
      $('db-delete-btn').addEventListener('click', function () {
        var delId = dbMeta.id;
        var delName = dbMeta.name;
        confirmAction(t('dbs.deleteConfirm', { name: delName }), async function () {
          try {
            await api('/_api/v1/dbs/' + enc(delId), { method: 'DELETE' });
            state.selectedDbId = '';
            state.dbResult = null;
            state.dbSchema = '';
            toast(t('dbs.deleted'), 'success');
            renderDbs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    }
    if ($('db-fts-panel') && dbMeta) {
      renderDbFts(dbMeta, canWriteDb);
    }
  }

  async function renderDbFts(db, canWrite) {
    var panel = $('db-fts-panel');
    var reqToken = state.token;
    var ftsSeq = ++dbFtsRequestSeq;
    function isCurrentFts() {
      return (
        panel &&
        panel.isConnected &&
        state.token === reqToken &&
        state.currentRoute === 'dbs' &&
        state.selectedDbId === db.id &&
        dbFtsRequestSeq === ftsSeq
      );
    }
    var path = '/_api/v1/dbs/' + enc(db.id) + '/fts';
    try {
      var indexes = await api(path);
      if (!isCurrentFts()) return;
      panel.innerHTML = '<div class="panel-header"><h3 class="panel-title">' + esc(t('dbs.tabFts')) + '</h3></div>' +
        '<div class="panel-body stack gap-md"><p class="muted text-xs">' + esc(t('dbs.ftsSubtitle')) + '</p>' +
        (canWrite ? '<form id="fts-create-form" class="stack gap-sm">' +
        '<input id="fts-name" required placeholder="' + esc(t('dbs.ftsIndexName')) + '">' +
        '<input id="fts-table" required placeholder="' + esc(t('dbs.ftsTable')) + '">' +
        '<input id="fts-columns" required placeholder="' + esc(t('dbs.ftsColumns')) + '">' +
        '<select id="fts-tokenizer">' +
        '<option value="unicode61">unicode61 (Default)</option>' +
        '<option value="cjk_jieba">cjk_jieba (CJK / Chinese)</option>' +
        '<option value="ngram3">ngram3 (Substring)</option>' +
        '<option value="raw">raw (Exact)</option>' +
        '</select>' +
        '<button type="submit" class="btn btn-primary">' + esc(t('dbs.ftsNewIndex')) + '</button></form>' : '') +
        (indexes.length ? indexes.map(function (idx) {
          return '<div class="panel"><div class="panel-body stack gap-sm"><strong>' + esc(idx.name) + '</strong>' +
            '<span class="muted">' + esc(idx.table + ' (' + idx.columns.join(', ') + ')' + (idx.tokenizer ? ' · ' + idx.tokenizer : '')) + '</span>' +
            '<form class="row" data-fts-query="' + esc(idx.name) + '"><input name="query" required placeholder="' +
            esc(t('dbs.ftsTestPlaceholder')) + '"><button class="btn" type="submit">' + esc(t('common.search')) + '</button></form>' +
            '<pre class="code-block hidden" data-fts-result="' + esc(idx.name) + '"></pre>' +
            (canWrite ? '<button class="btn btn-danger btn-sm" data-fts-delete="' + esc(idx.name) + '">' + esc(t('common.delete')) + '</button>' : '') +
            '</div></div>';
        }).join('') : '<div class="empty-state">' + esc(t('dbs.ftsNoIndexes')) + '</div>') + '</div>';
      if ($('fts-create-form')) $('fts-create-form').addEventListener('submit', async function (event) {
        event.preventDefault();
        try {
          await api(path, { method: 'POST', json: {
            name: $('fts-name').value.trim(), table: $('fts-table').value.trim(),
            columns: $('fts-columns').value.split(',').map(function (value) { return value.trim(); }).filter(Boolean),
            tokenizer: $('fts-tokenizer') ? $('fts-tokenizer').value : null
          } });
          if (isCurrentFts()) renderDbFts(db, canWrite);
        } catch (e) { toast(e.message, 'error'); }
      });
      panel.querySelectorAll('[data-fts-query]').forEach(function (form) {
        form.addEventListener('submit', async function (event) {
          event.preventDefault();
          try {
            var result = await api(path + '/' + enc(form.getAttribute('data-fts-query')) + '/query', {
              method: 'POST', json: { query: form.elements.query.value }
            });
            if (!isCurrentFts() || !form.isConnected) return;
            var output = form.nextElementSibling;
            output.textContent = JSON.stringify(result.rows, null, 2);
            output.classList.remove('hidden');
          } catch (e) { toast(e.message, 'error'); }
        });
      });
      panel.querySelectorAll('[data-fts-delete]').forEach(function (button) {
        button.addEventListener('click', function () {
          confirmAction(t('common.confirmDeleteTitle'), async function () {
            try {
              await api(path + '/' + enc(button.getAttribute('data-fts-delete')), { method: 'DELETE' });
              if (isCurrentFts()) renderDbFts(db, canWrite);
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      });
    } catch (e) {
      if (isCurrentFts()) panel.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
    }
  }

  async function loadDbSchema() {
    var res = await api('/_api/v1/dbs/' + enc(state.selectedDbId) + '/schema');
    state.dbSchema = res ? res.schema : '';
  }

  function parseDbParams() {
    var raw = ($('db-params-input') ? $('db-params-input').value : state.dbParams).trim();
    if (!raw) return [];
    var parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) throw new Error(t('dbs.invalidParamsJson'));
    return parsed;
  }

  async function runDbQuery(dbId) {
    var sql = ($('db-sql-input') ? $('db-sql-input').value : state.dbSql).trim();
    var params;
    try {
      params = parseDbParams();
    } catch (_) {
      toast(t('dbs.invalidParamsJson'), 'error');
      return;
    }
    try {
      var res = await api('/_api/v1/dbs/' + enc(dbId) + '/query', {
        method: 'POST',
        json: { sql: sql, params: params }
      });
      state.dbResult = { type: 'query', columns: res.columns, rows: res.rows };
      renderDbs(false);
    } catch (e) { toast(e.message, 'error'); }
  }

  async function runDbExecute(dbId) {
    var sql = ($('db-sql-input') ? $('db-sql-input').value : state.dbSql).trim();
    var params;
    try {
      params = parseDbParams();
    } catch (_) {
      toast(t('dbs.invalidParamsJson'), 'error');
      return;
    }
    try {
      var res = await api('/_api/v1/dbs/' + enc(dbId) + '/execute', {
        method: 'POST',
        json: { statements: [{ sql: sql, params: params }] }
      });
      state.dbResult = { type: 'execute', rows_affected: res.rows_affected };
      renderDbs(false);
    } catch (e) { toast(e.message, 'error'); }
  }

  function openCreateDbModal() {
    openModal({
      title: t('dbs.newDb'),
      body:
        '<form id="create-db-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('dbs.dbName')) + '</label><input type="text" id="cdb-name" required placeholder="' + esc(t('dbs.dbPlaceholder')) + '"></div>' +
        resourceGroupField() +
        '<div class="field"><label>' + esc(t('common.project')) + '</label><select id="cdb-proj">' + renderProjectSelectOptions('', true) + '</select></div>' +
        '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body stack gap-sm">' +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="cdb-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('dbs.initSql')) + '</label><textarea id="cdb-init-sql" class="mono" rows="3" placeholder="' + esc(t('dbs.initSqlPlaceholder')) + '"></textarea></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('dbs.maxRows')) + '</label><input type="number" id="cdb-max-rows" min="1" placeholder="1000000"></div>' +
        '<div class="field"><label>' + esc(t('dbs.maxSizeMb')) + '</label><input type="number" id="cdb-max-size" min="1" placeholder="512"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="cdb-fk" checked> ' + esc(t('dbs.foreignKeys')) + '</label>' +
        '<label class="checkbox-row"><input type="checkbox" id="cdb-ro"> ' + esc(t('dbs.readOnly')) + '</label>' +
        '</div></details>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="cdb-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="create-db-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('cdb-cancel').addEventListener('click', closeModal);
        $('create-db-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var maxRowsRaw = $('cdb-max-rows').value.trim();
          var maxSizeRaw = $('cdb-max-size').value.trim();
          var initSqlRaw = $('cdb-init-sql').value.trim();
          try {
            var res = await api('/_api/v1/dbs', {
              method: 'POST',
              json: {
                name: $('cdb-name').value.trim(),
                group: selectedResourceGroup(),
                project_id: $('cdb-proj').value || null,
                description: $('cdb-desc').value.trim() || null,
                init_sql: initSqlRaw || null,
                max_rows: maxRowsRaw ? Number(maxRowsRaw) : null,
                max_size_mb: maxSizeRaw ? Number(maxSizeRaw) : null,
                foreign_keys: $('cdb-fk').checked,
                read_only: $('cdb-ro').checked
              }
            });
            closeModal();
            state.selectedDbId = res.id;
            toast(t('dbs.created'), 'success');
            renderDbs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openEditDbModal(dbMeta) {
    openModal({
      title: t('common.editConfig') + ' · ' + dbMeta.name,
      body:
        '<form id="edit-db-form" class="stack gap-md">' +
        resourceGroupField(dbMeta.group || '') +
        '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="edb-desc" value="' + esc(dbMeta.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('dbs.maxRows')) + '</label><input type="number" id="edb-max-rows" min="0" value="' + esc(dbMeta.max_rows || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '<div class="field"><label>' + esc(t('dbs.maxSizeMb')) + '</label><input type="number" id="edb-max-size" min="0" value="' + esc(dbMeta.max_size_mb || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
        '</div>' +
        '<label class="checkbox-row"><input type="checkbox" id="edb-fk"' + (dbMeta.foreign_keys !== false ? ' checked' : '') + '> ' + esc(t('dbs.foreignKeys')) + '</label>' +
        '<label class="checkbox-row"><input type="checkbox" id="edb-ro"' + (dbMeta.read_only ? ' checked' : '') + '> ' + esc(t('dbs.readOnly')) + '</label>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="edb-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="edit-db-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
      onMount: function () {
        loadResourceGroupOptions();
        $('edb-cancel').addEventListener('click', closeModal);
        $('edit-db-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var maxRowsRaw = $('edb-max-rows').value.trim();
          var maxSizeRaw = $('edb-max-size').value.trim();
          try {
            await api('/_api/v1/dbs/' + enc(dbMeta.id), {
              method: 'PATCH',
              json: {
                group: $('resource-group').value.trim(),
                description: $('edb-desc').value.trim(),
                max_rows: maxRowsRaw ? Number(maxRowsRaw) : 0,
                max_size_mb: maxSizeRaw ? Number(maxSizeRaw) : 0,
                foreign_keys: $('edb-fk').checked,
                read_only: $('edb-ro').checked
              }
            });
            closeModal();
            toast(t('dbs.updated'), 'success');
            renderDbs(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  var searchState = {
    indexes: [],
    selected: '',
    query: '',
    fuzzy: true,
    sort: '',
    vectorText: '',
    alpha: '0.5',
    facets: {},
    result: null,
    offset: 0
  };

  async function runRealSearchQuery(selectedId) {
    var reqToken = state.token;
    var qSeq = ++searchQuerySeq;
    var payload = {
      query: searchState.query,
      fuzzy: Boolean(searchState.fuzzy),
      facets: searchState.facets || {}
    };
    if (searchState.sort) payload.sort = searchState.sort;
    var vecRaw = (searchState.vectorText || '').trim();
    if (vecRaw) {
      var parsedVec = JSON.parse(vecRaw);
      if (!Array.isArray(parsedVec)) {
        throw new Error('Vector must be a JSON array of floats');
      }
      payload.vector = parsedVec.map(Number);
      var alphaNum = Number(searchState.alpha);
      if (!Number.isNaN(alphaNum)) payload.alpha = alphaNum;
    }
    var result = await api('/_api/v1/search/' + enc(selectedId) + '/query', {
      method: 'POST',
      json: payload
    });
    if (
      state.currentRoute !== 'search' ||
      state.token !== reqToken ||
      searchState.selected !== selectedId ||
      searchQuerySeq !== qSeq
    ) {
      return;
    }
    searchState.result = result;
    renderSearch();
  }

  async function renderSearch() {
    var root = $('view-root');
    if (state.currentRoute !== 'search') return;
    var isCurrentSearch = beginViewRender('search');
    try {
      var data = await Promise.all([api('/_api/v1/search'), api('/_api/v1/apps')]);
      if (!isCurrentSearch()) return;
      searchState.indexes = data[0] || [];
      state.projects = data[1] || [];
      if (!searchState.indexes.some(function (item) { return item.id === searchState.selected; })) {
        searchState.selected = searchState.indexes.length ? searchState.indexes[0].id : '';
        searchState.result = null;
        searchState.facets = {};
        searchState.offset = 0;
      }
      var selected = searchState.indexes.find(function (item) { return item.id === searchState.selected; });
      if (selected && searchState.offset >= selected.doc_count) {
        searchState.offset = Math.max(0, Math.ceil(selected.doc_count / 100) * 100 - 100);
      }
      var docs = selected ? await api('/_api/v1/search/' + enc(selected.id) + '/docs?limit=101&offset=' + searchState.offset) : [];
      if (!isCurrentSearch()) return;
      var hasMoreDocs = docs.length > 100;
      docs = docs.slice(0, 100);
      var writable = selected && canWriteResource(selected.project_id);
      var options = searchState.indexes.map(function (item) {
        return '<option value="' + esc(item.id) + '"' + (item.id === searchState.selected ? ' selected' : '') +
          '>' + esc(item.name) + '</option>';
      }).join('');
      var facetEntries = (searchState.result && searchState.result.facets) ? Object.keys(searchState.result.facets) : [];
      var facetsHtml = facetEntries.length ? '<div class="row gap-xs">' + facetEntries.map(function (field) {
        var vals = searchState.result.facets[field] || {};
        return Object.keys(vals).map(function (val) {
          var active = searchState.facets[field] === val;
          return '<button type="button" class="btn btn-sm ' + (active ? 'btn-primary' : '') + '" data-search-facet="' + esc(field) + '" data-facet-val="' + esc(val) + '">' + esc(field + ':' + val + ' (' + vals[val] + ')') + '</button>';
        }).join('');
      }).join('') + '</div>' : '';

      root.innerHTML = '<div class="page-head"><h2 class="page-title">' + esc(t('search.title')) + '</h2>' +
        (!isReadonly() ? '<button class="btn btn-primary" id="search-create-index">' + esc(t('search.newIndex')) + '</button>' : '') + '</div>' +
        '<div class="panel"><div class="panel-body stack gap-md">' +
        '<select id="search-index-select">' + options + '</select>' +
        (selected ? '<form id="real-search-form" class="stack gap-sm">' +
          '<div class="row">' +
          '<input id="real-search-query" value="' + esc(searchState.query) + '" placeholder="' + esc(t('search.queryPlaceholder')) + '" style="flex:1 1 220px;">' +
          '<label class="checkbox-row"><input type="checkbox" id="real-search-fuzzy"' + (searchState.fuzzy ? ' checked' : '') + '> ' + esc(t('search.fuzzy')) + '</label>' +
          '<select id="real-search-sort" style="width:auto;">' +
          '<option value=""' + (!searchState.sort ? ' selected' : '') + '>' + esc(t('search.sortRelevance')) + '</option>' +
          '<option value="updated_desc"' + (searchState.sort === 'updated_desc' ? ' selected' : '') + '>' + esc(t('search.sortUpdated')) + '</option>' +
          '</select>' +
          '<button class="btn btn-primary" type="submit">' + esc(t('common.search')) + '</button>' +
          '</div>' +
          '<details class="adv-options"' + (searchState.vectorText ? ' open' : '') + '><summary>' + esc(t('search.hybridVector')) + '</summary>' +
          '<div class="adv-options-body form-grid">' +
          '<div class="field"><label>' + esc(t('search.vectorArray')) + '</label><input type="text" id="real-search-vector" class="mono" value="' + esc(searchState.vectorText) + '" placeholder="[0.12, -0.34, 0.56]"></div>' +
          '<div class="field"><label>' + esc(t('search.vectorAlpha')) + '</label><input type="number" id="real-search-alpha" min="0" max="1" step="0.05" value="' + esc(searchState.alpha) + '"></div>' +
          '</div></details></form>' +
          facetsHtml +
          (searchState.result ? '<pre class="code-block">' + esc(JSON.stringify(searchState.result, null, 2)) + '</pre>' : '') +
          (writable ? '<form id="real-search-doc-form" class="stack gap-sm"><label>' + esc(t('search.docJsonLabel')) + '</label>' +
          '<textarea id="real-search-doc" rows="5" required>{ "id": "", "title": "", "body": "" }</textarea>' +
          '<button class="btn btn-primary" type="submit">' + esc(t('common.save')) + '</button></form>' : '') +
          '<div class="stack gap-sm">' + docs.map(function (doc) {
            return '<div class="panel"><pre class="code-block">' + esc(JSON.stringify(doc, null, 2)) + '</pre>' +
              (writable && doc.id ? '<button class="btn btn-danger btn-sm" data-search-doc-delete="' + esc(doc.id) + '">' + esc(t('common.delete')) + '</button>' : '') + '</div>';
          }).join('') + '</div>' +
          '<div class="row gap-xs"><button class="btn btn-sm" id="search-prev"' + (searchState.offset === 0 ? ' disabled' : '') + '>' + esc(t('search.previousPage')) + '</button>' +
          '<button class="btn btn-sm" id="search-next"' + (!hasMoreDocs ? ' disabled' : '') + '>' + esc(t('search.nextPage')) + '</button></div>' +
          (writable ? '<div class="row gap-xs"><button class="btn" id="search-commit-index">' + esc(t('search.commitIndex')) + '</button><button class="btn" id="search-edit-index">' + esc(t('common.editConfig')) + '</button><button class="btn btn-danger" id="search-delete-index">' + esc(t('search.deleteIndex')) + '</button></div>' : '') : '') +
        '</div></div>';
      $('search-index-select').addEventListener('change', function (event) {
        searchState.selected = event.target.value;
        searchState.offset = 0;
        searchState.facets = {};
        searchState.result = null;
        renderSearch();
      });
      if ($('search-commit-index') && selected) {
        $('search-commit-index').addEventListener('click', async function () {
          try {
            await api('/_api/v1/search/' + enc(selected.id) + '/commit', { method: 'POST', json: {} });
            toast(t('search.committed'), 'success');
            renderSearch();
          } catch (e) { toast(e.message, 'error'); }
        });
      }
      root.querySelectorAll('[data-search-facet]').forEach(function (btn) {
        btn.addEventListener('click', async function () {
          var field = btn.getAttribute('data-search-facet');
          var val = btn.getAttribute('data-facet-val');
          if (searchState.facets[field] === val) delete searchState.facets[field];
          else searchState.facets[field] = val;
          try { await runRealSearchQuery(selected.id); } catch (e) { toast(e.message, 'error'); }
        });
      });
      if ($('search-prev')) $('search-prev').addEventListener('click', function () {
        searchState.offset = Math.max(0, searchState.offset - 100);
        renderSearch();
      });
      if ($('search-next')) $('search-next').addEventListener('click', function () {
        searchState.offset += 100;
        renderSearch();
      });
      if ($('search-edit-index') && selected) $('search-edit-index').addEventListener('click', function () {
        openModal({
          title: t('common.editConfig') + ' · ' + selected.name,
          body:
            '<form id="edit-search-index-form" class="stack gap-md">' +
            resourceGroupField(selected.group || '') +
            '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="esi-desc" value="' + esc(selected.description || '') + '" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
            '<div class="field"><label>' + esc(t('search.writerBudgetMb')) + '</label><input type="number" id="esi-budget" min="15" max="256" value="' + esc(selected.writer_budget_mb || 20) + '"></div>' +
            '</form>',
          footer:
            '<button type="button" class="btn" id="esi-cancel">' + esc(t('common.cancel')) + '</button>' +
            '<button form="edit-search-index-form" class="btn btn-primary" type="submit">' + esc(t('common.save')) + '</button>',
          onMount: function () {
            loadResourceGroupOptions();
            $('esi-cancel').addEventListener('click', closeModal);
            $('edit-search-index-form').addEventListener('submit', async function (event) {
              event.preventDefault();
              try {
                await api('/_api/v1/search/' + enc(selected.id), {
                  method: 'PATCH',
                  json: {
                    group: $('resource-group').value.trim(),
                    description: $('esi-desc').value.trim(),
                    writer_budget_mb: Number($('esi-budget').value || 20)
                  }
                });
                closeModal();
                toast(t('search.indexUpdated'), 'success');
                renderSearch();
              } catch (e) { toast(e.message, 'error'); }
            });
          }
        });
      });
      if ($('search-create-index')) $('search-create-index').addEventListener('click', function () {
        openModal({
          title: t('search.newIndex'),
          body: '<form id="search-index-form" class="stack gap-md">' +
            '<div class="field"><label>' + esc(t('common.name')) + '</label><input id="search-index-name" required placeholder="' + esc(t('search.indexNamePlaceholder')) + '"></div>' +
            resourceGroupField() +
            '<div class="field"><label>' + esc(t('common.project')) + '</label><select id="search-index-owner">' + renderProjectSelectOptions('', true) + '</select></div>' +
            '<details class="adv-options"><summary>' + esc(t('common.advancedOptions')) + '</summary>' +
            '<div class="adv-options-body stack gap-sm">' +
            '<div class="field"><label>' + esc(t('common.description')) + '</label><input type="text" id="search-index-desc" placeholder="' + esc(t('common.descriptionPlaceholder')) + '"></div>' +
            '<div class="form-grid">' +
            '<div class="field"><label>' + esc(t('search.tokenizer')) + '</label><select id="search-index-tok">' +
            '<option value="cjk_jieba">cjk_jieba (CJK / Chinese & English)</option>' +
            '<option value="unicode61">unicode61 (Standard)</option>' +
            '<option value="ngram3">ngram3 (Substring)</option>' +
            '<option value="raw">raw (Exact)</option>' +
            '</select></div>' +
            '<div class="field"><label>' + esc(t('search.writerBudgetMb')) + '</label><input type="number" id="search-index-budget" min="15" max="256" value="20"></div>' +
            '</div>' +
            '<div class="field"><label>' + esc(t('search.facetFields')) + '</label><input type="text" id="search-index-facets" placeholder="category,status,lang,source,level"></div>' +
            '</div></details>' +
            '</form>',
          footer: '<button form="search-index-form" class="btn btn-primary" type="submit">' + esc(t('common.create')) + '</button>',
          onMount: function () {
            loadResourceGroupOptions();
            $('search-index-form').addEventListener('submit', async function (event) {
              event.preventDefault();
              var facetsRaw = $('search-index-facets').value.trim();
              var facets = facetsRaw ? facetsRaw.split(',').map(function (s) { return s.trim(); }).filter(Boolean) : null;
              var budgetRaw = $('search-index-budget').value.trim();
              try {
                var created = await api('/_api/v1/search', { method: 'POST', json: {
                  name: $('search-index-name').value.trim(),
                  group: selectedResourceGroup(),
                  project_id: $('search-index-owner').value || null,
                  description: $('search-index-desc').value.trim() || null,
                  tokenizer: $('search-index-tok').value || null,
                  writer_budget_mb: budgetRaw ? Number(budgetRaw) : null,
                  facet_fields: facets
                } });
                searchState.selected = created.id;
                searchState.offset = 0;
                closeModal();
                renderSearch();
              } catch (e) { toast(e.message, 'error'); }
            });
          }
        });
      });
      if ($('real-search-form')) $('real-search-form').addEventListener('submit', async function (event) {
        event.preventDefault();
        searchState.query = $('real-search-query').value;
        searchState.fuzzy = Boolean($('real-search-fuzzy') && $('real-search-fuzzy').checked);
        searchState.sort = $('real-search-sort') ? $('real-search-sort').value : '';
        searchState.vectorText = $('real-search-vector') ? $('real-search-vector').value : '';
        searchState.alpha = $('real-search-alpha') ? $('real-search-alpha').value : '0.5';
        try {
          await runRealSearchQuery(selected.id);
        } catch (e) { toast(e.message, 'error'); }
      });
      if ($('real-search-doc-form')) $('real-search-doc-form').addEventListener('submit', async function (event) {
        event.preventDefault();
        var targetIndexId = selected.id;
        var reqToken = state.token;
        try {
          var doc = JSON.parse($('real-search-doc').value);
          await api('/_api/v1/search/' + enc(targetIndexId) + '/docs', { method: 'POST', json: doc });
          if (state.currentRoute !== 'search' || state.token !== reqToken || searchState.selected !== targetIndexId) return;
          searchState.result = null;
          searchState.offset = 0;
          renderSearch();
        } catch (e) { toast(e.message, 'error'); }
      });
      root.querySelectorAll('[data-search-doc-delete]').forEach(function (button) {
        button.addEventListener('click', function () {
          var targetIndexId = selected.id;
          var docId = button.getAttribute('data-search-doc-delete');
          var reqToken = state.token;
          confirmAction(t('common.confirmDeleteTitle'), async function () {
            try {
              await api('/_api/v1/search/' + enc(targetIndexId) + '/docs/' + enc(docId), { method: 'DELETE' });
              if (state.currentRoute !== 'search' || state.token !== reqToken || searchState.selected !== targetIndexId) return;
              searchState.result = null;
              renderSearch();
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      });
      if ($('search-delete-index')) $('search-delete-index').addEventListener('click', function () {
        var targetIndexId = selected.id;
        var reqToken = state.token;
        confirmAction(t('search.deleteIndexConfirm', { name: selected.name }), async function () {
          try {
            await api('/_api/v1/search/' + enc(targetIndexId), { method: 'DELETE' });
            if (state.currentRoute !== 'search' || state.token !== reqToken) return;
            searchState.selected = '';
            searchState.offset = 0;
            searchState.result = null;
            renderSearch();
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    } catch (e) {
      if (isCurrentSearch()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
    }
  }

  async function renderRuntime() {
    var root = $('view-root');
    var isCurrent = beginViewRender('apps');
    root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    try {
      await ensureRuntimeComponent();
      if (!isCurrent()) return;
      var res = await Promise.all([api('/_api/v1/apps'), api('/_api/v1/routes')]);
      if (!isCurrent()) return;
      state.projects = res[0] || [];
      state.routes = res[1] || [];
    } catch (e) {
      if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var rp = document.createElement('fp-runtime-panel');
    rp.token = state.token || readStoredToken();
    rp.canWrite = isAdmin();
    rp.status = state.status;
    rp.projects = state.projects;
    rp.routes = state.routes;
    rp.addEventListener('fp-create-runtime', openCreateRuntimeModal);
    root.appendChild(rp);
  }

  async function renderOps(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('ops');
    root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    try {
      await ensureOpsComponent();
      if (!isCurrent()) return;
      if (fetchData && !state.projects.length) {
        try {
          var projs = (await api('/_api/v1/apps')) || [];
          if (!isCurrent()) return;
          state.projects = projs;
        } catch (_) {}
      }
    } catch (e) {
      if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var ops = document.createElement('fp-ops-center');
    ops.token = state.token || readStoredToken();
    ops.scope = getScope();
    ops.projects = state.projects;
    root.appendChild(ops);
  }

  async function renderUpdates() {
    var root = $('view-root');
    var isCurrent = beginViewRender('updates');
    root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    try {
      await ensureUpdateComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var upd = document.createElement('fp-system-updates');
    upd.token = state.token || readStoredToken();
    upd.scope = getScope();
    root.appendChild(upd);
  }

  async function renderAi() {
    var root = $('view-root');
    var isCurrent = beginViewRender('ai');
    root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    try {
      await ensureAiComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var panel = document.createElement('fp-ai-gateway');
    panel.token = state.token || readStoredToken();
    panel.scope = getScope();
    root.appendChild(panel);
  }

  async function renderSettings() {
    var root = $('view-root');
    var isCurrent = beginViewRender('settings');
    root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    try {
      await ensureSettingsComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var panel = document.createElement('fp-platform-settings');
    panel.token = state.token || readStoredToken();
    panel.scope = getScope();
    root.appendChild(panel);
  }

  // --- 8. Tokens (Admin Only) ---

  async function renderTokens(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('tokens');
    if (fetchData) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      try {
        var res = await Promise.all([api('/_api/v1/tokens'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.tokens = res[0] || [];
        state.projects = res[1] || [];
      } catch (e) {
        if (isCurrent()) root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        return;
      }
    }
    if (!isCurrent()) return;

    var rows = state.tokens.map(function (tok) {
      var scopeStr = typeof tok.scope === 'string'
        ? tok.scope
        : (tok.scope && tok.scope.project ? 'project:' + tok.scope.project : JSON.stringify(tok.scope));
      return '<tr>' +
        '<td data-label="' + esc(t('common.name')) + '"><div class="text-strong">' + esc(tok.name) + '</div><div class="muted mono text-xs">' + esc(tok.id) + '</div></td>' +
        '<td data-label="' + esc(t('tokens.scope')) + '"><span class="badge badge-accent">' + esc(scopeStr) + '</span></td>' +
        '<td data-label="' + esc(t('common.expiresAt')) + '" class="text-xs muted">' + esc(tok.expires_at ? formatTime(tok.expires_at) : t('common.never')) + '</td>' +
        '<td data-label="' + esc(t('tokens.lastUsed')) + '" class="text-xs muted">' + esc(tok.last_used_at ? formatTime(tok.last_used_at) : '-') + '</td>' +
        '<td data-label="' + esc(t('common.createdAt')) + '" class="text-xs muted">' + esc(formatTime(tok.created_at)) + '</td>' +
        '<td class="cell-actions-td"><div class="cell-actions">' +
        '<button type="button" class="btn btn-danger btn-sm" data-revoke-tok="' + esc(tok.id) + '" data-tok-name="' + esc(tok.name) + '">' + esc(t('tokens.revoke')) + '</button>' +
        '</div></td></tr>';
    }).join('');

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('tokens.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('tokens.subtitle')) + '</p></div>' +
      '<button type="button" class="btn btn-primary" id="new-token-btn">' + esc(t('tokens.new')) + '</button></div>' +
      '<div class="panel">' +
      (rows
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('common.name')) + '</th><th>' + esc(t('tokens.scope')) + '</th><th>' + esc(t('common.expiresAt')) + '</th><th>' + esc(t('tokens.lastUsed')) + '</th><th>' + esc(t('common.createdAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + rows + '</tbody></table></div>'
        : '<div class="empty-state">' + esc(t('tokens.empty')) + '</div>') +
      '</div>';

    $('new-token-btn').addEventListener('click', openCreateTokenModal);
    root.querySelectorAll('[data-revoke-tok]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var id = btn.getAttribute('data-revoke-tok');
        var name = btn.getAttribute('data-tok-name');
        confirmAction(t('tokens.revokeConfirm', { name: name }), async function () {
          try {
            await api('/_api/v1/tokens/' + enc(id), { method: 'DELETE' });
            toast(t('tokens.revoked'), 'success');
            renderTokens(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });
  }

  function openCreateTokenModal() {
    openModal({
      title: t('tokens.new'),
      body:
        '<form id="create-tok-form" class="stack gap-md">' +
        '<div class="field"><label>' + esc(t('tokens.name')) + '</label><input type="text" id="ct-name" required placeholder="' + esc(t('tokens.namePlaceholder')) + '"></div>' +
        '<div class="field"><label>' + esc(t('tokens.scope')) + '</label><select id="ct-scope">' +
        '<option value="admin">' + esc(t('tokens.scopeAdmin')) + '</option>' +
        '<option value="readonly">' + esc(t('tokens.scopeReadonly')) + '</option>' +
        (state.projects.length ? '<option value="project">' + esc(t('tokens.scopeProject')) + '</option>' : '') +
        '</select></div>' +
        '<div class="field hidden" id="ct-proj-wrap"><label>' + esc(t('tokens.selectProject')) + '</label><select id="ct-proj">' + renderProjectSelectOptions('', false) + '</select></div>' +
        '<div class="field"><label>' + esc(t('tokens.expiresDays')) + '</label><input type="number" id="ct-exp" min="1" placeholder="' + esc(t('tokens.expiresPlaceholder')) + '"></div>' +
        '</form>',
      footer:
        '<button type="button" class="btn" id="ct-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="submit" form="create-tok-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
      onMount: function () {
        $('ct-cancel').addEventListener('click', closeModal);
        $('ct-scope').addEventListener('change', function (e) {
          $('ct-proj-wrap').classList.toggle('hidden', e.target.value !== 'project');
        });
        $('create-tok-form').addEventListener('submit', async function (ev) {
          ev.preventDefault();
          var sType = $('ct-scope').value;
          var scope = sType === 'project' ? 'project:' + $('ct-proj').value : sType;
          var expRaw = $('ct-exp').value.trim();
          try {
            var res = await api('/_api/v1/tokens', {
              method: 'POST',
              json: {
                name: $('ct-name').value.trim(),
                scope: scope,
                expires_in_days: expRaw ? Number(expRaw) : null
              }
            });
            openTokenSecretModal(res.token);
            renderTokens(true);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
    });
  }

  function openTokenSecretModal(rawToken) {
    openModal({
      title: t('tokens.createdNoticeTitle'),
      body:
        '<p class="muted" style="margin:0;">' + esc(t('tokens.createdNoticeBody')) + '</p>' +
        '<pre class="code-block">' + esc(rawToken) + '</pre>',
      footer:
        '<button type="button" class="btn" id="ts-copy">' + esc(t('common.copy')) + '</button>' +
        '<button type="button" class="btn btn-primary" id="ts-done">' + esc(t('common.close')) + '</button>',
      onMount: function () {
        $('ts-copy').addEventListener('click', function () { copyText(rawToken); });
        $('ts-done').addEventListener('click', closeModal);
      }
    });
  }

  // --- 9. Logs (SSE Live Tail + History Query) ---

  function stopSse() {
    if (state.sseSource) {
      state.sseSource.close();
      state.sseSource = null;
    }
    state.sseStatus = 'disconnected';
  }

  function startSse() {
    stopSse();
    var url = '/_api/v1/logs/stream?token=' + enc(state.token);
    var src = new EventSource(url);
    state.sseSource = src;
    state.sseStatus = 'connected';
    src.onmessage = function (ev) {
      if (state.sseStatus === 'paused') return;
      try {
        var entry = JSON.parse(ev.data);
        state.liveLogs.push(entry);
        if (state.liveLogs.length > 500) {
          state.liveLogs.splice(0, state.liveLogs.length - 500);
        }
        if (state.currentRoute === 'logs' && state.logsTab === 'live') {
          renderLogRowsInPlace('live-log-list', filterLiveLogs());
        }
      } catch (_) {}
    };
    src.onerror = function () {
      if (state.currentRoute === 'logs' && state.logsTab === 'live') {
        updateSseBadge();
      }
    };
  }

  function filterLiveLogs() {
    var lv = state.liveFilterLevel;
    var sr = state.liveFilterSource;
    var q = state.liveFilterSearch.trim().toLowerCase();
    return state.liveLogs.filter(function (e) {
      if (lv && e.level !== lv) return false;
      if (sr && e.source !== sr) return false;
      if (q) {
        var msg = (e.message || '').toLowerCase();
        var rid = (e.request_id || '').toLowerCase();
        if (msg.indexOf(q) === -1 && rid.indexOf(q) === -1) return false;
      }
      return true;
    });
  }

  function logLevelBadge(level) {
    if (level === 'error') return '<span class="badge badge-danger">' + esc(level) + '</span>';
    if (level === 'warn') return '<span class="badge badge-warn">' + esc(level) + '</span>';
    if (level === 'debug') return '<span class="badge badge-neutral">' + esc(level) + '</span>';
    return '<span class="badge badge-info">' + esc(level || 'info') + '</span>';
  }

  function highlightText(text, query) {
    var str = String(text === null || text === undefined ? '' : text);
    var terms = String(query || '').trim().split(/\s+/).filter(Boolean);
    if (!terms.length) return esc(str);
    var escaped = terms
      .slice()
      .sort(function (a, b) { return b.length - a.length; })
      .map(function (term) { return term.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); });
    var re = new RegExp('(' + escaped.join('|') + ')', 'gi');
    return str.split(re).map(function (part, idx) {
      return idx % 2 === 1 ? '<mark class="fts-hl">' + esc(part) + '</mark>' : esc(part);
    }).join('');
  }

  function buildLogRowsHtml(entries, emptyText, highlightQuery) {
    if (!entries || !entries.length) {
      return '<div class="empty-state">' + esc(emptyText) + '</div>';
    }
    var hl = highlightQuery
      ? function (s) { return highlightText(s, highlightQuery); }
      : esc;
    return entries.map(function (e) {
      var metaHtml = e.meta_json
        ? '<details class="log-meta"><summary>' + esc(t('logs.meta')) + '</summary><pre class="code-block" style="margin-top:6px;">' + hl(e.meta_json) + '</pre></details>'
        : '';
      return '<div class="log-row">' +
        '<div class="log-row-head">' +
        '<span class="muted mono">' + esc(formatTime(e.ts)) + '</span>' +
        logLevelBadge(e.level) +
        '<span class="badge badge-neutral">' + esc(e.source) + '</span>' +
        (e.project_id ? '<span class="badge badge-accent">' + esc(projectNameById(e.project_id)) + '</span>' : '') +
        (e.request_id ? '<span class="mono muted">' + hl(e.request_id) + '</span>' : '') +
        '</div>' +
        '<div class="log-msg">' + hl(e.message) + '</div>' +
        metaHtml + '</div>';
    }).join('');
  }

  function renderLogRowsInPlace(containerId, entries) {
    var el = $(containerId);
    if (!el) return;
    var atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    el.innerHTML = buildLogRowsHtml(entries, state.sseStatus === 'disconnected' ? t('logs.liveHint') : t('logs.noLogs'), state.liveFilterSearch);
    if (atBottom) el.scrollTop = el.scrollHeight;
  }

  function updateSseBadge() {
    var badge = $('sse-status-badge');
    if (!badge) return;
    if (state.sseStatus === 'connected') {
      badge.className = 'badge badge-success';
      badge.innerHTML = '<span class="dot"></span>' + esc(t('logs.connected'));
    } else if (state.sseStatus === 'paused') {
      badge.className = 'badge badge-warn';
      badge.innerHTML = '<span class="dot"></span>' + esc(t('logs.paused'));
    } else {
      badge.className = 'badge badge-neutral';
      badge.innerHTML = '<span class="dot"></span>' + esc(t('logs.disconnected'));
    }
  }

  async function renderLogs(fetchData) {
    var root = $('view-root');
    var reqToken = state.token;
    var isCurrent = beginViewRender('logs');
    if (fetchData && !state.projects.length) {
      try {
        var projs = (await api('/_api/v1/apps')) || [];
        if (!isCurrent()) return;
        state.projects = projs;
      } catch (_) {}
    }
    if (fetchData && state.logsTab === 'history') {
      await ensureSearchBundle();
      if (!isCurrent()) return;
      await queryHistoryLogs();
      if (!isCurrent()) return;
    } else if (state.logsTab === 'history') {
      await ensureSearchBundle();
      if (!isCurrent()) return;
    }
    if (!isCurrent()) return;

    var tabsHtml =
      '<div class="tabs">' +
      '<button type="button" class="tab-btn' + (state.logsTab === 'live' ? ' active' : '') + '" data-logs-tab="live">' + esc(t('logs.tabLive')) + '</button>' +
      '<button type="button" class="tab-btn' + (state.logsTab === 'history' ? ' active' : '') + '" data-logs-tab="history">' + esc(t('logs.tabHistory')) + '</button>' +
      '</div>';

    var bodyHtml = '';
    if (state.logsTab === 'live') {
      var isConn = state.sseStatus !== 'disconnected';
      bodyHtml =
        '<div class="panel"><div class="panel-header"><div class="toolbar">' +
        '<div class="row">' +
        '<span id="sse-status-badge" class="badge badge-neutral"></span>' +
        '<button type="button" class="btn btn-primary btn-sm" id="sse-toggle-btn">' + esc(isConn ? t('logs.disconnect') : t('logs.connect')) + '</button>' +
        (isConn ? '<button type="button" class="btn btn-sm" id="sse-pause-btn">' + esc(state.sseStatus === 'paused' ? t('logs.resume') : t('logs.pause')) + '</button>' : '') +
        '<button type="button" class="btn btn-sm" id="sse-clear-btn">' + esc(t('logs.clear')) + '</button>' +
        '</div>' +
        '<div class="toolbar-filters">' +
        '<select id="live-source" style="width:auto;">' +
        '<option value="">' + esc(t('logs.allSources')) + '</option>' +
        ['access', 'runtime', 'queue', 'cron', 'audit', 'system'].map(function (s) {
          return '<option value="' + s + '"' + (state.liveFilterSource === s ? ' selected' : '') + '>' + s + '</option>';
        }).join('') + '</select>' +
        '<select id="live-level" style="width:auto;">' +
        '<option value="">' + esc(t('logs.allLevels')) + '</option>' +
        ['info', 'warn', 'error', 'debug'].map(function (l) {
          return '<option value="' + l + '"' + (state.liveFilterLevel === l ? ' selected' : '') + '>' + l + '</option>';
        }).join('') + '</select>' +
        '<input type="search" id="live-search" style="max-width:220px;" value="' + esc(state.liveFilterSearch) + '" placeholder="' + esc(t('logs.searchPlaceholder')) + '">' +
        '</div></div></div>' +
        '<div id="live-log-list" class="log-list">' +
        buildLogRowsHtml(filterLiveLogs(), state.sseStatus === 'disconnected' ? t('logs.liveHint') : t('logs.noLogs'), state.liveFilterSearch) +
        '</div></div>';
    } else {
      bodyHtml = '<fp-log-search-panel id="log-fts-panel"></fp-log-search-panel>';
    }

    root.innerHTML =
      '<div class="page-head"><div><h2 class="page-title">' + esc(t('logs.title')) + '</h2>' +
      '<p class="page-subtitle">' + esc(t('logs.subtitle')) + '</p></div></div>' +
      tabsHtml + bodyHtml;

    updateSseBadge();

    root.querySelectorAll('[data-logs-tab]').forEach(function (btn) {
      btn.addEventListener('click', async function () {
        state.logsTab = btn.getAttribute('data-logs-tab');
        if (state.logsTab === 'history') {
          try { await queryHistoryLogs(); } catch (e) { toast(e.message, 'error'); }
        }
        if (state.currentRoute === 'logs' && state.token === reqToken) {
          renderLogs(false);
        }
      });
    });

    if ($('sse-toggle-btn')) {
      $('sse-toggle-btn').addEventListener('click', function () {
        if (state.sseStatus === 'disconnected') {
          startSse();
        } else {
          stopSse();
        }
        renderLogs(false);
      });
    }
    if ($('sse-pause-btn')) {
      $('sse-pause-btn').addEventListener('click', function () {
        state.sseStatus = state.sseStatus === 'paused' ? 'connected' : 'paused';
        renderLogs(false);
      });
    }
    if ($('sse-clear-btn')) {
      $('sse-clear-btn').addEventListener('click', function () {
        state.liveLogs = [];
        renderLogs(false);
      });
    }
    if ($('live-source')) {
      $('live-source').addEventListener('change', function (e) {
        state.liveFilterSource = e.target.value;
        renderLogRowsInPlace('live-log-list', filterLiveLogs());
      });
    }
    if ($('live-level')) {
      $('live-level').addEventListener('change', function (e) {
        state.liveFilterLevel = e.target.value;
        renderLogRowsInPlace('live-log-list', filterLiveLogs());
      });
    }
    if ($('live-search')) {
      $('live-search').addEventListener('input', function (e) {
        state.liveFilterSearch = e.target.value;
        renderLogRowsInPlace('live-log-list', filterLiveLogs());
      });
    }
    if ($('log-fts-panel')) {
      var panel = $('log-fts-panel');
      panel.projects = state.projects;
      panel.scope = getScope();
      panel.filters = Object.assign({}, state.historyFilters);
      panel.liveEntries = state.historyLogs;
      panel.addEventListener('fp-log-query', async function (ev) {
        var d = ev.detail || {};
        state.historyFilters = {
          q: d.q || '',
          source: d.source || '',
          level: d.level || '',
          project_id: d.project_id || '',
          request_id: d.request_id || '',
          windowSecs: d.windowSecs || '',
          limit: d.limit || '100'
        };
        try {
          await queryHistoryLogs();
          if (panel.isConnected && state.currentRoute === 'logs' && state.token === reqToken) {
            panel.liveEntries = state.historyLogs;
          }
        } catch (e) { toast(e.message, 'error'); }
      });
    }
  }

  async function queryHistoryLogs() {
    var hf = state.historyFilters;
    var params = [];
    if (hf.q) params.push('q=' + enc(hf.q));
    if (hf.source) params.push('source=' + enc(hf.source));
    if (hf.level) params.push('level=' + enc(hf.level));
    if (hf.project_id) params.push('project_id=' + enc(hf.project_id));
    if (hf.request_id) params.push('request_id=' + enc(hf.request_id));
    if (hf.windowSecs) {
      var since = Date.now() - Number(hf.windowSecs) * 1000;
      params.push('since_ts=' + since);
    }
    if (hf.limit) params.push('limit=' + enc(hf.limit));
    var qs = params.length ? '?' + params.join('&') : '';
    state.historyLogs = (await api('/_api/v1/logs' + qs)) || [];
  }

  // --- Bootstrap & Event Wiring ---

  async function init() {
    populateLangSelects();

    document.querySelectorAll('[data-action="select-lang"]').forEach(function (sel) {
      sel.addEventListener('change', function () {
        selectLang(sel.value);
      });
    });
    document.querySelectorAll('[data-action="toggle-theme"]').forEach(function (btn) {
      btn.addEventListener('click', toggleTheme);
    });
    if ($('refresh-btn')) {
      $('refresh-btn').addEventListener('click', function () { renderCurrentRoute(true); });
    }
    if ($('open-drawer-btn')) $('open-drawer-btn').addEventListener('click', openDrawer);
    if ($('mobile-more-btn')) $('mobile-more-btn').addEventListener('click', openDrawer);
    if ($('close-drawer-btn')) $('close-drawer-btn').addEventListener('click', closeDrawer);
    if ($('drawer-backdrop')) $('drawer-backdrop').addEventListener('click', closeDrawer);
    if ($('disconnect-btn')) $('disconnect-btn').addEventListener('click', function () { disconnect(false); });

    document.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape') {
        closeModal();
        closeDrawer();
      }
    });

    ['gesturestart', 'gesturechange', 'gestureend'].forEach(function (evt) {
      document.addEventListener(evt, function (ev) { ev.preventDefault(); }, { passive: false });
    });
    document.addEventListener('touchstart', function (ev) {
      if (ev.touches && ev.touches.length > 1) ev.preventDefault();
    }, { passive: false });
    document.addEventListener('touchmove', function (ev) {
      if (ev.touches && ev.touches.length > 1) ev.preventDefault();
    }, { passive: false });

    document.addEventListener('fp-toast', function (ev) {
      if (ev.detail && ev.detail.message) {
        toast(ev.detail.message, ev.detail.kind || 'info');
      }
    });

    document.addEventListener('fp-navigate', function (ev) {
      if (ev.detail && ev.detail.hash) {
        if (ev.detail.projectTab) {
          state.pendingRuntimeTab = ev.detail.projectTab;
        }
        navigate(ev.detail.hash);
      }
    });

    document.addEventListener('fp-settings-changed', async function () {
      if (!state.token) return;
      try {
        var st = await api('/_api/v1/status', { skipAuthRedirect: true });
        state.status = st;
        applyDocumentTitle();
      } catch (_) {}
    });

    document.addEventListener('click', function (ev) {
      var link = ev.target && ev.target.closest ? ev.target.closest('a[href^="#/"]') : null;
      if (!link || ev.metaKey || ev.ctrlKey || ev.shiftKey || ev.altKey || ev.button !== 0) return;
      ev.preventDefault();
      var tab = link.getAttribute('data-runtime-tab');
      if (tab) state.pendingRuntimeTab = tab;
      navigate(link.getAttribute('href'));
    });

    function onBrowserRouteChange() {
      if (!state.status) {
        var tok = state.token || readStoredToken();
        if (tok) {
          connectWithToken(tok, true);
        }
        return;
      }
      parseHash();
      closeDrawer();
      renderCurrentRoute(true);
    }

    window.addEventListener('popstate', onBrowserRouteChange);
    window.addEventListener('hashchange', onBrowserRouteChange);

    $('toggle-token-vis').addEventListener('click', function () {
      var inp = $('token-input');
      var show = inp.type === 'password';
      inp.type = show ? 'text' : 'password';
      $('toggle-token-vis').textContent = show ? t('auth.hideToken') : t('auth.showToken');
    });

    $('login-form').addEventListener('submit', function (ev) {
      ev.preventDefault();
      connectWithToken($('token-input').value, false);
    });

    if (I18n.ready) {
      var loadedLocale = await I18n.ready();
      if (loadedLocale && loadedLocale !== I18n.getLocale()) {
        I18n.setLocale(loadedLocale);
      }
    }
    applyStaticI18n();

    var savedToken = readStoredToken();
    if (savedToken) {
      $('token-input').value = savedToken;
      connectWithToken(savedToken, true);
    } else {
      $('login-view').classList.remove('hidden');
    }
  }

  init();
})();
