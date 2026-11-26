(function () {
  'use strict';

  var views = (window.__FP_VIEWS = window.__FP_VIEWS || {});
  views.queues = function (ctx) {
    var state = ctx.state;
    var $ = ctx.$;
    var t = ctx.t;
    var esc = ctx.esc;
    var enc = ctx.enc;
    var formatTime = ctx.formatTime;
    var decodePayloadBytes = ctx.decodePayloadBytes;
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

    // --- 5. Queues ---

    function buildQueueStatsHtml(st) {
      return '<div class="stats-grid">' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsPending')) + '</span><span class="stat-value">' + esc(st.pending) + '</span></div>' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsRunning')) + '</span><span class="stat-value">' + esc(st.running) + '</span></div>' +
        '<div class="stat-card"><span class="stat-label">' + esc(t('queues.statsDead')) + '</span><span class="stat-value" style="color:' + (st.dead > 0 ? 'var(--danger)' : 'inherit') + ';">' + esc(st.dead) + '</span></div>' +
        '</div>';
    }

    function buildQueueBadgesHtml(qCfg) {
      return (qCfg.ephemeral ? '<span class="badge badge-info">' + esc(t('queues.ephemeralBadge')) + '</span>' : '') +
        (qCfg.max_pending_jobs ? '<span class="badge badge-neutral text-xs">max:' + esc(qCfg.max_pending_jobs) + '</span>' : '');
    }

    function buildQueueReplayAllHtml(qCfg, canWriteQ, st) {
      return (canWriteQ && st.dead > 0)
        ? '<button type="button" class="btn btn-sm" id="q-replay-all-btn">' + esc(t('queues.replayAll')) + '</button>'
        : '';
    }

    function buildQueueJobsHtml(canWriteQ) {
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
      return jobRows
        ? '<div class="table-wrap"><table class="data-table stack-mobile"><thead><tr><th>' + esc(t('queues.jobId')) + '</th><th>' + esc(t('queues.payload')) + '</th><th>' + esc(t('queues.attempts')) + '</th><th>' + esc(t('queues.lastError')) + '</th><th>' + esc(t('queues.runAt')) + '</th><th class="th-actions">' + esc(t('common.actions')) + '</th></tr></thead><tbody>' + jobRows + '</tbody></table></div>'
        : '<div class="empty-state">' + esc(t('queues.noJobs')) + '</div>';
    }

    function bindQueueReplayEvents(qCfg, canWriteQ) {
      if ($('q-replay-all-btn')) {
        $('q-replay-all-btn').addEventListener('click', async function () {
          try {
            var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/dlq/replay', { method: 'POST', json: {} });
            toast(t('queues.replayed', { count: res.replayed }), 'success');
            await loadSelectedQueue();
            renderQueueDetailPartial();
          } catch (e) { toast(e.message, 'error'); }
        });
      }
      var jobsSlot = $('queue-jobs-slot');
      if (jobsSlot) {
        jobsSlot.querySelectorAll('[data-replay-job]').forEach(function (btn) {
          btn.addEventListener('click', async function () {
            var jobId = Number(btn.getAttribute('data-replay-job'));
            try {
              var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/dlq/replay', {
                method: 'POST',
                json: { job_ids: [jobId] }
              });
              toast(t('queues.replayed', { count: res.replayed }), 'success');
              await loadSelectedQueue();
              renderQueueDetailPartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        });
      }
    }

    function renderQueueDetailPartial() {
      var statsSlot = $('queue-stats-slot');
      var jobsSlot = $('queue-jobs-slot');
      var qCfg = state.queues.find(function (q) { return q.name === state.selectedQueue; });
      if (!statsSlot || !jobsSlot || !qCfg) return renderQueues(false);
      var canWriteQ = canWriteResource(qCfg.project_id);
      var st = state.queueStats || { pending: 0, running: 0, dead: 0 };
      statsSlot.innerHTML = buildQueueStatsHtml(st);
      var replaySlot = $('queue-replay-all-slot');
      if (replaySlot) replaySlot.innerHTML = buildQueueReplayAllHtml(qCfg, canWriteQ, st);
      jobsSlot.innerHTML = buildQueueJobsHtml(canWriteQ);
      bindQueueReplayEvents(qCfg, canWriteQ);
    }

    function updateQueueInPlace(updatedQ) {
      if (!updatedQ) return;
      var idx = state.queues.findIndex(function (q) { return q.name === updatedQ.name; });
      if (idx >= 0) state.queues[idx] = updatedQ;
      if (state.selectedQueue === updatedQ.name) {
        var badgesEl = $('queue-header-badges');
        if (badgesEl) badgesEl.innerHTML = buildQueueBadgesHtml(updatedQ);
      }
      var sel = $('queue-switcher');
      if (sel) {
        for (var i = 0; i < sel.options.length; i++) {
          if (sel.options[i].value === updatedQ.name) {
            sel.options[i].textContent = updatedQ.name + (updatedQ.project_id ? ' · ' + projectNameById(updatedQ.project_id) : '');
            break;
          }
        }
      }
    }

    async function renderQueues(fetchData) {
      var root = $('view-root');
      var isCurrent = beginViewRender('queues');
      if (state.currentRoute !== 'queues') return;
      if (fetchData) {
        showViewLoadingIfNeeded(root, 'queues');
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
          if (isCurrent()) {
            root.removeAttribute('data-view-key');
            root.innerHTML = '<div class="alert alert-error">' + esc(e.message) + '</div>';
          }
          return;
        }
      }
      if (!isCurrent()) return;

      var qCfg = state.queues.find(function (q) { return q.name === state.selectedQueue; });
      var canWriteQ = qCfg && canWriteResource(qCfg.project_id);
      var createQBtnHtml = isAdmin() ? '<button type="button" class="btn btn-primary btn-sm" id="new-queue-btn">+ ' + esc(t('queues.new')) + '</button>' : '';

      var detailHtml = '';
      if (!qCfg) {
        detailHtml = '<div class="panel"><div class="empty-state">' + esc(t('queues.selectQueue')) + '</div></div>';
      } else {
        var st = state.queueStats || { pending: 0, running: 0, dead: 0 };

        detailHtml =
          '<div class="stack gap-md">' +
          '<div id="queue-stats-slot">' + buildQueueStatsHtml(st) + '</div>' +
          '<div class="panel"><div class="panel-header"><div class="toolbar">' +
          '<div class="row">' +
          '<h3 class="panel-title mono">' + esc(qCfg.name) + '</h3>' +
          '<span id="queue-header-badges" class="row gap-xs">' + buildQueueBadgesHtml(qCfg) + '</span>' +
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
              '<span id="queue-replay-all-slot">' + buildQueueReplayAllHtml(qCfg, canWriteQ, st) + '</span>' +
              '<button type="button" class="btn btn-danger btn-sm" id="q-purge-btn">' + esc(t('queues.purge')) + '</button>' +
              '<button type="button" class="btn btn-danger btn-sm" id="q-delete-btn">' + esc(t('queues.deleteQueue')) + '</button>' +
              '</div>'
            : '') +
          '</div></div>' +
          '<div id="queue-jobs-slot">' + buildQueueJobsHtml(canWriteQ) + '</div>' +
          '</div></div>';
      }

      var snap = captureUiSnapshot(root);
      if (state.queues.length === 0) {
        root.innerHTML = renderResourceEmpty(t('queues.empty'), createQBtnHtml);
      } else {
        root.innerHTML =
          renderResourceBar({
            selectId: 'queue-switcher',
            items: state.queues,
            selectedValue: state.selectedQueue,
            valueFn: function (q) { return q.name; },
            labelFn: function (q) { return q.name + (q.project_id ? ' · ' + projectNameById(q.project_id) : ''); },
            actionHtml: createQBtnHtml
          }) +
          detailHtml;
      }

      if ($('new-queue-btn')) $('new-queue-btn').addEventListener('click', openCreateQueueModal);
      if ($('queue-switcher')) {
        $('queue-switcher').addEventListener('change', async function (ev) {
          state.selectedQueue = ev.target.value;
          try {
            await loadSelectedQueue();
            renderQueues(false);
          } catch (e) { toast(e.message, 'error'); }
        });
      }
      if ($('queue-inspect-status')) {
        $('queue-inspect-status').addEventListener('change', async function (e) {
          state.queueInspectStatus = e.target.value;
          try {
            await loadSelectedQueue();
            renderQueueDetailPartial();
          } catch (err) { toast(err.message, 'error'); }
        });
      }
      if ($('q-enqueue-btn')) $('q-enqueue-btn').addEventListener('click', function () { openEnqueueModal(qCfg.name); });
      if ($('q-edit-btn')) {
        $('q-edit-btn').addEventListener('click', function () {
          var latest = state.queues.find(function (q) { return q.name === state.selectedQueue; }) || qCfg;
          openEditQueueModal(latest);
        });
      }
      if ($('q-purge-btn')) {
        $('q-purge-btn').addEventListener('click', function () {
          confirmAction(t('queues.purgeConfirm', { name: qCfg.name }), async function () {
            try {
              var res = await api('/_api/v1/queues/' + enc(qCfg.name) + '/purge', { method: 'POST' });
              toast(t('queues.purged', { count: res.purged }), 'success');
              await loadSelectedQueue();
              renderQueueDetailPartial();
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
      if (qCfg) bindQueueReplayEvents(qCfg, canWriteQ);
      markViewRendered(root, 'queues');
      restoreUiSnapshot(root, snap);
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
          withSubmitLock('edit-queue-form', async function () {
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
              var updated = await api('/_api/v1/queues/' + enc(qCfg.name), { method: 'PATCH', json: body });
              closeModal();
              toast(t('queues.updated'), 'success');
              updateQueueInPlace(updated);
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
          withSubmitLock('create-q-form', async function () {
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
          withSubmitLock('enqueue-form', async function () {
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
              renderQueueDetailPartial();
            } catch (e) { toast(e.message, 'error'); }
          });
        }
      });
    }

    return { renderQueues: renderQueues };
  };
})();
