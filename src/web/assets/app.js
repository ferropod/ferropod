(function () {
  'use strict';

  var I18n = window.FerroI18n;
  var t = I18n.t;
  var formatTime = I18n.formatTime;
  var formatBytes = I18n.formatBytes;

  var TOKEN_STORAGE = 'ferropod_token';
  var THEME_STORAGE = 'ferropod_theme';
  var SIDEBAR_STORAGE = 'ferropod_sidebar_collapsed';

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
    blobDir: '',
    blobSearch: '',
    blobFolders: [],
    blobObjects: [],
    blobTruncated: false,
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

  function showViewLoadingIfNeeded(root, viewKey) {
    if (!root) return;
    if (root.getAttribute('data-view-key') !== viewKey) {
      root.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
    }
  }

  function markViewRendered(root, viewKey) {
    if (root) root.setAttribute('data-view-key', viewKey);
  }

  function captureUiSnapshot(root) {
    var snap = {
      scrollX: window.scrollX || 0,
      scrollY: window.scrollY || 0,
      activeId: '',
      selStart: null,
      selEnd: null,
      openDetails: []
    };
    if (!root) return snap;
    var active = document.activeElement;
    if (active && root.contains(active) && active.id) {
      snap.activeId = active.id;
      try {
        if (typeof active.selectionStart === 'number' && typeof active.selectionEnd === 'number') {
          snap.selStart = active.selectionStart;
          snap.selEnd = active.selectionEnd;
        }
      } catch (_) {}
    }
    root.querySelectorAll('details').forEach(function (d) {
      snap.openDetails.push(Boolean(d.open));
    });
    return snap;
  }

  function restoreUiSnapshot(root, snap) {
    if (!root || !snap) return;
    if (snap.openDetails && snap.openDetails.length) {
      var details = root.querySelectorAll('details');
      for (var i = 0; i < details.length && i < snap.openDetails.length; i++) {
        if (snap.openDetails[i]) details[i].open = true;
      }
    }
    if (snap.activeId) {
      var el = $(snap.activeId);
      if (el && root.contains(el)) {
        try {
          el.focus({ preventScroll: true });
        } catch (_) {
          try { el.focus(); } catch (__) {}
        }
        if (snap.selStart !== null && snap.selEnd !== null) {
          try { el.setSelectionRange(snap.selStart, snap.selEnd); } catch (_) {}
        }
      }
    }
    window.scrollTo(snap.scrollX, snap.scrollY);
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
    document.querySelectorAll('[data-i18n-title]').forEach(function (el) {
      var label = t(el.getAttribute('data-i18n-title'));
      el.setAttribute('title', label);
      el.setAttribute('aria-label', label);
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

  function focusModalFirstField(root) {
    var body = $('modal-body-slot');
    if (!body || root.contains(document.activeElement)) return;
    var candidates = body.querySelectorAll('input:not([type="hidden"]):not([type="checkbox"]):not([type="file"]):not([disabled]):not([readonly]), textarea:not([disabled]):not([readonly]), select:not([disabled])');
    for (var i = 0; i < candidates.length; i += 1) {
      var el = candidates[i];
      if (!el.closest('.hidden') && !el.closest('details:not([open])') && el.offsetParent !== null) {
        el.focus();
        if (el.tagName === 'INPUT' && el.value && (el.type === 'text' || el.type === 'search' || el.type === 'url')) {
          try { el.setSelectionRange(el.value.length, el.value.length); } catch (_) {}
        }
        return;
      }
    }
  }

  function withSubmitLock(formSelectorOrEl, asyncFn) {
    var form = typeof formSelectorOrEl === 'string' ? $(formSelectorOrEl) : formSelectorOrEl;
    if (!form) return;
    var inFlight = false;
    form.addEventListener('submit', async function (ev) {
      ev.preventDefault();
      if (inFlight) return;
      inFlight = true;
      var buttons = [];
      form.querySelectorAll('button[type="submit"], input[type="submit"]').forEach(function (b) { buttons.push(b); });
      if (form.id) {
        document.querySelectorAll('button[type="submit"][form="' + form.id + '"]').forEach(function (b) {
          if (buttons.indexOf(b) === -1) buttons.push(b);
        });
      }
      buttons.forEach(function (b) { b.disabled = true; });
      try {
        await asyncFn(ev);
      } finally {
        inFlight = false;
        buttons.forEach(function (b) {
          if (b.isConnected) b.disabled = false;
        });
      }
    });
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
    focusModalFirstField(root);
  }

  function closeModal() {
    var root = $('modal-root');
    if (!root) return;
    root.classList.add('hidden');
    root.setAttribute('aria-hidden', 'true');
    root.innerHTML = '';
  }

  function confirmAction(message, onConfirm, opts) {
    opts = opts || {};
    var title = opts.title || t('common.confirmDeleteTitle');
    var confirmLabel = opts.confirmLabel || t('common.confirm');
    var btnClass = opts.danger === false ? 'btn btn-primary' : 'btn btn-danger';
    openModal({
      title: title,
      body: '<p style="margin:0;">' + esc(message) + '</p>',
      footer:
        '<button type="button" class="btn" id="confirm-cancel">' + esc(t('common.cancel')) + '</button>' +
        '<button type="button" class="' + btnClass + '" id="confirm-ok">' + esc(confirmLabel) + '</button>',
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

  function isDesktopViewport() {
    return window.matchMedia && window.matchMedia('(min-width: 960px)').matches;
  }

  function applySidebarCollapsed(collapsed) {
    var shell = $('app-shell');
    if (shell) shell.classList.toggle('sidebar-collapsed', Boolean(collapsed));
    var btn = $('open-drawer-btn');
    if (btn) btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
    try {
      localStorage.setItem(SIDEBAR_STORAGE, collapsed ? '1' : '0');
    } catch (_) {}
  }

  function toggleSidebar() {
    if (isDesktopViewport()) {
      var shell = $('app-shell');
      var collapsed = shell ? shell.classList.contains('sidebar-collapsed') : false;
      applySidebarCollapsed(!collapsed);
    } else if ($('sidebar').classList.contains('open')) {
      closeDrawer();
    } else {
      openDrawer();
    }
  }

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
    } else if (route === 'data') {
      history.replaceState(null, '', '#/dbs');
      route = 'dbs';
      parts = ['dbs'];
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

  function navSectionForRoute(route) {
    if (['logs', 'ops'].indexOf(route) !== -1) return 'ops';
    if (['settings', 'tokens', 'updates'].indexOf(route) !== -1) return 'settings';
    return route;
  }

  function updateNavActive() {
    var navSet = $('nav-settings');
    if (navSet) {
      navSet.classList.toggle('hidden', !isAdmin() && !isReadonly());
    }
    var activeSection = navSectionForRoute(state.currentRoute);
    document.querySelectorAll('[data-nav]').forEach(function (el) {
      var target = el.getAttribute('data-nav');
      el.classList.toggle('active', target === activeSection || target === state.currentRoute);
    });
    var titleKey = 'nav.' + state.currentRoute;
    $('topbar-title').textContent = t(titleKey);
  }

  function renderSectionTabs(section, actionsHtml) {
    var items = [];
    if (section === 'ops') {
      items = [
        { route: 'logs', href: '#/logs', label: t('nav.logs') },
        { route: 'ops', href: '#/ops', label: t('nav.ops') }
      ];
    } else if (section === 'settings') {
      items = [
        { route: 'settings', href: '#/settings', label: t('nav.settings') }
      ];
      if (isAdmin()) {
        items.push({ route: 'tokens', href: '#/tokens', label: t('nav.tokens') });
      }
      items.push({ route: 'updates', href: '#/updates', label: t('nav.updates') });
    }
    var tabs = items.map(function (it) {
      var active = state.currentRoute === it.route;
      return '<a href="' + it.href + '" class="subnav-tab' + (active ? ' active' : '') + '" role="tab" aria-selected="' + (active ? 'true' : 'false') + '">' + esc(it.label) + '</a>';
    }).join('');
    return '<div class="section-bar">' +
      '<div class="subnav-tabs" role="tablist">' + tabs + '</div>' +
      (actionsHtml ? '<div class="section-actions">' + actionsHtml + '</div>' : '') +
      '</div>';
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
      if (vr) {
        vr.removeAttribute('data-view-key');
        vr.innerHTML = '<div class="panel"><div class="empty-state">' + esc(t('common.loading')) + '</div></div>';
      }
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
    var vr = $('view-root');
    if (vr) vr.removeAttribute('data-view-key');
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

  function groupedSelectOptions(items, selectedValue, valueFn, labelFn) {
    var groups = new Map();
    items.forEach(function (item) {
      var name = item.group || '';
      if (!groups.has(name)) groups.set(name, []);
      groups.get(name).push(item);
    });
    function renderOpt(item) {
      var val = String(valueFn(item));
      var sel = val === String(selectedValue || '') ? ' selected' : '';
      return '<option value="' + esc(val) + '"' + sel + '>' + esc(labelFn(item)) + '</option>';
    }
    if (groups.size === 1 && groups.has('')) {
      return items.map(renderOpt).join('');
    }
    return Array.from(groups.keys()).sort().map(function (name) {
      return '<optgroup label="' + esc(name || t('groups.ungrouped')) + '">' +
        groups.get(name).map(renderOpt).join('') +
        '</optgroup>';
    }).join('');
  }

  function renderResourceBar(opts) {
    var items = opts.items || [];
    var optionsHtml = groupedSelectOptions(items, opts.selectedValue, opts.valueFn, opts.labelFn);
    if (opts.extraOption) {
      optionsHtml = '<option value="' + esc(opts.extraOption.value) + '"' +
        (String(opts.extraOption.value) === String(opts.selectedValue || '') ? ' selected' : '') + '>' +
        esc(opts.extraOption.label) + '</option>' + optionsHtml;
    }
    var selectHtml = (items.length > 0 || opts.extraOption)
      ? '<select id="' + esc(opts.selectId) + '" class="resource-select">' + optionsHtml + '</select>' +
        (items.length > 0 ? '<span class="badge badge-neutral mono">' + esc(items.length) + '</span>' : '')
      : '';
    var leftHtml = (selectHtml || opts.extraLeftHtml)
      ? '<div class="resource-switcher">' + selectHtml + (opts.extraLeftHtml || '') + '</div>'
      : '';
    var rightHtml = opts.actionHtml
      ? '<div class="section-actions">' + opts.actionHtml + '</div>'
      : '';
    if (!leftHtml && !rightHtml) return '';
    return '<div class="resource-bar">' + leftHtml + rightHtml + '</div>';
  }

  function renderResourceEmpty(emptyText, actionHtml, extraHtml) {
    return '<div class="panel"><div class="empty-state stack gap-sm" style="align-items:center;">' +
      '<div class="muted">' + esc(emptyText) + '</div>' +
      (extraHtml || '') +
      (actionHtml || '') +
      '</div></div>';
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
      showViewLoadingIfNeeded(root, 'overview');
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
        if (isCurrent()) {
          root.removeAttribute('data-view-key');
          root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        }
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

    var hasAppsOrRoutes = state.projects.length > 0 || state.routes.length > 0;
    var appsSectionHtml = hasAppsOrRoutes
      ? '<div class="grid-2">' +
        '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('overview.recentProjects')) + '</h3><a href="#/apps" class="btn btn-ghost btn-sm">' + esc(t('common.view')) + '</a></div>' +
        (projRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('common.name')) + '</th><th>' + esc(t('projects.activeDeployment')) + '</th><th>' + esc(t('projects.previewDeployment')) + '</th></tr></thead><tbody>' + projRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('common.empty')) + '</div>') +
        '</div>' +
        '<div class="panel"><div class="panel-header"><h3 class="panel-title">' + esc(t('overview.recentRoutes')) + '</h3><a href="#/apps" class="btn btn-ghost btn-sm">' + esc(t('common.view')) + '</a></div>' +
        (routeRows
          ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('routes.host')) + '</th><th>' + esc(t('routes.project')) + '</th><th>' + esc(t('routes.targetEnv')) + '</th></tr></thead><tbody>' + routeRows + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('common.empty')) + '</div>') +
        '</div></div>'
      : '<div class="panel"><div class="empty-state stack gap-sm" style="align-items:center;">' +
        '<div class="muted">' + esc(t('common.empty')) + '</div>' +
        (isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="overview-empty-new-app-btn">+ ' + esc(t('runtime.newRuntime')) + '</button>' : '') +
        '</div></div>';

    var snap = captureUiSnapshot(root);
    root.innerHTML =
      '<div class="overview-bar">' +
      '<div class="row gap-xs">' +
      healthBadge +
      '<span class="badge badge-neutral">' + esc(st.profile || '-') + '</span>' +
      '</div>' +
      (isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="overview-new-app-btn">+ ' + esc(t('runtime.newRuntime')) + '</button>' : '') +
      '</div>' +

      '<div class="stats-grid stats-grid-4">' +
      statCard(t('overview.projects'), st.projects, '#/apps') +
      statCard(t('overview.routes'), st.routes, '#/apps') +
      statCard(t('overview.dbs'), st.dbs, '#/dbs') +
      statCard(t('overview.kvNamespaces'), st.kv_namespaces, '#/kv') +
      statCard(t('overview.buckets'), st.buckets, '#/blobs') +
      statCard(t('overview.queues'), st.queues, '#/queues') +
      statCard(t('overview.searchIndexes'), st.search_indexes || 0, '#/search') +
      statCard(t('overview.aiProviders'), st.ai_providers || 0, '#/ai') +
      '</div>' +

      '<fp-perf-monitor id="overview-perf-monitor"></fp-perf-monitor>' +

      appsSectionHtml +

      '<details class="minimal-details">' +
      '<summary>' + esc(t('overview.endpointsTitle')) + '</summary>' +
      '<div class="minimal-details-body row gap-xs">' +
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
      '</div></details>';

    if ($('overview-new-app-btn')) $('overview-new-app-btn').addEventListener('click', openCreateRuntimeModal);
    if ($('overview-empty-new-app-btn')) $('overview-empty-new-app-btn').addEventListener('click', openCreateRuntimeModal);

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
    markViewRendered(root, 'overview');
    restoreUiSnapshot(root, snap);
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
        withSubmitLock('create-proj-form', async function () {
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
        withSubmitLock('edit-proj-form', async function () {
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
    var viewKey = 'project:' + id;
    var isCurrentDetail = beginViewRender('apps', function () {
      return state.projectId === id;
    });
    if (fetchData) {
      showViewLoadingIfNeeded(root, viewKey);
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
        if (isCurrentDetail()) {
          root.removeAttribute('data-view-key');
          root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        }
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
          : '<div class="empty-state stack gap-sm" style="align-items:center;">' +
            '<span>' + esc(t('projectDetail.noDeployments')) + '</span>' +
            (writable ? '<button type="button" class="btn btn-primary" id="pd-empty-new-dep">+ ' + esc(t('projectDetail.newDeployment')) + '</button>' : '') +
            '</div>') +
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

    var snap = captureUiSnapshot(root);
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
    if ($('pd-empty-new-dep')) $('pd-empty-new-dep').addEventListener('click', function () { openNewDeploymentModal(p); });
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
    markViewRendered(root, viewKey);
    restoreUiSnapshot(root, snap);
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

    var hasAdv = Boolean(base && (
      base.assets_prefix ||
      base.artifact_path ||
      !base.spa_fallback ||
      (base.crons && base.crons.length) ||
      (base.bindings && Object.keys(base.bindings).length)
    ));

    openModal({
      title: t('projectDetail.newDeployment'),
      body:
        '<form id="new-dep-form" class="stack gap-md">' +
        '<div class="form-grid">' +
        '<div class="field"><label>' + esc(t('projectDetail.targetEnv')) + '</label>' +
        '<select id="dep-env"><option value="prod">' + esc(t('projectDetail.envProd')) + '</option>' +
        '<option value="preview">' + esc(t('projectDetail.envPreview')) + '</option>' +
        '<option value="">' + esc(t('projectDetail.envReady')) + '</option></select></div>' +
        '<div class="field"><label>' + esc(t('projectDetail.assetsBucket')) + '</label><select id="dep-bucket">' + bktOpts + '</select></div>' +
        '</div>' +
        '<div class="field"><label for="dep-file">' + esc(t('projectDetail.artifactFile')) + '</label><input type="file" id="dep-file" accept=".wasm,.wat"><span class="muted text-xs">' + esc(t('projectDetail.artifactFileHint')) + '</span></div>' +
        '<details class="adv-options"' + (hasAdv ? ' open' : '') + '>' +
        '<summary>' + esc(t('common.advancedOptions')) + '</summary>' +
        '<div class="adv-options-body form-grid">' +
        '<div class="field"><label>' + esc(t('projectDetail.assetsPrefix')) + '</label><input type="text" id="dep-prefix" placeholder="dist"></div>' +
        '<div class="field"><label>' + esc(t('projectDetail.artifactPath')) + ' (' + esc(t('common.optional')) + ')</label><input type="text" id="dep-artifact" placeholder="worker.wasm"></div>' +
        '<div class="span-2"><label class="checkbox-row"><input type="checkbox" id="dep-spa" checked> ' + esc(t('projectDetail.spaFallback')) + '</label></div>' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.crons')) + '</label><textarea id="dep-crons" placeholder="' + esc(t('projectDetail.cronsPlaceholder')) + '"></textarea></div>' +
        '<div class="field span-2"><label>' + esc(t('projectDetail.bindings')) + '</label><textarea id="dep-bindings" class="mono" placeholder="' + esc(t('projectDetail.bindingsPlaceholder')) + '"></textarea></div>' +
        '</div></details>' +
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
        withSubmitLock('new-dep-form', async function () {
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
        withSubmitLock('route-form', async function () {
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
        });
      }
    });
  }

  // --- 4-7. Resource View Modules (KV, Queues, Blobs, DBs, Search) ---

  var viewCtx = {
    state: state,
    $: $,
    t: t,
    esc: esc,
    enc: enc,
    encPath: encPath,
    formatBytes: formatBytes,
    formatTime: formatTime,
    decodePayloadBytes: decodePayloadBytes,
    copyText: copyText,
    api: api,
    toast: toast,
    openModal: openModal,
    closeModal: closeModal,
    confirmAction: confirmAction,
    withSubmitLock: withSubmitLock,
    isAdmin: isAdmin,
    isReadonly: isReadonly,
    canWriteResource: canWriteResource,
    projectNameById: projectNameById,
    beginViewRender: beginViewRender,
    showViewLoadingIfNeeded: showViewLoadingIfNeeded,
    markViewRendered: markViewRendered,
    captureUiSnapshot: captureUiSnapshot,
    restoreUiSnapshot: restoreUiSnapshot,
    renderResourceBar: renderResourceBar,
    renderResourceEmpty: renderResourceEmpty,
    renderProjectSelectOptions: renderProjectSelectOptions,
    loadResourceGroupOptions: loadResourceGroupOptions,
    resourceGroupField: resourceGroupField,
    selectedResourceGroup: selectedResourceGroup
  };

  var fpViews = window.__FP_VIEWS || {};
  var kvView = fpViews.kv ? fpViews.kv(viewCtx) : null;
  var queuesView = fpViews.queues ? fpViews.queues(viewCtx) : null;
  var blobsView = fpViews.blobs ? fpViews.blobs(viewCtx) : null;
  var dbsView = fpViews.dbs ? fpViews.dbs(viewCtx) : null;
  var searchView = fpViews.search ? fpViews.search(viewCtx) : null;

  function renderKv(fetchData) { return kvView.renderKv(fetchData); }
  function renderQueues(fetchData) { return queuesView.renderQueues(fetchData); }
  function renderBlobs(fetchData) { return blobsView.renderBlobs(fetchData); }
  function renderDbs(fetchData) { return dbsView.renderDbs(fetchData); }
  function renderSearch(fetchData) { return searchView.renderSearch(fetchData); }

  async function renderRuntime() {
    var root = $('view-root');
    var isCurrent = beginViewRender('apps');
    showViewLoadingIfNeeded(root, 'runtime');
    try {
      await ensureRuntimeComponent();
      if (!isCurrent()) return;
      var res = await Promise.all([api('/_api/v1/apps'), api('/_api/v1/routes')]);
      if (!isCurrent()) return;
      state.projects = res[0] || [];
      state.routes = res[1] || [];
    } catch (e) {
      if (isCurrent()) {
        root.removeAttribute('data-view-key');
        root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      }
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
    markViewRendered(root, 'runtime');
  }

  async function renderOps(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('ops');
    showViewLoadingIfNeeded(root, 'ops');
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
      if (isCurrent()) {
        root.removeAttribute('data-view-key');
        root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      }
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = renderSectionTabs('ops') + '<div class="subnav-content"></div>';
    var ops = document.createElement('fp-ops-center');
    ops.token = state.token || readStoredToken();
    ops.scope = getScope();
    ops.projects = state.projects;
    root.querySelector('.subnav-content').appendChild(ops);
    markViewRendered(root, 'ops');
  }

  async function renderUpdates() {
    var root = $('view-root');
    var isCurrent = beginViewRender('updates');
    showViewLoadingIfNeeded(root, 'updates');
    try {
      await ensureUpdateComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) {
        root.removeAttribute('data-view-key');
        root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      }
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = renderSectionTabs('settings') + '<div class="subnav-content"></div>';
    var upd = document.createElement('fp-system-updates');
    upd.token = state.token || readStoredToken();
    upd.scope = getScope();
    root.querySelector('.subnav-content').appendChild(upd);
    markViewRendered(root, 'updates');
  }

  async function renderAi() {
    var root = $('view-root');
    var isCurrent = beginViewRender('ai');
    showViewLoadingIfNeeded(root, 'ai');
    try {
      await ensureAiComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) {
        root.removeAttribute('data-view-key');
        root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      }
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = '';
    var panel = document.createElement('fp-ai-gateway');
    panel.token = state.token || readStoredToken();
    panel.scope = getScope();
    root.appendChild(panel);
    markViewRendered(root, 'ai');
  }

  async function renderSettings() {
    var root = $('view-root');
    var isCurrent = beginViewRender('settings');
    showViewLoadingIfNeeded(root, 'settings');
    try {
      await ensureSettingsComponent();
      if (!isCurrent()) return;
    } catch (e) {
      if (isCurrent()) {
        root.removeAttribute('data-view-key');
        root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
      }
      return;
    }
    if (!isCurrent()) return;
    root.innerHTML = renderSectionTabs('settings') + '<div class="subnav-content"></div>';
    var panel = document.createElement('fp-platform-settings');
    panel.token = state.token || readStoredToken();
    panel.scope = getScope();
    root.querySelector('.subnav-content').appendChild(panel);
    markViewRendered(root, 'settings');
  }

  // --- 8. Tokens (Admin Only) ---

  function buildTokensTableHtml() {
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
    return rows
      ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('common.name')) + '</th><th>' + esc(t('tokens.scope')) + '</th><th>' + esc(t('common.expiresAt')) + '</th><th>' + esc(t('tokens.lastUsed')) + '</th><th>' + esc(t('common.createdAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + rows + '</tbody></table></div>'
      : '<div class="empty-state">' + esc(t('tokens.empty')) + '</div>';
  }

  function bindTokensTableEvents(scopeEl) {
    if (!scopeEl) return;
    scopeEl.querySelectorAll('[data-revoke-tok]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        var id = btn.getAttribute('data-revoke-tok');
        var name = btn.getAttribute('data-tok-name');
        confirmAction(t('tokens.revokeConfirm', { name: name }), async function () {
          try {
            await api('/_api/v1/tokens/' + enc(id), { method: 'DELETE' });
            state.tokens = state.tokens.filter(function (tok) { return tok.id !== id; });
            toast(t('tokens.revoked'), 'success');
            renderTokensTablePartial();
          } catch (e) { toast(e.message, 'error'); }
        });
      });
    });
  }

  function renderTokensTablePartial() {
    var slot = $('tokens-table-slot');
    if (!slot) return renderTokens(false);
    slot.innerHTML = buildTokensTableHtml();
    bindTokensTableEvents(slot);
  }

  async function renderTokens(fetchData) {
    var root = $('view-root');
    var isCurrent = beginViewRender('tokens');
    if (fetchData) {
      showViewLoadingIfNeeded(root, 'tokens');
      try {
        var res = await Promise.all([api('/_api/v1/tokens'), api('/_api/v1/apps')]);
        if (!isCurrent()) return;
        state.tokens = res[0] || [];
        state.projects = res[1] || [];
      } catch (e) {
        if (isCurrent()) {
          root.removeAttribute('data-view-key');
          root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        }
        return;
      }
    }
    if (!isCurrent()) return;

    var snap = captureUiSnapshot(root);
    root.innerHTML =
      renderSectionTabs(
        'settings',
        '<button type="button" class="btn btn-primary btn-sm" id="new-token-btn">+ ' + esc(t('tokens.new')) + '</button>'
      ) +
      '<div id="tokens-table-slot" class="panel">' +
      buildTokensTableHtml() +
      '</div>';

    $('new-token-btn').addEventListener('click', openCreateTokenModal);
    bindTokensTableEvents($('tokens-table-slot'));
    markViewRendered(root, 'tokens');
    restoreUiSnapshot(root, snap);
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
        withSubmitLock('create-tok-form', async function () {
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
            state.tokens = (await api('/_api/v1/tokens')) || [];
            renderTokensTablePartial();
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
      renderSectionTabs('ops') +
      tabsHtml + bodyHtml;
    markViewRendered(root, 'logs');

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
    if ($('open-drawer-btn')) $('open-drawer-btn').addEventListener('click', toggleSidebar);
    if ($('mobile-more-btn')) $('mobile-more-btn').addEventListener('click', openDrawer);
    if ($('close-drawer-btn')) $('close-drawer-btn').addEventListener('click', closeDrawer);
    if ($('drawer-backdrop')) $('drawer-backdrop').addEventListener('click', closeDrawer);
    if ($('disconnect-btn')) $('disconnect-btn').addEventListener('click', function () { disconnect(false); });

    document.addEventListener('keydown', function (ev) {
      if (ev.key === 'Escape') {
        closeModal();
        closeDrawer();
      } else if ((ev.ctrlKey || ev.metaKey) && !ev.shiftKey && !ev.altKey && (ev.key === 'b' || ev.key === 'B')) {
        var tag = ev.target && ev.target.tagName;
        if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || (ev.target && ev.target.isContentEditable)) return;
        if (state.status) {
          ev.preventDefault();
          toggleSidebar();
        }
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

    document.addEventListener('fp-confirm', function (ev) {
      var d = ev.detail || {};
      if (d.message && typeof d.onConfirm === 'function') {
        ev.preventDefault();
        confirmAction(d.message, d.onConfirm, {
          title: d.title,
          confirmLabel: d.confirmLabel,
          danger: d.danger !== false
        });
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
    try {
      if (localStorage.getItem(SIDEBAR_STORAGE) === '1') {
        applySidebarCollapsed(true);
      }
    } catch (_) {}

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
