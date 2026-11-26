(function () {
  'use strict';

  var views = (window.__FP_VIEWS = window.__FP_VIEWS || {});
  views.search = function (ctx) {
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
    var isReadonly = ctx.isReadonly;
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
    var searchQuerySeq = 0;


    var searchState = {
      indexes: [],
      selected: '',
      tab: 'playground',
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
      renderSearch(false);
    }

    function openUpsertSearchDocModal(selectedIndex, initialDoc) {
      var jsonVal = initialDoc
        ? JSON.stringify(initialDoc, null, 2)
        : '{\n  "id": "",\n  "title": "",\n  "body": ""\n}';
      openModal({
        title: t('search.upsertDoc') + ' · ' + selectedIndex.name,
        body:
          '<form id="upsert-search-doc-form" class="stack gap-md">' +
          '<div class="field"><label for="real-search-doc">' + esc(t('search.docJsonLabel')) + '</label>' +
          '<textarea id="real-search-doc" class="mono" rows="8" required>' + esc(jsonVal) + '</textarea></div>' +
          '</form>',
        footer:
          '<button type="button" class="btn" id="usd-cancel">' + esc(t('common.cancel')) + '</button>' +
          '<button type="submit" form="upsert-search-doc-form" class="btn btn-primary">' + esc(t('common.save')) + '</button>',
        onMount: function () {
          $('usd-cancel').addEventListener('click', closeModal);
          withSubmitLock('upsert-search-doc-form', async function () {
            var targetIndexId = selectedIndex.id;
            var reqToken = state.token;
            try {
              var doc = JSON.parse($('real-search-doc').value);
              await api('/_api/v1/search/' + enc(targetIndexId) + '/docs', { method: 'POST', json: doc });
              closeModal();
              if (state.currentRoute !== 'search' || state.token !== reqToken || searchState.selected !== targetIndexId) return;
              searchState.result = null;
              searchState.offset = 0;
              toast(t('common.updated'), 'success');
              renderSearch();
            } catch (e) { toast(e.message, 'error'); }
          });
        }
      });
    }

    async function renderSearch(fetchData) {
      var root = $('view-root');
      if (state.currentRoute !== 'search') return;
      var isCurrentSearch = beginViewRender('search');
      if (fetchData) {
        showViewLoadingIfNeeded(root, 'search');
      }
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
        var docs = (selected && searchState.tab === 'docs')
          ? await api('/_api/v1/search/' + enc(selected.id) + '/docs?limit=101&offset=' + searchState.offset)
          : [];
        if (!isCurrentSearch()) return;
        var hasMoreDocs = docs.length > 100;
        docs = docs.slice(0, 100);
        var writable = selected && canWriteResource(selected.project_id);
        var createSearchBtnHtml = !isReadonly() ? '<button type="button" class="btn btn-primary btn-sm" id="search-create-index">+ ' + esc(t('search.newIndex')) + '</button>' : '';

        var facetEntries = (searchState.result && searchState.result.facets) ? Object.keys(searchState.result.facets) : [];
        var facetsHtml = facetEntries.length ? '<div class="row gap-xs">' + facetEntries.map(function (field) {
          var vals = searchState.result.facets[field] || {};
          return Object.keys(vals).map(function (val) {
            var active = searchState.facets[field] === val;
            return '<button type="button" class="btn btn-sm ' + (active ? 'btn-primary' : '') + '" data-search-facet="' + esc(field) + '" data-facet-val="' + esc(val) + '">' + esc(field + ':' + val + ' (' + vals[val] + ')') + '</button>';
          }).join('');
        }).join('') + '</div>' : '';

        var tabBodyHtml = '';
        if (selected) {
          if (searchState.tab === 'docs') {
            tabBodyHtml =
              '<div class="row-between">' +
              '<div class="row gap-xs">' +
              '<button type="button" class="btn btn-sm" id="search-prev"' + (searchState.offset === 0 ? ' disabled' : '') + '>' + esc(t('search.previousPage')) + '</button>' +
              '<button type="button" class="btn btn-sm" id="search-next"' + (!hasMoreDocs ? ' disabled' : '') + '>' + esc(t('search.nextPage')) + '</button>' +
              '</div>' +
              (writable ? '<button type="button" class="btn btn-primary btn-sm" id="search-upsert-doc-btn">+ ' + esc(t('search.upsertDoc')) + '</button>' : '') +
              '</div>' +
              (docs.length
                ? '<div class="stack gap-sm">' + docs.map(function (doc) {
                    return '<div class="panel"><div class="panel-header"><span class="mono text-xs text-strong">' + esc(doc.id || '-') + '</span>' +
                      (writable && doc.id
                        ? '<div class="row gap-xs">' +
                          '<button type="button" class="btn btn-sm" data-search-doc-edit="' + esc(doc.id) + '">' + esc(t('common.edit')) + '</button>' +
                          '<button type="button" class="btn btn-danger btn-sm" data-search-doc-delete="' + esc(doc.id) + '">' + esc(t('common.delete')) + '</button>' +
                          '</div>'
                        : '') +
                      '</div><div class="panel-body" style="padding:12px;"><pre class="code-block" style="margin:0;">' + esc(JSON.stringify(doc, null, 2)) + '</pre></div></div>';
                  }).join('') + '</div>'
                : '<div class="empty-state stack gap-sm" style="align-items:center;">' +
                  '<span>' + esc(t('common.empty')) + '</span>' +
                  (writable ? '<button type="button" class="btn btn-primary" id="search-empty-upsert-doc">+ ' + esc(t('search.upsertDoc')) + '</button>' : '') +
                  '</div>');
          } else {
            tabBodyHtml =
              '<form id="real-search-form" class="stack gap-sm">' +
              '<div class="row">' +
              '<input type="search" id="real-search-query" value="' + esc(searchState.query) + '" placeholder="' + esc(t('search.queryPlaceholder')) + '" style="flex:1 1 220px;">' +
              '<label class="checkbox-row"><input type="checkbox" id="real-search-fuzzy"' + (searchState.fuzzy ? ' checked' : '') + '> ' + esc(t('search.fuzzy')) + '</label>' +
              '<select id="real-search-sort" style="width:auto;">' +
              '<option value=""' + (!searchState.sort ? ' selected' : '') + '>' + esc(t('search.sortRelevance')) + '</option>' +
              '<option value="updated_desc"' + (searchState.sort === 'updated_desc' ? ' selected' : '') + '>' + esc(t('search.sortUpdated')) + '</option>' +
              '</select>' +
              '<button class="btn btn-primary" type="submit">' + esc(t('common.search')) + '</button>' +
              '</div>' +
              '<details class="adv-options"' + (searchState.vectorText ? ' open' : '') + '><summary>' + esc(t('search.hybridVector')) + '</summary>' +
              '<div class="adv-options-body form-grid">' +
              '<div class="field"><label for="real-search-vector">' + esc(t('search.vectorArray')) + '</label><input type="text" id="real-search-vector" class="mono" value="' + esc(searchState.vectorText) + '" placeholder="[0.12, -0.34, 0.56]"></div>' +
              '<div class="field"><label for="real-search-alpha">' + esc(t('search.vectorAlpha')) + '</label><input type="number" id="real-search-alpha" min="0" max="1" step="0.05" value="' + esc(searchState.alpha) + '"></div>' +
              '</div></details></form>' +
              facetsHtml +
              (searchState.result ? '<pre class="code-block">' + esc(JSON.stringify(searchState.result, null, 2)) + '</pre>' : '');
          }
        }

        var detailHtml = !selected
          ? '<div class="panel"><div class="empty-state">' + esc(t('search.selectIndex')) + '</div></div>'
          : '<div class="panel"><div class="panel-header"><div class="toolbar">' +
            '<div><div class="row gap-xs"><h3 class="panel-title mono">' + esc(selected.name) + '</h3>' +
            (selected.tokenizer ? '<span class="badge badge-info mono">' + esc(selected.tokenizer) + '</span>' : '') +
            '<span class="badge badge-neutral mono">' + esc(t('search.statDocs')) + ': ' + esc(selected.doc_count || 0) + '</span>' +
            '</div>' +
            (selected.description ? '<div class="muted text-xs">' + esc(selected.description) + '</div>' : '') +
            '</div>' +
            (writable
              ? '<div class="row">' +
                '<button type="button" class="btn btn-sm" id="search-commit-index">' + esc(t('search.commitIndex')) + '</button>' +
                '<button type="button" class="btn btn-sm" id="search-edit-index">' + esc(t('common.editConfig')) + '</button>' +
                '<button type="button" class="btn btn-danger btn-sm" id="search-delete-index">' + esc(t('search.deleteIndex')) + '</button>' +
                '</div>'
              : '') +
            '</div></div>' +
            '<div class="panel-body stack gap-md">' +
            '<div class="tabs" role="tablist">' +
            '<button type="button" class="tab-btn' + (searchState.tab === 'playground' ? ' active' : '') + '" data-search-tab="playground">' + esc(t('search.tabPlayground')) + '</button>' +
            '<button type="button" class="tab-btn' + (searchState.tab === 'docs' ? ' active' : '') + '" data-search-tab="docs">' + esc(t('search.tabDocs')) + ' (' + esc(selected.doc_count || 0) + ')</button>' +
            '</div>' +
            tabBodyHtml +
            '</div></div>';

        var snap = captureUiSnapshot(root);
        if (searchState.indexes.length === 0) {
          root.innerHTML = renderResourceEmpty(t('search.noIndexes'), createSearchBtnHtml);
        } else {
          root.innerHTML =
            renderResourceBar({
              selectId: 'search-switcher',
              items: searchState.indexes,
              selectedValue: searchState.selected,
              valueFn: function (item) { return item.id; },
              labelFn: function (item) { return item.name + ' (' + (item.doc_count || 0) + ')'; },
              actionHtml: createSearchBtnHtml
            }) +
            detailHtml;
        }

        if ($('search-switcher')) {
          $('search-switcher').addEventListener('change', function (ev) {
            searchState.selected = ev.target.value;
            searchState.offset = 0;
            searchState.facets = {};
            searchState.result = null;
            renderSearch(false);
          });
        }
        root.querySelectorAll('[data-search-tab]').forEach(function (btn) {
          btn.addEventListener('click', function () {
            searchState.tab = btn.getAttribute('data-search-tab') || 'playground';
            renderSearch(false);
          });
        });
        if ($('search-upsert-doc-btn') && selected) {
          $('search-upsert-doc-btn').addEventListener('click', function () {
            openUpsertSearchDocModal(selected, null);
          });
        }
        if ($('search-empty-upsert-doc') && selected) {
          $('search-empty-upsert-doc').addEventListener('click', function () {
            openUpsertSearchDocModal(selected, null);
          });
        }
        root.querySelectorAll('[data-search-doc-edit]').forEach(function (btn) {
          btn.addEventListener('click', function () {
            var docId = btn.getAttribute('data-search-doc-edit');
            var found = docs.find(function (d) { return String(d.id) === String(docId); }) || null;
            if (selected) openUpsertSearchDocModal(selected, found);
          });
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
              '<div class="form-grid">' +
              '<div class="field"><label>' + esc(t('search.maxDocs')) + '</label><input type="number" id="esi-max-docs" min="0" value="' + esc(selected.max_docs || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
              '<div class="field"><label>' + esc(t('search.rateLimitRpm')) + '</label><input type="number" id="esi-rpm" min="0" value="' + esc(selected.rate_limit_rpm || '') + '" placeholder="' + esc(t('common.unlimited')) + '"></div>' +
              '</div>' +
              '</form>',
            footer:
              '<button type="button" class="btn" id="esi-cancel">' + esc(t('common.cancel')) + '</button>' +
              '<button form="edit-search-index-form" class="btn btn-primary" type="submit">' + esc(t('common.save')) + '</button>',
            onMount: function () {
              loadResourceGroupOptions();
              $('esi-cancel').addEventListener('click', closeModal);
              withSubmitLock('edit-search-index-form', async function () {
                var maxDocsRaw = $('esi-max-docs').value.trim();
                var rpmRaw = $('esi-rpm').value.trim();
                try {
                  await api('/_api/v1/search/' + enc(selected.id), {
                    method: 'PATCH',
                    json: {
                      group: $('resource-group').value.trim(),
                      description: $('esi-desc').value.trim(),
                      writer_budget_mb: Number($('esi-budget').value || 20),
                      max_docs: maxDocsRaw ? Number(maxDocsRaw) : 0,
                      rate_limit_rpm: rpmRaw ? Number(rpmRaw) : 0
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
              '<div class="form-grid">' +
              '<div class="field"><label>' + esc(t('search.maxDocs')) + '</label><input type="number" id="search-index-max-docs" min="1" placeholder="100000"></div>' +
              '<div class="field"><label>' + esc(t('search.rateLimitRpm')) + '</label><input type="number" id="search-index-rpm" min="1" placeholder="600"></div>' +
              '</div>' +
              '<div class="field"><label>' + esc(t('search.facetFields')) + '</label><input type="text" id="search-index-facets" placeholder="category,status,lang,source,level"></div>' +
              '</div></details>' +
              '</form>',
            footer:
              '<button type="button" class="btn" id="si-cancel">' + esc(t('common.cancel')) + '</button>' +
              '<button form="search-index-form" class="btn btn-primary" type="submit">' + esc(t('common.create')) + '</button>',
            onMount: function () {
              loadResourceGroupOptions();
              $('si-cancel').addEventListener('click', closeModal);
              withSubmitLock('search-index-form', async function () {
                var facetsRaw = $('search-index-facets').value.trim();
                var facets = facetsRaw ? facetsRaw.split(',').map(function (s) { return s.trim(); }).filter(Boolean) : null;
                var budgetRaw = $('search-index-budget').value.trim();
                var maxDocsRaw = $('search-index-max-docs').value.trim();
                var rpmRaw = $('search-index-rpm').value.trim();
                try {
                  var created = await api('/_api/v1/search', { method: 'POST', json: {
                    name: $('search-index-name').value.trim(),
                    group: selectedResourceGroup(),
                    project_id: $('search-index-owner').value || null,
                    description: $('search-index-desc').value.trim() || null,
                    tokenizer: $('search-index-tok').value || null,
                    writer_budget_mb: budgetRaw ? Number(budgetRaw) : null,
                    max_docs: maxDocsRaw ? Number(maxDocsRaw) : null,
                    rate_limit_rpm: rpmRaw ? Number(rpmRaw) : null,
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
        if ($('real-search-sort')) {
        $('real-search-sort').addEventListener('change', async function () {
          if ($('real-search-query')) searchState.query = $('real-search-query').value;
          searchState.fuzzy = Boolean($('real-search-fuzzy') && $('real-search-fuzzy').checked);
          searchState.sort = $('real-search-sort').value;
          try { await runRealSearchQuery(selected.id); } catch (e) { toast(e.message, 'error'); }
        });
      }
      if ($('real-search-fuzzy')) {
        $('real-search-fuzzy').addEventListener('change', async function () {
          if ($('real-search-query')) searchState.query = $('real-search-query').value;
          searchState.fuzzy = Boolean($('real-search-fuzzy').checked);
          searchState.sort = $('real-search-sort') ? $('real-search-sort').value : '';
          try { await runRealSearchQuery(selected.id); } catch (e) { toast(e.message, 'error'); }
        });
      }
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
        markViewRendered(root, 'search');
        restoreUiSnapshot(root, snap);
      } catch (e) {
        if (isCurrentSearch()) {
          root.removeAttribute('data-view-key');
          root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
        }
      }
    }

    return { renderSearch: renderSearch };
  };
})();
