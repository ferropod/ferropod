(function () {
  'use strict';

  var views = (window.__FP_VIEWS = window.__FP_VIEWS || {});
  views.kv = function (ctx) {
    var state = ctx.state;
    var $ = ctx.$;
    var t = ctx.t;
    var esc = ctx.esc;
    var enc = ctx.enc;
    var formatBytes = ctx.formatBytes;
    var formatTime = ctx.formatTime;
    var api = ctx.api;
    var toast = ctx.toast;
    var openModal = ctx.openModal;
    var closeModal = ctx.closeModal;
    var confirmAction = ctx.confirmAction;
    var withSubmitLock = ctx.withSubmitLock;
    var isAdmin = ctx.isAdmin;
    var isReadonly = ctx.isReadonly;
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

    // --- 4. KV Store ---

    function buildKvMetaHtml(nsMeta) {
      if (!nsMeta) return '';
      return 'TTL: ' + esc(nsMeta.default_ttl_secs || 604800) + 's' +
        (nsMeta.max_keys ? ' · Max Keys: ' + esc(nsMeta.max_keys) : '') +
        (nsMeta.max_value_bytes ? ' · Max Value: ' + esc(formatBytes(nsMeta.max_value_bytes)) : '') +
        (nsMeta.rate_limit_rpm ? ' · ' + esc(nsMeta.rate_limit_rpm) + ' RPM' : '') +
        (nsMeta.description ? ' · ' + esc(nsMeta.description) : '');
    }

    function buildKvTableHtml(canWriteNs) {
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
      return keyRows
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('kv.key')) + '</th><th>' + esc(t('kv.size')) + '</th><th>' + esc(t('common.expiresAt')) + '</th><th>' + esc(t('common.updatedAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + keyRows + '</tbody></table></div>'
        : '<div class="empty-state">' + esc(t('kv.noKeys')) + '</div>';
    }

    function bindKvTableEvents(scopeEl, canWriteNs) {
      if (!scopeEl) return;
      scopeEl.querySelectorAll('[data-view-kv]').forEach(function (btn) {
        btn.addEventListener('click', async function () {
          var key = btn.getAttribute('data-view-kv');
          try {
            var item = await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key));
            openKvValueModal(key, item, canWriteNs);
          } catch (e) { toast(e.message, 'error'); }
        });
      });
      scopeEl.querySelectorAll('[data-incr-kv]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          openKvIncrModal(btn.getAttribute('data-incr-kv'));
        });
      });
      scopeEl.querySelectorAll('[data-del-kv]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          var key = btn.getAttribute('data-del-kv');
          confirmAction(t('kv.deleteConfirm', { key: key }), async function () {
            try {
              await api('/_api/v1/kv/' + enc(state.selectedKvNs) + '/' + enc(key), { method: 'DELETE' });
              toast(t('kv.deleted'), 'success');
              await loadKvKeys();
              renderKvTablePartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      });
    }

    function renderKvTablePartial() {
      var slot = $('kv-table-slot');
      if (!slot) return renderKv(false);
      var nsMeta = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; });
      var canWriteNs = !isReadonly() && (isAdmin() || (nsMeta && canWriteResource(nsMeta.project_id)));
      slot.innerHTML = buildKvTableHtml(canWriteNs);
      bindKvTableEvents(slot, canWriteNs);
    }

    function updateKvNamespaceInPlace(updatedNs) {
      if (!updatedNs) return;
      var idx = state.kvNamespaces.findIndex(function (n) { return n.name === updatedNs.name; });
      if (idx >= 0) state.kvNamespaces[idx] = updatedNs;
      if (state.selectedKvNs === updatedNs.name) {
        var metaSlot = $('kv-meta-slot');
        if (metaSlot) metaSlot.innerHTML = buildKvMetaHtml(updatedNs);
      }
      var sel = $('kv-switcher');
      if (sel) {
        for (var i = 0; i < sel.options.length; i++) {
          if (sel.options[i].value === updatedNs.name) {
            sel.options[i].textContent = updatedNs.name + (updatedNs.project_id ? ' · ' + projectNameById(updatedNs.project_id) : '');
            break;
          }
        }
      }
    }

    async function renderKv(fetchData) {
      var root = $('view-root');
      var isCurrent = beginViewRender('kv');
      if (fetchData) {
        showViewLoadingIfNeeded(root, 'kv');
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
          if (isCurrent()) {
            root.removeAttribute('data-view-key');
            root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
          }
          return;
        }
      }
      if (!isCurrent()) return;

      var nsMeta = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; });
      var canWriteNs = !isReadonly() && (isAdmin() || (nsMeta && canWriteResource(nsMeta.project_id)));
      var createKvBtnHtml = isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="new-kv-ns-btn">+ ' + esc(t('kv.newNamespace')) + '</button>' : '';
      var manualFormHtml = '<form id="kv-manual-form" class="resource-inline-form">' +
        '<input type="text" id="kv-manual-ns" placeholder="' + esc(t('kv.manualNamespace')) + '">' +
        '<button type="submit" class="btn btn-sm">' + esc(t('kv.openNamespace')) + '</button></form>';

      var rightHtml = !state.selectedKvNs
        ? '<div class="panel"><div class="empty-state">' + esc(t('kv.selectNamespace')) + '</div></div>'
        : '<div class="panel">' +
          '<div class="panel-header"><div class="toolbar">' +
          '<div><h3 class="panel-title mono">' + esc(state.selectedKvNs) + '</h3>' +
          '<div id="kv-meta-slot" class="muted text-xs">' + buildKvMetaHtml(nsMeta) + '</div>' +
          '</div>' +
          '<div class="row">' +
          '<input type="search" id="kv-prefix-input" style="width:190px;" value="' + esc(state.kvPrefix) + '" placeholder="' + esc(t('kv.prefixPlaceholder')) + '">' +
          '<button type="button" class="btn btn-sm" id="kv-filter-btn">' + esc(t('common.filter')) + '</button>' +
          (canWriteNs ? '<button type="button" class="btn btn-primary btn-sm" id="kv-put-btn">' + esc(t('kv.putKey')) + '</button>' : '') +
          (canWriteNs && nsMeta ? '<button type="button" class="btn btn-sm" id="kv-edit-ns-btn">' + esc(t('common.editConfig')) + '</button>' : '') +
          (canWriteNs && nsMeta ? '<button type="button" class="btn btn-danger btn-sm" id="kv-del-ns-btn">' + esc(t('kv.deleteNamespace')) + '</button>' : '') +
          '</div></div></div>' +
          '<div id="kv-table-slot">' + buildKvTableHtml(canWriteNs) + '</div>' +
          '</div>';

      var snap = captureUiSnapshot(root);
      if (state.kvNamespaces.length === 0 && !state.selectedKvNs) {
        root.innerHTML = renderResourceEmpty(t('kv.noNamespaces'), createKvBtnHtml, manualFormHtml);
      } else {
        var extraOpt = (state.selectedKvNs && !nsMeta)
          ? { value: state.selectedKvNs, label: state.selectedKvNs }
          : null;
        root.innerHTML =
          renderResourceBar({
            selectId: 'kv-switcher',
            items: state.kvNamespaces,
            selectedValue: state.selectedKvNs,
            valueFn: function (n) { return n.name; },
            labelFn: function (n) { return n.name + (n.project_id ? ' · ' + projectNameById(n.project_id) : ''); },
            extraOption: extraOpt,
            extraLeftHtml: manualFormHtml,
            actionHtml: createKvBtnHtml
          }) +
          rightHtml;
      }

      if ($('new-kv-ns-btn')) $('new-kv-ns-btn').addEventListener('click', openCreateKvNamespaceModal);
      if ($('kv-manual-form')) {
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
      }
      if ($('kv-switcher')) {
        $('kv-switcher').addEventListener('change', async function (ev) {
          state.selectedKvNs = ev.target.value;
          try {
            await loadKvKeys();
            renderKv(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
      if ($('kv-filter-btn')) {
        var runKvFilter = async function () {
          state.kvPrefix = $('kv-prefix-input').value.trim();
          try {
            await loadKvKeys();
            renderKvTablePartial();
          } catch (e) { toast(e.message, 'error'); }
        };
        $('kv-filter-btn').addEventListener('click', runKvFilter);
        if ($('kv-prefix-input')) {
          $('kv-prefix-input').addEventListener('keydown', function (ev) {
            if (ev.key === 'Enter') {
              ev.preventDefault();
              runKvFilter();
            }
          });
          $('kv-prefix-input').addEventListener('search', runKvFilter);
        }
      }
      if ($('kv-put-btn')) {
        $('kv-put-btn').addEventListener('click', function () {
          openKvPutModal(null);
        });
      }
      if ($('kv-edit-ns-btn') && nsMeta) {
        $('kv-edit-ns-btn').addEventListener('click', function () {
          var latest = state.kvNamespaces.find(function (n) { return n.name === state.selectedKvNs; }) || nsMeta;
          openEditKvNamespaceModal(latest);
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
      bindKvTableEvents($('kv-table-slot'), canWriteNs);
      markViewRendered(root, 'kv');
      restoreUiSnapshot(root, snap);
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
          '<div class="field"><label>' + esc(t('kv.rateLimitRpm')) + '</label><input type="number" id="kns-rpm" min="1" placeholder="1000"></div>' +
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
          withSubmitLock('kv-ns-form', async function () {
            var name = $('kns-name').value.trim();
            var pid = $('kns-proj').value || null;
            var ttl = Number($('kns-ttl').value || 604800);
            var desc = $('kns-desc').value.trim();
            var maxValRaw = $('kns-max-val').value.trim();
            var maxKeysRaw = $('kns-max-keys').value.trim();
            var rpmRaw = $('kns-rpm').value.trim();
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
                  max_keys: maxKeysRaw ? Number(maxKeysRaw) : null,
                  rate_limit_rpm: rpmRaw ? Number(rpmRaw) : null
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
          '<div class="field"><label>' + esc(t('kv.rateLimitRpm')) + '</label><input type="number" id="ekns-rpm" min="0" value="' + esc(nsMeta.rate_limit_rpm || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
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
          withSubmitLock('edit-kv-ns-form', async function () {
            var maxValRaw = $('ekns-max-val').value.trim();
            var maxKeysRaw = $('ekns-max-keys').value.trim();
            var rpmRaw = $('ekns-rpm').value.trim();
            try {
              var updated = await api('/_api/v1/kv/' + enc(nsMeta.name), {
                method: 'PATCH',
                json: {
                  group: $('resource-group').value.trim(),
                  description: $('ekns-desc').value.trim(),
                  default_ttl_secs: Number($('ekns-ttl').value || 604800),
                  default_ephemeral: $('ekns-eph').checked,
                  max_value_bytes: maxValRaw ? Number(maxValRaw) : 0,
                  max_keys: maxKeysRaw ? Number(maxKeysRaw) : 0,
                  rate_limit_rpm: rpmRaw ? Number(rpmRaw) : 0
                }
              });
              closeModal();
              toast(t('kv.namespaceUpdated'), 'success');
              updateKvNamespaceInPlace(updated);
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
          withSubmitLock('kv-put-form', async function () {
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
              renderKvTablePartial();
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
          withSubmitLock('kv-incr-form', async function () {
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
              renderKvTablePartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        }
      });
    }

    return { renderKv: renderKv };
  };
})();
