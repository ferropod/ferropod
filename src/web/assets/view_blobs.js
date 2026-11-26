(function () {
  'use strict';

  var views = (window.__FP_VIEWS = window.__FP_VIEWS || {});
  views.blobs = function (ctx) {
    var state = ctx.state;
    var $ = ctx.$;
    var t = ctx.t;
    var esc = ctx.esc;
    var enc = ctx.enc;
    var encPath = ctx.encPath;
    var formatBytes = ctx.formatBytes;
    var formatTime = ctx.formatTime;
    var copyText = ctx.copyText;
    var api = ctx.api;
    var toast = ctx.toast;
    var openModal = ctx.openModal;
    var closeModal = ctx.closeModal;
    var confirmAction = ctx.confirmAction;
    var withSubmitLock = ctx.withSubmitLock;
    var isAdmin = ctx.isAdmin;
    var canWriteResource = ctx.canWriteResource;
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

    // --- 6. Blobs / Object Storage ---

    function resetBlobBrowserState() {
      state.blobDir = '';
      state.blobSearch = '';
      state.blobFolders = [];
      state.blobObjects = [];
      state.blobTruncated = false;
    }

    function parentBlobDir(dir) {
      if (!dir) return '';
      var trimmed = dir.replace(/\/+$/, '');
      var idx = trimmed.lastIndexOf('/');
      return idx >= 0 ? trimmed.slice(0, idx + 1) : '';
    }

    async function navigateBlobDir(nextDir, clearSearch) {
      state.blobDir = nextDir || '';
      if (clearSearch) state.blobSearch = '';
      try {
        await loadBucketObjects();
        renderBlobContentPartial();
      } catch (e) { toast(e.message, 'error'); }
    }

    function buildBlobBadgeHtml(bCfg) {
      return bCfg.public_read
        ? '<span class="badge badge-success">' + esc(t('blobs.public')) + '</span>'
        : '<span class="badge badge-neutral">' + esc(t('blobs.private')) + '</span>';
    }

    function buildBlobParts(bCfg, canWriteBkt) {
      var folderIconSvg = '<svg class="blob-icon blob-icon-folder" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>';
      var upIconSvg = '<svg class="blob-icon blob-icon-folder" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5v0a5.5 5.5 0 0 1-5.5 5.5H11"/></svg>';
      var fileIconSvg = '<svg class="blob-icon blob-icon-file" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/></svg>';

      var segs = state.blobDir ? state.blobDir.replace(/\/+$/, '').split('/').filter(Boolean) : [];
      var crumbAcc = '';
      var crumbsHtml =
        '<button type="button" class="blob-crumb mono' + (segs.length === 0 ? ' blob-crumb-current' : '') + '" data-nav-dir="">' + esc(bCfg.name) + '</button>' +
        segs.map(function (seg, idx) {
          crumbAcc += seg + '/';
          var isLast = idx === segs.length - 1;
          return '<span class="blob-crumb-sep">/</span>' +
            '<button type="button" class="blob-crumb mono' + (isLast ? ' blob-crumb-current' : '') + '" data-nav-dir="' + esc(crumbAcc) + '">' + esc(seg) + '</button>';
        }).join('');

      var pathbarLeftHtml =
        '<button type="button" class="btn btn-sm" id="blob-up-btn"' + (state.blobDir ? '' : ' disabled') + ' title="' + esc(t('blobs.upLevel')) + '">↑</button>' +
        '<div class="blob-breadcrumbs">' + crumbsHtml + '</div>' +
        '<span class="blob-count-summary">' + esc(t('blobs.dirSummary', { folders: state.blobFolders.length, files: state.blobObjects.length })) + '</span>';

      var parentDir = parentBlobDir(state.blobDir);
      var parentRowHtml = (state.blobDir && !state.blobSearch)
        ? '<tr class="blob-parent-row">' +
          '<td data-label="' + esc(t('blobs.name')) + '"><div class="blob-name-cell">' + upIconSvg +
          '<button type="button" class="blob-folder-link mono" data-nav-dir="' + esc(parentDir) + '">..</button></div></td>' +
          '<td data-label="' + esc(t('kv.size')) + '" class="muted">-</td>' +
          '<td data-label="' + esc(t('blobs.contentType')) + '" class="text-xs muted">' + esc(t('blobs.parentDir')) + '</td>' +
          '<td data-label="' + esc(t('common.updatedAt')) + '" class="text-xs muted">-</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          '<button type="button" class="btn btn-sm" data-nav-dir="' + esc(parentDir) + '">' + esc(t('blobs.upLevel')) + '</button>' +
          '</div></td></tr>'
        : '';

      var folderRows = state.blobFolders.map(function (f) {
        return '<tr>' +
          '<td data-label="' + esc(t('blobs.name')) + '"><div class="blob-name-cell">' + folderIconSvg +
          '<button type="button" class="blob-folder-link mono text-strong break-all" data-nav-dir="' + esc(f.prefix) + '">' + esc(f.name) + '/</button></div></td>' +
          '<td data-label="' + esc(t('kv.size')) + '">' + esc(formatBytes(f.total_size)) + '</td>' +
          '<td data-label="' + esc(t('blobs.contentType')) + '" class="text-xs muted">' + esc(t('blobs.folderMeta', { count: f.object_count })) + '</td>' +
          '<td data-label="' + esc(t('common.updatedAt')) + '" class="text-xs muted">' + esc(formatTime(f.updated_at)) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          '<button type="button" class="btn btn-sm" data-nav-dir="' + esc(f.prefix) + '">' + esc(t('blobs.openFolder')) + '</button>' +
          (canWriteBkt ? '<button type="button" class="btn btn-danger btn-sm" data-del-folder="' + esc(f.prefix) + '" data-folder-count="' + esc(f.object_count) + '">' + esc(t('common.delete')) + '</button>' : '') +
          '</div></td></tr>';
      }).join('');

      var objRows = state.blobObjects.map(function (o) {
        var displayName = state.blobDir && o.key.indexOf(state.blobDir) === 0
          ? o.key.slice(state.blobDir.length)
          : o.key;
        return '<tr>' +
          '<td data-label="' + esc(t('blobs.name')) + '"><div class="blob-name-cell">' + fileIconSvg +
          '<span class="mono text-strong break-all" title="' + esc(o.key) + '">' + esc(displayName) + '</span></div></td>' +
          '<td data-label="' + esc(t('kv.size')) + '">' + esc(formatBytes(o.size)) + '</td>' +
          '<td data-label="' + esc(t('blobs.contentType')) + '" class="text-xs muted">' + esc(o.content_type) + '</td>' +
          '<td data-label="' + esc(t('common.updatedAt')) + '" class="text-xs muted">' + esc(formatTime(o.updated_at)) + '</td>' +
          '<td class="cell-actions-td"><div class="cell-actions">' +
          '<button type="button" class="btn btn-sm" data-dl-obj="' + esc(o.key) + '">' + esc(t('common.download')) + '</button>' +
          '<button type="button" class="btn btn-sm" data-presign-obj="' + esc(o.key) + '">' + esc(t('blobs.presign')) + '</button>' +
          (canWriteBkt ? '<button type="button" class="btn btn-danger btn-sm" data-del-obj="' + esc(o.key) + '">' + esc(t('common.delete')) + '</button>' : '') +
          '</div></td></tr>';
      }).join('');

      var pathDisplay = '/' + (state.blobDir || '');
      var searchBannerHtml = state.blobSearch
        ? '<div class="blob-search-banner">' +
          '<span>' + esc(t('blobs.searchingIn', { query: state.blobSearch, path: pathDisplay })) + '</span>' +
          '<button type="button" class="btn btn-sm" id="blob-clear-search-btn">' + esc(t('blobs.clearSearch')) + '</button>' +
          '</div>'
        : '';

      var truncatedBannerHtml = state.blobTruncated
        ? '<div class="alert alert-warn" style="margin:0;border-radius:0;border-left:none;border-right:none;border-top:none;">' +
          esc(t('blobs.truncatedNotice')) + '</div>'
        : '';

      var hasEntries = Boolean(folderRows || objRows);
      var tableHtml = (parentRowHtml || hasEntries)
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('blobs.name')) + '</th><th>' + esc(t('kv.size')) + '</th><th>' + esc(t('blobs.contentType')) + '</th><th>' + esc(t('common.updatedAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + parentRowHtml + folderRows + objRows + '</tbody></table></div>'
        : '';

      var emptyHtml = '';
      if (!hasEntries) {
        if (state.blobSearch) {
          emptyHtml =
            '<div class="empty-state stack gap-sm" style="align-items:center;">' +
            '<div>' + esc(t('blobs.noSearchResults')) + '</div>' +
            '<div><button type="button" class="btn btn-sm" id="blob-empty-clear-search">' + esc(t('blobs.clearSearch')) + '</button></div>' +
            '</div>';
        } else if (canWriteBkt) {
          emptyHtml =
            '<div class="empty-state stack gap-sm" style="align-items:center;">' +
            '<div>' + esc(t('blobs.emptyDirHint')) + '</div>' +
            (state.blobDir ? '<div class="text-xs muted">' + esc(t('blobs.virtualDirHint')) + '</div>' : '') +
            '<div class="row" style="justify-content:center;margin-top:4px;">' +
            '<button type="button" class="btn btn-primary btn-sm" id="blob-empty-upload-files">' + esc(t('blobs.uploadFiles')) + '</button>' +
            '<button type="button" class="btn btn-sm" id="blob-empty-upload-dir">' + esc(t('blobs.uploadFolder')) + '</button>' +
            '</div></div>';
        } else {
          emptyHtml = '<div class="empty-state">' + esc(t('blobs.noObjects')) + '</div>';
        }
      }

      return {
        pathbarLeftHtml: pathbarLeftHtml,
        bannersHtml: searchBannerHtml + truncatedBannerHtml,
        tableSlotHtml: tableHtml + emptyHtml,
        dropOverlayText: t('blobs.dropOverlayHint', { target: pathDisplay })
      };
    }

    function bindBlobContentSlotEvents(bCfg, canWriteBkt) {
      var pathLeft = $('blob-pathbar-left');
      var bannersSlot = $('blob-banners-slot');
      var tableSlot = $('blob-browser-table-slot');

      if ($('blob-up-btn')) {
        $('blob-up-btn').addEventListener('click', function () {
          if (state.blobDir) navigateBlobDir(parentBlobDir(state.blobDir), true);
        });
      }
      [pathLeft, tableSlot].forEach(function (container) {
        if (!container) return;
        container.querySelectorAll('[data-nav-dir]').forEach(function (el) {
          el.addEventListener('click', function () {
            navigateBlobDir(el.getAttribute('data-nav-dir') || '', true);
          });
        });
      });
      if ($('blob-clear-search-btn')) {
        $('blob-clear-search-btn').addEventListener('click', function () {
          navigateBlobDir(state.blobDir, true);
        });
      }
      if ($('blob-empty-clear-search')) {
        $('blob-empty-clear-search').addEventListener('click', function () {
          navigateBlobDir(state.blobDir, true);
        });
      }
      if (canWriteBkt) {
        if ($('blob-empty-upload-files')) {
          $('blob-empty-upload-files').addEventListener('click', function () {
            if ($('blob-file-input')) $('blob-file-input').click();
          });
        }
        if ($('blob-empty-upload-dir')) {
          $('blob-empty-upload-dir').addEventListener('click', function () {
            if ($('blob-dir-input')) $('blob-dir-input').click();
          });
        }
      }
      if (!tableSlot) return;
      tableSlot.querySelectorAll('[data-dl-obj]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          downloadBlobObject(bCfg.name, btn.getAttribute('data-dl-obj'));
        });
      });
      tableSlot.querySelectorAll('[data-presign-obj]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          openPresignModal(bCfg.name, btn.getAttribute('data-presign-obj'), canWriteBkt);
        });
      });
      tableSlot.querySelectorAll('[data-del-obj]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          var key = btn.getAttribute('data-del-obj');
          confirmAction(t('blobs.deleteConfirm', { key: key }), async function () {
            try {
              await api('/_blob/' + enc(bCfg.name) + '/' + encPath(key), { method: 'DELETE' });
              toast(t('blobs.deleted'), 'success');
              await loadBucketObjects();
              renderBlobContentPartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      });
      tableSlot.querySelectorAll('[data-del-folder]').forEach(function (btn) {
        btn.addEventListener('click', function () {
          var prefix = btn.getAttribute('data-del-folder');
          var count = btn.getAttribute('data-folder-count') || '0';
          confirmAction(t('blobs.deleteFolderConfirm', { path: prefix, count: count }), async function () {
            try {
              var res = await api('/_api/v1/buckets/' + enc(bCfg.name) + '/prefix?prefix=' + enc(prefix), { method: 'DELETE' });
              toast(t('blobs.folderDeleted', { count: (res && res.deleted) !== undefined ? res.deleted : count }), 'success');
              await loadBucketObjects();
              renderBlobContentPartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      });
    }

    function renderBlobContentPartial() {
      var pathLeft = $('blob-pathbar-left');
      var bannersSlot = $('blob-banners-slot');
      var tableSlot = $('blob-browser-table-slot');
      var bCfg = state.buckets.find(function (b) { return b.name === state.selectedBucket; });
      if (!pathLeft || !bannersSlot || !tableSlot || !bCfg) {
        return renderBlobs(false);
      }
      var canWriteBkt = canWriteResource(bCfg.project_id);
      var parts = buildBlobParts(bCfg, canWriteBkt);
      pathLeft.innerHTML = parts.pathbarLeftHtml;
      bannersSlot.innerHTML = parts.bannersHtml;
      tableSlot.innerHTML = parts.tableSlotHtml;
      var overlay = $('blob-drop-overlay');
      if (overlay) overlay.textContent = parts.dropOverlayText;
      var searchInput = $('blob-search-input');
      if (searchInput && searchInput.value !== state.blobSearch) searchInput.value = state.blobSearch;
      bindBlobContentSlotEvents(bCfg, canWriteBkt);
    }

    function updateBlobBucketInPlace(updatedBucket) {
      if (!updatedBucket) return;
      var idx = state.buckets.findIndex(function (b) { return b.name === updatedBucket.name; });
      if (idx >= 0) state.buckets[idx] = updatedBucket;
      if (state.selectedBucket === updatedBucket.name) {
        var badgeEl = $('blob-header-badge');
        if (badgeEl) badgeEl.innerHTML = buildBlobBadgeHtml(updatedBucket);
      }
      var sel = $('blob-switcher');
      if (sel) {
        for (var i = 0; i < sel.options.length; i++) {
          if (sel.options[i].value === updatedBucket.name) {
            sel.options[i].textContent = updatedBucket.name + (updatedBucket.public_read ? ' (' + t('blobs.public') + ')' : '');
            break;
          }
        }
      }
    }

    async function renderBlobs(fetchData) {
      var root = $('view-root');
      var isCurrent = beginViewRender('blobs');
      if (fetchData) {
        showViewLoadingIfNeeded(root, 'blobs');
        try {
          var res = await Promise.all([api('/_api/v1/buckets'), api('/_api/v1/apps')]);
          if (!isCurrent()) return;
          state.buckets = res[0] || [];
          state.projects = res[1] || [];
          if (!state.selectedBucket && state.buckets.length) {
            state.selectedBucket = state.buckets[0].name;
            resetBlobBrowserState();
          } else if (state.selectedBucket && !state.buckets.some(function (b) { return b.name === state.selectedBucket; })) {
            state.selectedBucket = state.buckets.length ? state.buckets[0].name : '';
            resetBlobBrowserState();
          }
          if (state.selectedBucket) {
            await loadBucketObjects();
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

      var bCfg = state.buckets.find(function (b) { return b.name === state.selectedBucket; });
      var canWriteBkt = bCfg && canWriteResource(bCfg.project_id);
      var createBktBtnHtml = isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="new-bucket-btn">+ ' + esc(t('blobs.newBucket')) + '</button>' : '';

      var rightHtml = '';
      if (!bCfg) {
        rightHtml = '<div class="panel"><div class="empty-state">' + esc(t('blobs.selectBucket')) + '</div></div>';
      } else {
        var parts = buildBlobParts(bCfg, canWriteBkt);

        var uploadProgressHtml = canWriteBkt
          ? '<div id="blob-upload-progress" class="blob-upload-progress hidden">' +
            '<div class="blob-progress-head">' +
            '<span id="blob-progress-title" class="text-strong"></span>' +
            '<span id="blob-progress-stats" class="mono text-xs muted"></span>' +
            '</div>' +
            '<div class="blob-progress-bar"><div id="blob-progress-fill" class="blob-progress-fill"></div></div>' +
            '<div id="blob-progress-file" class="mono text-xs muted truncate"></div>' +
            '</div>'
          : '';

        rightHtml =
          '<div class="stack gap-md">' +
          '<div class="panel"><div class="panel-header"><div class="toolbar" style="width:100%;">' +
          '<div class="row"><h3 class="panel-title mono">' + esc(bCfg.name) + '</h3>' +
          '<span id="blob-header-badge">' + buildBlobBadgeHtml(bCfg) + '</span>' +
          '</div>' +
          '<div class="row">' +
          '<input type="search" id="blob-search-input" style="width:220px;" value="' + esc(state.blobSearch) + '" placeholder="' + esc(t('blobs.searchPlaceholder')) + '">' +
          '<button type="button" class="btn btn-sm" id="blob-filter-btn">' + esc(t('common.filter')) + '</button>' +
          (canWriteBkt ? '<button type="button" class="btn btn-sm" id="blob-edit-bkt-btn">' + esc(t('common.editConfig')) + '</button>' : '') +
          (isAdmin() ? '<button type="button" class="btn btn-sm" id="blob-toggle-pub">' + esc(t('blobs.togglePublic')) + '</button>' : '') +
          (canWriteBkt ? '<button type="button" class="btn btn-danger btn-sm" id="blob-del-bkt-btn">' + esc(t('blobs.deleteBucket')) + '</button>' : '') +
          '</div></div></div>' +
          '<div class="blob-pathbar">' +
          '<div id="blob-pathbar-left" class="blob-breadcrumbs-wrap">' + parts.pathbarLeftHtml + '</div>' +
          (canWriteBkt
            ? '<div class="blob-actions-bar">' +
              '<button type="button" class="btn btn-primary btn-sm" id="blob-upload-files-btn">' + esc(t('blobs.uploadFiles')) + '</button>' +
              '<button type="button" class="btn btn-sm" id="blob-upload-dir-btn">' + esc(t('blobs.uploadFolder')) + '</button>' +
              '<button type="button" class="btn btn-sm" id="blob-new-folder-btn">+ ' + esc(t('blobs.newFolder')) + '</button>' +
              '<input type="file" id="blob-file-input" multiple class="hidden">' +
              '<input type="file" id="blob-dir-input" webkitdirectory directory multiple class="hidden">' +
              '</div>'
            : '') +
          '</div>' +
          '<div id="blob-banners-slot">' + parts.bannersHtml + '</div>' +
          uploadProgressHtml +
          '<div id="blob-browser-body" class="blob-browser-body">' +
          (canWriteBkt ? '<div id="blob-drop-overlay" class="blob-drop-overlay">' + esc(parts.dropOverlayText) + '</div>' : '') +
          '<div id="blob-browser-table-slot">' + parts.tableSlotHtml + '</div>' +
          '</div></div></div>';
      }

      var snap = captureUiSnapshot(root);
      if (state.buckets.length === 0) {
        root.innerHTML = renderResourceEmpty(t('blobs.noBuckets'), createBktBtnHtml);
      } else {
        root.innerHTML =
          renderResourceBar({
            selectId: 'blob-switcher',
            items: state.buckets,
            selectedValue: state.selectedBucket,
            valueFn: function (b) { return b.name; },
            labelFn: function (b) { return b.name + (b.public_read ? ' (' + t('blobs.public') + ')' : ''); },
            actionHtml: createBktBtnHtml
          }) +
          rightHtml;
      }

      if ($('new-bucket-btn')) $('new-bucket-btn').addEventListener('click', openCreateBucketModal);
      if ($('blob-switcher')) {
        $('blob-switcher').addEventListener('change', async function (ev) {
          state.selectedBucket = ev.target.value;
          resetBlobBrowserState();
          try {
            await loadBucketObjects();
            renderBlobs(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }

      if ($('blob-filter-btn')) {
        var runBlobSearch = async function () {
          state.blobSearch = $('blob-search-input').value.trim();
          try {
            await loadBucketObjects();
            renderBlobContentPartial();
          } catch (e) { toast(e.message, 'error'); }
        };
        $('blob-filter-btn').addEventListener('click', runBlobSearch);
        if ($('blob-search-input')) {
          $('blob-search-input').addEventListener('keydown', function (ev) {
            if (ev.key === 'Enter') {
              ev.preventDefault();
              runBlobSearch();
            }
          });
          $('blob-search-input').addEventListener('search', runBlobSearch);
        }
      }

      if ($('blob-toggle-pub')) {
        $('blob-toggle-pub').addEventListener('click', async function () {
          var curBkt = state.buckets.find(function (b) { return b.name === state.selectedBucket; }) || bCfg;
          if (!curBkt) return;
          try {
            var updated = await api('/_api/v1/buckets/' + enc(curBkt.name), {
              method: 'PATCH',
              json: { public_read: !curBkt.public_read }
            });
            toast(t('blobs.bucketUpdated'), 'success');
            updateBlobBucketInPlace(updated || Object.assign({}, curBkt, { public_read: !curBkt.public_read }));
          } catch (e) { toast(e.message, 'error'); }
        });
      }
      if ($('blob-edit-bkt-btn') && bCfg) {
        $('blob-edit-bkt-btn').addEventListener('click', function () {
          var curBkt = state.buckets.find(function (b) { return b.name === state.selectedBucket; }) || bCfg;
          openEditBucketModal(curBkt);
        });
      }
      if ($('blob-del-bkt-btn') && bCfg) {
        $('blob-del-bkt-btn').addEventListener('click', function () {
          var bName = bCfg.name;
          confirmAction(t('blobs.deleteBucketConfirm', { name: bName }), async function () {
            try {
              await api('/_api/v1/buckets/' + enc(bName), { method: 'DELETE' });
              state.selectedBucket = '';
              resetBlobBrowserState();
              toast(t('blobs.bucketDeleted'), 'success');
              renderBlobs(true);
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      }

      if (canWriteBkt && bCfg) {
        var fi = $('blob-file-input');
        var di = $('blob-dir-input');
        var triggerFilePicker = function () { if (fi) fi.click(); };
        var triggerDirPicker = function () { if (di) di.click(); };
        if ($('blob-upload-files-btn')) $('blob-upload-files-btn').addEventListener('click', triggerFilePicker);
        if ($('blob-upload-dir-btn')) $('blob-upload-dir-btn').addEventListener('click', triggerDirPicker);
        if ($('blob-new-folder-btn')) $('blob-new-folder-btn').addEventListener('click', openNewFolderModal);

        if (fi) {
          fi.addEventListener('change', function () {
            var files = fi.files ? Array.from(fi.files) : [];
            fi.value = '';
            if (files.length) {
              uploadBlobEntries(bCfg.name, files.map(function (f) {
                return { file: f, relPath: f.name };
              }));
            }
          });
        }
        if (di) {
          di.addEventListener('change', function () {
            var files = di.files ? Array.from(di.files) : [];
            di.value = '';
            if (files.length) {
              uploadBlobEntries(bCfg.name, files.map(function (f) {
                return { file: f, relPath: f.webkitRelativePath || f.name };
              }));
            }
          });
        }

        var browserBody = $('blob-browser-body');
        if (browserBody) {
          var dragDepth = 0;
          var isFileDrag = function (e) {
            var dt = e.dataTransfer;
            if (!dt || !dt.types) return true;
            return Array.from(dt.types).indexOf('Files') !== -1;
          };
          browserBody.addEventListener('dragenter', function (e) {
            if (!isFileDrag(e)) return;
            e.preventDefault();
            dragDepth += 1;
            browserBody.classList.add('dragover');
          });
          browserBody.addEventListener('dragover', function (e) {
            if (!isFileDrag(e)) return;
            e.preventDefault();
            if (e.dataTransfer) e.dataTransfer.dropEffect = 'copy';
          });
          browserBody.addEventListener('dragleave', function () {
            if (dragDepth === 0) return;
            dragDepth = Math.max(0, dragDepth - 1);
            if (dragDepth === 0) browserBody.classList.remove('dragover');
          });
          browserBody.addEventListener('drop', async function (e) {
            if (!isFileDrag(e)) return;
            e.preventDefault();
            dragDepth = 0;
            browserBody.classList.remove('dragover');
            try {
              var entries = await collectDroppedEntries(e.dataTransfer);
              if (entries.length) {
                uploadBlobEntries(bCfg.name, entries);
              }
            } catch (err) {
              toast(err.message || String(err), 'error');
            }
          });
        }
      }

      if (bCfg) bindBlobContentSlotEvents(bCfg, canWriteBkt);
      markViewRendered(root, 'blobs');
      restoreUiSnapshot(root, snap);
    }

    async function loadBucketObjects() {
      var params = [];
      if (state.blobDir) params.push('prefix=' + enc(state.blobDir));
      if (state.blobSearch) params.push('search=' + enc(state.blobSearch));
      var qs = params.length ? '?' + params.join('&') : '';
      var res = await api('/_api/v1/buckets/' + enc(state.selectedBucket) + '/browse' + qs);
      state.blobDir = (res && res.prefix) || '';
      state.blobFolders = (res && res.folders) || [];
      state.blobObjects = (res && res.objects) || [];
      state.blobTruncated = Boolean(res && res.truncated);
    }

    async function collectDroppedEntries(dataTransfer) {
      var out = [];
      var items = dataTransfer && dataTransfer.items ? Array.from(dataTransfer.items) : [];
      var rootEntries = [];
      for (var i = 0; i < items.length; i++) {
        if (typeof items[i].webkitGetAsEntry === 'function') {
          var entry = items[i].webkitGetAsEntry();
          if (entry) rootEntries.push(entry);
        }
      }
      if (rootEntries.length > 0) {
        var readAllDirectoryEntries = async function (dirReader) {
          var all = [];
          while (true) {
            var batch = await new Promise(function (resolve, reject) {
              dirReader.readEntries(resolve, reject);
            });
            if (!batch || batch.length === 0) break;
            for (var j = 0; j < batch.length; j++) all.push(batch[j]);
          }
          return all;
        };
        var walkEntry = async function (entry) {
          if (entry.isFile) {
            var file = await new Promise(function (resolve, reject) {
              entry.file(resolve, reject);
            });
            var relPath = (entry.fullPath || entry.name || '').replace(/^\/+/, '');
            out.push({ file: file, relPath: relPath || file.name });
          } else if (entry.isDirectory) {
            var children = await readAllDirectoryEntries(entry.createReader());
            for (var k = 0; k < children.length; k++) {
              await walkEntry(children[k]);
            }
          }
        };
        for (var r = 0; r < rootEntries.length; r++) {
          await walkEntry(rootEntries[r]);
        }
        return out;
      }
      var files = dataTransfer && dataTransfer.files ? Array.from(dataTransfer.files) : [];
      return files.map(function (f) {
        return { file: f, relPath: f.webkitRelativePath || f.name };
      });
    }

    async function uploadBlobEntries(bucket, entries) {
      var valid = [];
      for (var i = 0; i < entries.length; i++) {
        var item = entries[i];
        if (!item || !item.file) continue;
        var rawRel = String(item.relPath || item.file.name || '').replace(/\\/g, '/').replace(/^\/+|\/+$/g, '');
        var segs = rawRel.split('/').filter(function (s) { return s.length > 0; });
        if (!segs.length || segs.some(function (s) { return s === '.' || s === '..' || s.indexOf('\0') >= 0; })) {
          continue;
        }
        valid.push({ file: item.file, relPath: segs.join('/') });
      }
      if (!valid.length) return;

      var baseDir = state.blobDir || '';
      var progressBox = $('blob-upload-progress');
      var titleEl = $('blob-progress-title');
      var statsEl = $('blob-progress-stats');
      var fillEl = $('blob-progress-fill');
      var fileEl = $('blob-progress-file');
      var total = valid.length;
      var done = 0;
      var succeeded = 0;
      var failed = 0;
      var firstErr = '';

      if (progressBox) progressBox.classList.remove('hidden');
      if (titleEl) titleEl.textContent = t('blobs.uploadToTarget', { target: '/' + baseDir });

      for (var j = 0; j < total; j++) {
        var entry = valid[j];
        var key = baseDir + entry.relPath;
        var pct = Math.round((done / total) * 100);
        if (statsEl) statsEl.textContent = done + ' / ' + total + ' (' + pct + '%)';
        if (fillEl) fillEl.style.width = pct + '%';
        if (fileEl) fileEl.textContent = key;

        try {
          await api('/_blob/' + enc(bucket) + '/' + encPath(key), {
            method: 'PUT',
            headers: entry.file.type ? { 'Content-Type': entry.file.type } : {},
            body: entry.file
          });
          succeeded += 1;
        } catch (e) {
          failed += 1;
          if (!firstErr) firstErr = e.message || String(e);
        }
        done += 1;
      }

      if (statsEl) statsEl.textContent = done + ' / ' + total + ' (100%)';
      if (fillEl) fillEl.style.width = '100%';
      if (progressBox) progressBox.classList.add('hidden');

      if (failed === 0) {
        toast(t('blobs.uploaded', { count: succeeded }), 'success');
      } else {
        toast(t('blobs.uploadPartialFailed', { ok: succeeded, fail: failed, err: firstErr }), 'error');
      }

      if (state.currentRoute === 'blobs' && state.selectedBucket === bucket) {
        try {
          await loadBucketObjects();
          renderBlobContentPartial();
        } catch (e) { toast(e.message, 'error'); }
      }
    }

    function openNewFolderModal() {
      openModal({
        title: t('blobs.newFolderTitle'),
        body:
          '<form id="new-folder-form" class="stack gap-sm">' +
          '<div class="field"><label>' + esc(t('blobs.newFolderLabel')) + '</label>' +
          '<input type="text" id="nf-name" required placeholder="' + esc(t('blobs.newFolderPlaceholder')) + '"></div>' +
          '<div class="text-xs muted">' + esc(t('blobs.newFolderHint')) + '</div>' +
          '</form>',
        footer:
          '<button type="button" class="btn" id="nf-cancel">' + esc(t('common.cancel')) + '</button>' +
          '<button type="submit" form="new-folder-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
        onMount: function () {
          $('nf-cancel').addEventListener('click', closeModal);
          if ($('nf-name')) $('nf-name').focus();
          withSubmitLock('new-folder-form', async function () {
            var raw = $('nf-name').value.trim();
            var clean = raw.replace(/^\/+|\/+$/g, '');
            if (!clean || raw.indexOf('\0') >= 0 || raw.indexOf('\\') >= 0 ||
                clean.split('/').some(function (s) { return !s || s === '.' || s === '..'; })) {
              toast(t('blobs.invalidFolderName'), 'error');
              return;
            }
            closeModal();
            await navigateBlobDir((state.blobDir || '') + clean + '/', true);
          });
        }
      });
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
          '<div class="field"><label>' + esc(t('blobs.rateLimitRpm')) + '</label><input type="number" id="cb-rpm" min="1" placeholder="600"></div>' +
          '</div></details>' +
          '</form>',
        footer:
          '<button type="button" class="btn" id="cb-cancel">' + esc(t('common.cancel')) + '</button>' +
          '<button type="submit" form="create-bkt-form" class="btn btn-primary">' + esc(t('common.create')) + '</button>',
        onMount: function () {
          loadResourceGroupOptions();
          $('cb-cancel').addEventListener('click', closeModal);
          withSubmitLock('create-bkt-form', async function () {
            var name = $('cb-name').value.trim();
            var maxObjRaw = $('cb-max-obj').value.trim();
            var quotaRaw = $('cb-quota').value.trim();
            var rpmRaw = $('cb-rpm').value.trim();
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
                  cors_origins: $('cb-cors').value.trim() || null,
                  rate_limit_rpm: rpmRaw ? Number(rpmRaw) : null
                }
              });
              closeModal();
              state.selectedBucket = name;
              resetBlobBrowserState();
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
          '<div class="field"><label>' + esc(t('blobs.rateLimitRpm')) + '</label><input type="number" id="eb-rpm" min="0" value="' + esc(bCfg.rate_limit_rpm || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
          '</form>',
        footer:
          '<button type="button" class="btn" id="eb-cancel">' + esc(t('common.cancel')) + '</button>' +
          '<button type="submit" form="edit-bkt-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
        onMount: function () {
          loadResourceGroupOptions();
          $('eb-cancel').addEventListener('click', closeModal);
          withSubmitLock('edit-bkt-form', async function () {
            var maxObjRaw = $('eb-max-obj').value.trim();
            var quotaRaw = $('eb-quota').value.trim();
            var rpmRaw = $('eb-rpm').value.trim();
            var body = {
              group: $('resource-group').value.trim(),
              description: $('eb-desc').value.trim(),
              max_object_size_mb: maxObjRaw ? Number(maxObjRaw) : 0,
              quota_mb: quotaRaw ? Number(quotaRaw) : 0,
              allowed_content_types: $('eb-types').value.trim(),
              cache_control: $('eb-cache').value.trim(),
              cors_origins: $('eb-cors').value.trim(),
              rate_limit_rpm: rpmRaw ? Number(rpmRaw) : 0
            };
            if ($('eb-pub')) body.public_read = $('eb-pub').checked;
            try {
              var updated = await api('/_api/v1/buckets/' + enc(bCfg.name), { method: 'PATCH', json: body });
              closeModal();
              toast(t('blobs.bucketUpdated'), 'success');
              updateBlobBucketInPlace(updated);
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
          withSubmitLock('presign-form', async function () {
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

    return { renderBlobs: renderBlobs };
  };
})();
