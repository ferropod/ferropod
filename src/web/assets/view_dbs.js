(function () {
  'use strict';

  var views = (window.__FP_VIEWS = window.__FP_VIEWS || {});
  views.dbs = function (ctx) {
    var state = ctx.state;
    var $ = ctx.$;
    var t = ctx.t;
    var esc = ctx.esc;
    var enc = ctx.enc;
    var api = ctx.api;
    var toast = ctx.toast;
    var openModal = ctx.openModal;
    var closeModal = ctx.closeModal;
    var confirmAction = ctx.confirmAction;
    var withSubmitLock = ctx.withSubmitLock;
    var isAdmin = ctx.isAdmin;
    var canWriteResource = ctx.canWriteResource;
    var projectNameById = ctx.projectNameById;
    var beginViewRender = ctx.beginViewRender;
    var showViewLoadingIfNeeded = ctx.showViewLoadingIfNeeded;
    var markViewRendered = ctx.markViewRendered;
    var captureUiSnapshot = ctx.captureUiSnapshot;
    var restoreUiSnapshot = ctx.restoreUiSnapshot;
    var renderResourceBar = ctx.renderResourceBar;
    var renderResourceEmpty = ctx.renderResourceEmpty;
    var renderProjectSelectOptions = ctx.renderProjectSelectOptions;
    var loadResourceGroupOptions = ctx.loadResourceGroupOptions;
    var resourceGroupField = ctx.resourceGroupField;
    var selectedResourceGroup = ctx.selectedResourceGroup;
    var dbFtsRequestSeq = 0;

    // --- 7. Databases (Turso User DBs) ---

    function buildDbMetaHtml(dbMeta) {
      if (!dbMeta) return '';
      return esc(dbMeta.id) +
        (dbMeta.read_only ? ' · Read-Only' : '') +
        (dbMeta.max_rows ? ' · Max Rows: ' + esc(dbMeta.max_rows) : '') +
        (dbMeta.max_size_mb ? ' · Max Size: ' + esc(dbMeta.max_size_mb) + ' MiB' : '') +
        (dbMeta.query_timeout_ms ? ' · Timeout: ' + esc(dbMeta.query_timeout_ms) + 'ms' : '') +
        (dbMeta.rate_limit_rpm ? ' · ' + esc(dbMeta.rate_limit_rpm) + ' RPM' : '') +
        (dbMeta.description ? ' · ' + esc(dbMeta.description) : '');
    }

    function buildDbResultHtml() {
      if (!state.dbResult) return '';
      if (state.dbResult.type === 'execute') {
        return '<div class="alert alert-info">' + esc(t('dbs.rowsAffected', { count: state.dbResult.rows_affected })) + '</div>';
      }
      var cols = state.dbResult.columns || [];
      var rows = state.dbResult.rows || [];
      var thead = '<tr>' + cols.map(function (c) { return '<th>' + esc(c) + '</th>'; }).join('') + '</tr>';
      var tbody = rows.map(function (r) {
        return '<tr>' + r.map(function (cell) {
          var txt = cell === null ? 'NULL' : (typeof cell === 'object' ? JSON.stringify(cell) : String(cell));
          return '<td class="mono text-xs">' + esc(txt) + '</td>';
        }).join('') + '</tr>';
      }).join('');
      return '<div class="muted text-xs">' + esc(t('dbs.rowsReturned', { count: rows.length })) + '</div>' +
        (rows.length
          ? '<div class="table-wrap"><table class="data-table"><thead>' + thead + '</thead><tbody>' + tbody + '</tbody></table></div>'
          : '<div class="empty-state">' + esc(t('dbs.noRows')) + '</div>');
    }

    function renderDbResultPartial() {
      var slot = $('db-result-slot');
      if (!slot) return renderDbs(false);
      slot.innerHTML = buildDbResultHtml();
    }

    function renderDbSchemaPartial() {
      var slot = $('db-schema-slot');
      if (!slot) return renderDbs(false);
      slot.innerHTML = state.dbSchema
        ? '<pre class="code-block">' + esc(state.dbSchema) + '</pre>'
        : '<div class="empty-state">' + esc(t('dbs.emptySchema')) + '</div>';
    }

    function updateDbInPlace(updatedDb) {
      if (!updatedDb) return;
      var idx = state.dbs.findIndex(function (d) { return d.id === updatedDb.id; });
      if (idx >= 0) state.dbs[idx] = updatedDb;
      if (state.selectedDbId === updatedDb.id) {
        var metaSlot = $('db-header-meta');
        if (metaSlot) metaSlot.innerHTML = buildDbMetaHtml(updatedDb);
      }
      var sel = $('db-switcher');
      if (sel) {
        for (var i = 0; i < sel.options.length; i++) {
          if (sel.options[i].value === updatedDb.id) {
            sel.options[i].textContent = updatedDb.name + (updatedDb.project_id ? ' · ' + projectNameById(updatedDb.project_id) : '');
            break;
          }
        }
      }
    }

    async function renderDbs(fetchData) {
      var root = $('view-root');
      var isCurrent = beginViewRender('dbs');
      if (fetchData) {
        showViewLoadingIfNeeded(root, 'dbs');
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
          if (isCurrent()) {
            root.removeAttribute('data-view-key');
            root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
          }
          return;
        }
      }
      if (!isCurrent()) return;

      var dbMeta = state.dbs.find(function (d) { return d.id === state.selectedDbId; });
      var canWriteDb = dbMeta && canWriteResource(dbMeta.project_id);
      var createDbBtnHtml = isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="new-db-btn">+ ' + esc(t('dbs.newDb')) + '</button>' : '';

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
            '<div id="db-schema-slot" class="panel-body">' +
            (state.dbSchema
              ? '<pre class="code-block">' + esc(state.dbSchema) + '</pre>'
              : '<div class="empty-state">' + esc(t('dbs.emptySchema')) + '</div>') +
            '</div></div>';
        } else {
          bodyHtml =
            '<div class="panel"><div class="panel-header"><div class="toolbar">' +
            '<div><h3 class="panel-title mono">' + esc(dbMeta.name) + '</h3><div id="db-header-meta" class="muted text-xs mono">' +
            buildDbMetaHtml(dbMeta) +
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
            '<div id="db-result-slot" class="stack gap-sm">' + buildDbResultHtml() + '</div>' +
            '</div></div>';
        }
        rightHtml = '<div class="stack gap-md">' + tabsHtml + bodyHtml + '</div>';
      }

      var snap = captureUiSnapshot(root);
      if (state.dbs.length === 0) {
        root.innerHTML = renderResourceEmpty(t('dbs.noDbs'), createDbBtnHtml);
      } else {
        root.innerHTML =
          renderResourceBar({
            selectId: 'db-switcher',
            items: state.dbs,
            selectedValue: state.selectedDbId,
            valueFn: function (d) { return d.id; },
            labelFn: function (d) { return d.name + (d.project_id ? ' · ' + projectNameById(d.project_id) : ''); },
            actionHtml: createDbBtnHtml
          }) +
          rightHtml;
      }

      if ($('new-db-btn')) $('new-db-btn').addEventListener('click', openCreateDbModal);
      if ($('db-switcher')) {
        $('db-switcher').addEventListener('change', async function (ev) {
          state.selectedDbId = ev.target.value;
          state.dbResult = null;
          if (state.dbTab === 'schema') {
            try { await loadDbSchema(); } catch (e) { toast(e.message, 'error'); }
          }
          renderDbs(false);
        });
      }
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
            renderDbSchemaPartial();
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
      if ($('db-edit-btn') && dbMeta) {
        $('db-edit-btn').addEventListener('click', function () {
          var latest = state.dbs.find(function (d) { return d.id === state.selectedDbId; }) || dbMeta;
          openEditDbModal(latest);
        });
      }
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
      markViewRendered(root, 'dbs');
      restoreUiSnapshot(root, snap);
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
        panel.innerHTML =
          '<div class="panel-header"><div><h3 class="panel-title">' + esc(t('dbs.tabFts')) + '</h3>' +
          '<div class="muted text-xs">' + esc(t('dbs.ftsSubtitle')) + '</div></div></div>' +
          '<div class="panel-body stack gap-md">' +
          (canWrite
            ? '<form id="fts-create-form" class="form-grid">' +
              '<div class="field"><label for="fts-name">' + esc(t('dbs.ftsIndexName')) + '</label><input type="text" id="fts-name" required placeholder="' + esc(t('dbs.ftsIndexName')) + '"></div>' +
              '<div class="field"><label for="fts-table">' + esc(t('dbs.ftsTable')) + '</label><input type="text" id="fts-table" required placeholder="' + esc(t('dbs.ftsTable')) + '"></div>' +
              '<div class="field"><label for="fts-columns">' + esc(t('dbs.ftsColumns')) + '</label><input type="text" id="fts-columns" required placeholder="' + esc(t('dbs.ftsColumns')) + '"></div>' +
              '<div class="field"><label for="fts-tokenizer">' + esc(t('dbs.ftsTokenizer')) + '</label><select id="fts-tokenizer">' +
              '<option value="unicode61">unicode61 (Default)</option>' +
              '<option value="cjk_jieba">cjk_jieba (CJK / Chinese)</option>' +
              '<option value="ngram3">ngram3 (Substring)</option>' +
              '<option value="raw">raw (Exact)</option>' +
              '</select></div>' +
              '<div class="span-2 row"><button type="submit" class="btn btn-primary">' + esc(t('dbs.ftsNewIndex')) + '</button></div>' +
              '</form>'
            : '') +
          (indexes.length
            ? indexes.map(function (idx) {
                return '<div class="panel"><div class="panel-header"><div class="row"><strong class="mono">' + esc(idx.name) + '</strong>' +
                  '<span class="badge badge-neutral mono">' + esc(idx.table + ' (' + idx.columns.join(', ') + ')') + '</span>' +
                  (idx.tokenizer ? '<span class="badge badge-info mono">' + esc(idx.tokenizer) + '</span>' : '') +
                  '</div>' +
                  (canWrite ? '<button type="button" class="btn btn-danger btn-sm" data-fts-delete="' + esc(idx.name) + '">' + esc(t('common.delete')) + '</button>' : '') +
                  '</div>' +
                  '<div class="panel-body stack gap-sm">' +
                  '<form class="input-group" data-fts-query="' + esc(idx.name) + '"><input type="search" name="query" required placeholder="' +
                  esc(t('dbs.ftsTestPlaceholder')) + '"><button class="btn btn-primary" type="submit">' + esc(t('common.search')) + '</button></form>' +
                  '<pre class="code-block hidden" data-fts-result="' + esc(idx.name) + '"></pre>' +
                  '</div></div>';
              }).join('')
            : '<div class="empty-state">' + esc(t('dbs.ftsNoIndexes')) + '</div>') +
          '</div>';
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
        renderDbResultPartial();
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
        renderDbResultPartial();
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
          '<div class="form-grid">' +
          '<div class="field"><label>' + esc(t('dbs.queryTimeoutMs')) + '</label><input type="number" id="cdb-timeout" min="100" placeholder="15000"></div>' +
          '<div class="field"><label>' + esc(t('dbs.rateLimitRpm')) + '</label><input type="number" id="cdb-rpm" min="1" placeholder="600"></div>' +
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
          withSubmitLock('create-db-form', async function () {
            var maxRowsRaw = $('cdb-max-rows').value.trim();
            var maxSizeRaw = $('cdb-max-size').value.trim();
            var timeoutRaw = $('cdb-timeout').value.trim();
            var rpmRaw = $('cdb-rpm').value.trim();
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
                  read_only: $('cdb-ro').checked,
                  query_timeout_ms: timeoutRaw ? Number(timeoutRaw) : null,
                  rate_limit_rpm: rpmRaw ? Number(rpmRaw) : null
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
          '<div class="form-grid">' +
          '<div class="field"><label>' + esc(t('dbs.queryTimeoutMs')) + '</label><input type="number" id="edb-timeout" min="0" value="' + esc(dbMeta.query_timeout_ms || '') + '" placeholder="15000"></div>' +
          '<div class="field"><label>' + esc(t('dbs.rateLimitRpm')) + '</label><input type="number" id="edb-rpm" min="0" value="' + esc(dbMeta.rate_limit_rpm || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
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
          withSubmitLock('edit-db-form', async function () {
            var maxRowsRaw = $('edb-max-rows').value.trim();
            var maxSizeRaw = $('edb-max-size').value.trim();
            var timeoutRaw = $('edb-timeout').value.trim();
            var rpmRaw = $('edb-rpm').value.trim();
            try {
              var updated = await api('/_api/v1/dbs/' + enc(dbMeta.id), {
                method: 'PATCH',
                json: {
                  group: $('resource-group').value.trim(),
                  description: $('edb-desc').value.trim(),
                  max_rows: maxRowsRaw ? Number(maxRowsRaw) : 0,
                  max_size_mb: maxSizeRaw ? Number(maxSizeRaw) : 0,
                  foreign_keys: $('edb-fk').checked,
                  read_only: $('edb-ro').checked,
                  query_timeout_ms: timeoutRaw ? Number(timeoutRaw) : 0,
                  rate_limit_rpm: rpmRaw ? Number(rpmRaw) : 0
                }
              });
              closeModal();
              toast(t('dbs.updated'), 'success');
              updateDbInPlace(updated);
            } catch (e) { toast(e.message, 'error'); }
          });
        }
      });
    }

    return { renderDbs: renderDbs };
  };
})();
