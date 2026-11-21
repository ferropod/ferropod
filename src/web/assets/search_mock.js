(function () {
  'use strict';

  function escHtml(val) {
    if (val === null || val === undefined) return '';
    return String(val)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }

  // Split a query into searchable terms (handles whitespace + mixed CJK/ASCII)
  function tokenizeQuery(query) {
    var raw = String(query || '').trim();
    if (!raw) return [];
    var parts = raw.split(/\s+/).filter(Boolean);
    var terms = [];
    parts.forEach(function (p) {
      var lower = p.toLowerCase();
      if (terms.indexOf(lower) === -1) terms.push(lower);
      // For multi-char CJK chunks, also add bigrams so partial Chinese phrases still score well
      var cjk = lower.match(/[\u4e00-\u9fff]{3,}/g);
      if (cjk) {
        cjk.forEach(function (seg) {
          for (var i = 0; i < seg.length - 1; i++) {
            var bi = seg.slice(i, i + 2);
            if (terms.indexOf(bi) === -1) terms.push(bi);
          }
        });
      }
    });
    return terms;
  }

  // Levenshtein distance <= 1 check for optional fuzzy matching on ASCII tokens
  function editDistanceOne(a, b) {
    if (Math.abs(a.length - b.length) > 1) return false;
    if (a === b) return true;
    var i = 0, j = 0, diffs = 0;
    while (i < a.length && j < b.length) {
      if (a[i] === b[j]) {
        i++; j++;
      } else {
        diffs++;
        if (diffs > 1) return false;
        if (a.length > b.length) i++;
        else if (b.length > a.length) j++;
        else { i++; j++; }
      }
    }
    if (i < a.length || j < b.length) diffs++;
    return diffs <= 1;
  }

  function highlightText(text, query) {
    var safe = escHtml(text || '');
    var raw = String(query || '').trim();
    if (!raw) return safe;
    var tokens = raw.split(/\s+/).filter(function (t) { return t.length > 0; });
    if (!tokens.length) return safe;
    // Sort longer tokens first to avoid partial overlapping replacements
    tokens.sort(function (a, b) { return b.length - a.length; });
    var pattern = tokens.map(function (t) { return escapeRegex(escHtml(t)); }).join('|');
    if (!pattern) return safe;
    try {
      var re = new RegExp('(' + pattern + ')', 'gi');
      return safe.replace(re, '<mark class="fts-hl">$1</mark>');
    } catch (_) {
      return safe;
    }
  }

  function scoreDoc(doc, query, fuzzy) {
    var raw = String(query || '').trim();
    if (!raw) return 1.0;
    var primaryTerms = raw.toLowerCase().split(/\s+/).filter(Boolean);
    var expandedTerms = tokenizeQuery(raw);

    var title = String(doc.title || '').toLowerCase();
    var body = String(doc.body || doc.description || doc.content || '').toLowerCase();
    var tags = Array.isArray(doc.tags) ? doc.tags.join(' ').toLowerCase() : String(doc.tags || '').toLowerCase();
    var allText = title + ' ' + body + ' ' + tags + ' ' + String(doc.id || '').toLowerCase();
    var asciiWords = allText.match(/[a-z0-9_-]{3,}/g) || [];

    var score = 0;
    var matchedPrimary = 0;

    primaryTerms.forEach(function (term) {
      var hit = false;
      if (title.indexOf(term) !== -1) {
        score += 4.2;
        hit = true;
      }
      if (tags.indexOf(term) !== -1) {
        score += 2.6;
        hit = true;
      }
      if (body.indexOf(term) !== -1) {
        // Count occurrences capped at 4 for BM25-like saturation
        var idx = 0, count = 0;
        while ((idx = body.indexOf(term, idx)) !== -1 && count < 4) {
          count++;
          idx += term.length;
        }
        score += 1.4 + Math.log(1 + count) * 1.1;
        hit = true;
      }
      if (!hit && fuzzy && /^[a-z0-9_-]{3,}$/.test(term)) {
        for (var k = 0; k < asciiWords.length; k++) {
          if (editDistanceOne(term, asciiWords[k])) {
            score += 1.25;
            hit = true;
            break;
          }
        }
      }
      if (hit) matchedPrimary++;
    });

    // Partial CJK bigram boost when exact long phrase didn't match
    if (matchedPrimary === 0 && expandedTerms.length > primaryTerms.length) {
      var biHits = 0;
      expandedTerms.forEach(function (bi) {
        if (primaryTerms.indexOf(bi) !== -1) return;
        if (title.indexOf(bi) !== -1) { score += 1.6; biHits++; }
        else if (body.indexOf(bi) !== -1) { score += 0.8; biHits++; }
      });
      if (biHits === 0) return 0;
    } else if (matchedPrimary === 0) {
      return 0;
    }

    // Slight length normalization
    var norm = 1 / (1 + Math.log10(1 + Math.max(body.length, 40) / 120));
    return Number((score * norm).toFixed(2));
  }

  // --- 1. Standalone SearchStore Mock Data ---

  var now = Date.now();
  var searchIndexes = [
    {
      id: 'idx_docs_kb',
      name: 'docs_kb',
      project_id: null,
      tokenizer: 'jieba',
      segments: 3,
      disk_bytes: 1843200,
      writer_budget_mb: 15,
      created_at: now - 86400000 * 5,
      updated_at: now - 1800000,
      schema: [
        { name: 'id', type: 'keyword', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'title', type: 'text', tokenizer: 'jieba', stored: true, indexed: true, fast: false },
        { name: 'body', type: 'text', tokenizer: 'jieba', stored: true, indexed: true, fast: false },
        { name: 'category', type: 'facet', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'lang', type: 'facet', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'tags', type: 'text', tokenizer: 'simple', stored: true, indexed: true, fast: false }
      ],
      docs: [
        {
          id: 'doc-101',
          title: 'Ferropod 512 MiB 低内存 VPS 部署与调优指南',
          body: '使用 --profile low-mem 启动参数可以限制 Turso 用户库 LRU 句柄数、队列微批通道上限以及 Tantivy IndexWriter 内存预算（15 MiB），让整套 PaaS 稳定运行在 512 MiB 内存内。',
          category: 'ops',
          lang: 'zh',
          tags: ['low-mem', 'vps', 'tantivy', 'turso'],
          updated_at: now - 3600000 * 2
        },
        {
          id: 'doc-102',
          title: 'Turso MVCC 并发事务与内置全文搜索 (FTS)',
          body: 'Ferropod 基于 Turso 0.8 MVCC 引擎，开启 fts feature 后可直接通过 CREATE INDEX ... USING fts 建立倒排索引，与业务表共享快照与事务回滚，避免双写不一致。',
          category: 'architecture',
          lang: 'zh',
          tags: ['turso', 'mvcc', 'fts', 'sqlite'],
          updated_at: now - 3600000 * 5
        },
        {
          id: 'doc-103',
          title: 'Wasmtime WIT Host Bindings Reference',
          body: 'Workers import kv, db, queue, bucket, search, and env interfaces defined in wit/platform.wit. Full-text search queries execute natively on the host with zero Wasm heap overhead.',
          category: 'guide',
          lang: 'en',
          tags: ['wasmtime', 'wit', 'search', 'worker'],
          updated_at: now - 3600000 * 9
        },
        {
          id: 'doc-104',
          title: '使用 tantivy-jieba 实现高质量中文分词与高亮',
          body: '独立 SearchStore 原语通过全局单例 OnceLock<Arc<Jieba>> 共享词典内存，支持 BM25 相关性排序、Snippet 关键词高亮、分面过滤（Facet）和拼写容错查询。',
          category: 'architecture',
          lang: 'zh',
          tags: ['jieba', 'chinese', 'bm25', 'highlight'],
          updated_at: now - 3600000 * 14
        },
        {
          id: 'doc-105',
          title: 'Queue 4-Layer Peak Shaving & Async Indexing',
          body: 'Use Ferropod persistent queues with micro-batch ingestion (200 items / 10ms) to asynchronously sync database mutations into SearchStore indexes without blocking foreground HTTP requests.',
          category: 'guide',
          lang: 'en',
          tags: ['queue', 'backpressure', 'indexing'],
          updated_at: now - 86400000
        },
        {
          id: 'doc-106',
          title: 'Native MCP Server for AI Agents (M3)',
          body: 'Expose search_query, logs_query, and deploy_stack over Streamable HTTP /mcp or stdio so AI agents can inspect knowledge bases, trace errors, and ship safe rollbacks.',
          category: 'guide',
          lang: 'en',
          tags: ['mcp', 'ai', 'agent', 'rmcp'],
          updated_at: now - 86400000 * 2
        }
      ]
    },
    {
      id: 'idx_products_catalog',
      name: 'products_catalog',
      project_id: null,
      tokenizer: 'jieba+ngram',
      segments: 2,
      disk_bytes: 942080,
      writer_budget_mb: 15,
      created_at: now - 86400000 * 3,
      updated_at: now - 600000,
      schema: [
        { name: 'id', type: 'keyword', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'title', type: 'text', tokenizer: 'jieba', stored: true, indexed: true, fast: false },
        { name: 'body', type: 'text', tokenizer: 'jieba', stored: true, indexed: true, fast: false },
        { name: 'category', type: 'facet', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'price_cny', type: 'i64', tokenizer: 'raw', stored: true, indexed: false, fast: true },
        { name: 'tags', type: 'text', tokenizer: 'ngram', stored: true, indexed: true, fast: false }
      ],
      docs: [
        {
          id: 'sku-001',
          title: '机械键盘 Keychron K2 Pro 无线双模红轴',
          body: '支持蓝牙 5.1 与有线 Type-C 双模连接，QMK/VIA 开源改键，PBT 键帽，适合 Rust 开发者日常编码。',
          category: 'hardware',
          price_cny: 599,
          tags: ['keyboard', 'wireless', 'qmk'],
          updated_at: now - 1200000
        },
        {
          id: 'sku-002',
          title: '轻量云主机 VPS 2C 512M NVMe 节点',
          body: '专为自托管单二进制服务打造的超轻量云主机，标配 NVMe SSD 与无限流量出站，完美运行 Ferropod low-mem 配置。',
          category: 'cloud',
          price_cny: 29,
          tags: ['vps', 'nvme', 'self-hosted'],
          updated_at: now - 7200000
        },
        {
          id: 'sku-003',
          title: 'USB-C 100W 氮化镓桌面充电器四口',
          body: '支持 PD 3.1 快充协议，可同时为 MacBook Pro、手机和便携屏供电，体积小巧散热稳定。',
          category: 'hardware',
          price_cny: 189,
          tags: ['gan', 'charger', 'usb-c'],
          updated_at: now - 18000000
        },
        {
          id: 'sku-004',
          title: '对象存储冷备空间 500GB 年度包',
          body: '兼容 S3 协议与 rclone / restic 增量快照，适合配合 ferropod snapshot 做异地灾备归档。',
          category: 'cloud',
          price_cny: 99,
          tags: ['s3', 'backup', 'rclone'],
          updated_at: now - 43200000
        }
      ]
    },
    {
      id: 'idx_code_snippets',
      name: 'code_snippets',
      project_id: null,
      tokenizer: 'ngram',
      segments: 1,
      disk_bytes: 318464,
      writer_budget_mb: 15,
      created_at: now - 86400000 * 2,
      updated_at: now - 3600000,
      schema: [
        { name: 'id', type: 'keyword', tokenizer: 'raw', stored: true, indexed: true, fast: true },
        { name: 'title', type: 'text', tokenizer: 'ngram', stored: true, indexed: true, fast: false },
        { name: 'body', type: 'text', tokenizer: 'ngram', stored: true, indexed: true, fast: false },
        { name: 'category', type: 'facet', tokenizer: 'raw', stored: true, indexed: true, fast: true }
      ],
      docs: [
        {
          id: 'snip-1',
          title: 'CREATE INDEX USING fts (Turso SQL)',
          body: 'CREATE INDEX idx_articles_fts ON articles USING fts (title, body) WITH (tokenizer = \'ngram\');',
          category: 'sql',
          tags: ['turso', 'fts', 'ddl'],
          updated_at: now - 5400000
        },
        {
          id: 'snip-2',
          title: 'Tantivy low-mem IndexWriter snippet',
          body: 'let mut writer = index.writer_with_num_threads(1, 15_000_000)?; // 15 MiB budget for 512 MiB VPS',
          category: 'rust',
          tags: ['tantivy', 'rust', 'memory'],
          updated_at: now - 9600000
        }
      ]
    }
  ];

  function findIndex(id) {
    return searchIndexes.find(function (idx) { return idx.id === id; }) || null;
  }

  function parseScopeProject(scope) {
    if (typeof scope === 'string' && scope.indexOf('project:') === 0) {
      return scope.slice('project:'.length) || null;
    }
    return null;
  }

  function listIndexes(scope) {
    var scopePid = parseScopeProject(scope);
    return searchIndexes
      .filter(function (idx) {
        return !scopePid || idx.project_id === scopePid;
      })
      .map(function (idx) {
        return {
          id: idx.id,
          name: idx.name,
          project_id: idx.project_id,
          tokenizer: idx.tokenizer,
          doc_count: idx.docs.length,
          segments: idx.segments,
          disk_bytes: idx.disk_bytes,
          writer_budget_mb: idx.writer_budget_mb,
          created_at: idx.created_at,
          updated_at: idx.updated_at,
          schema: idx.schema
        };
      });
  }

  function createIndex(opts, scope) {
    if (scope === 'readonly') {
      throw new Error('Readonly token cannot create search indexes');
    }
    var scopePid = parseScopeProject(scope);
    var requestedPid = (opts && opts.project_id) ? String(opts.project_id) : null;
    if (scopePid && requestedPid && requestedPid !== scopePid) {
      throw new Error('Token scope cannot create index for another project');
    }
    var resolvedPid = scopePid || requestedPid || null;

    var name = String(opts.name || '').trim();
    if (!name) throw new Error('Index name is required');
    if (searchIndexes.some(function (i) { return i.name === name && i.project_id === resolvedPid; })) {
      throw new Error('Index "' + name + '" already exists');
    }
    var tok = opts.tokenizer || 'jieba';
    var ts = Date.now();
    var schema = Array.isArray(opts.schema) && opts.schema.length ? opts.schema : [
      { name: 'id', type: 'keyword', tokenizer: 'raw', stored: true, indexed: true, fast: true },
      { name: 'title', type: 'text', tokenizer: tok, stored: true, indexed: true, fast: false },
      { name: 'body', type: 'text', tokenizer: tok, stored: true, indexed: true, fast: false },
      { name: 'category', type: 'facet', tokenizer: 'raw', stored: true, indexed: true, fast: true }
    ];
    var created = {
      id: 'idx_' + name.replace(/[^a-zA-Z0-9_-]/g, '_') + '_' + String(ts).slice(-4),
      name: name,
      project_id: resolvedPid,
      tokenizer: tok,
      segments: 1,
      disk_bytes: 16384,
      writer_budget_mb: 15,
      created_at: ts,
      updated_at: ts,
      schema: schema,
      docs: []
    };
    searchIndexes.unshift(created);
    return created;
  }

  function deleteIndex(id) {
    var i = searchIndexes.findIndex(function (idx) { return idx.id === id; });
    if (i === -1) return false;
    searchIndexes.splice(i, 1);
    return true;
  }

  function listDocs(indexId) {
    var idx = findIndex(indexId);
    return idx ? idx.docs.slice() : [];
  }

  function upsertDoc(indexId, doc) {
    var idx = findIndex(indexId);
    if (!idx) throw new Error('Index not found');
    if (!doc || typeof doc !== 'object') throw new Error('Document must be a JSON object');
    var copy = JSON.parse(JSON.stringify(doc));
    if (!copy.id) copy.id = 'doc-' + String(Date.now()).slice(-6);
    copy.updated_at = Date.now();
    var pos = idx.docs.findIndex(function (d) { return String(d.id) === String(copy.id); });
    if (pos >= 0) {
      idx.docs[pos] = copy;
    } else {
      idx.docs.unshift(copy);
      idx.segments = Math.min(idx.segments + 1, 12);
      idx.disk_bytes += Math.max(512, JSON.stringify(copy).length * 4);
    }
    idx.updated_at = Date.now();
    return copy;
  }

  function deleteDoc(indexId, docId) {
    var idx = findIndex(indexId);
    if (!idx) return false;
    var pos = idx.docs.findIndex(function (d) { return String(d.id) === String(docId); });
    if (pos === -1) return false;
    idx.docs.splice(pos, 1);
    idx.updated_at = Date.now();
    return true;
  }

  function commitIndex(indexId) {
    var idx = findIndex(indexId);
    if (!idx) return null;
    idx.segments = Math.max(1, Math.min(idx.segments, 2));
    idx.updated_at = Date.now();
    return idx;
  }

  function search(indexId, opts) {
    var idx = findIndex(indexId);
    if (!idx) {
      return { hits: [], total: 0, took_ms: 0.4, facets: {}, tokenizer: '-' };
    }
    var query = String((opts && opts.query) || '').trim();
    var fuzzy = Boolean(opts && opts.fuzzy);
    var sort = (opts && opts.sort) || 'relevance';
    var limit = Number((opts && opts.limit) || 20);
    var activeFacets = (opts && opts.facets) || {};

    // Score all docs first
    var scored = [];
    idx.docs.forEach(function (doc) {
      var s = scoreDoc(doc, query, fuzzy);
      if (s > 0) {
        scored.push({ doc: doc, score: s });
      }
    });

    // Build facet counts over query-matched docs
    var facets = {};
    var facetFields = idx.schema
      .filter(function (f) { return f.type === 'facet'; })
      .map(function (f) { return f.name; });

    facetFields.forEach(function (field) {
      var counts = {};
      scored.forEach(function (item) {
        var val = item.doc[field];
        if (val !== undefined && val !== null && val !== '') {
          var key = String(val);
          counts[key] = (counts[key] || 0) + 1;
        }
      });
      if (Object.keys(counts).length) {
        facets[field] = counts;
      }
    });

    // Apply facet filters
    var filtered = scored.filter(function (item) {
      var keys = Object.keys(activeFacets);
      for (var i = 0; i < keys.length; i++) {
        var k = keys[i];
        var expected = activeFacets[k];
        if (expected && String(item.doc[k] || '') !== String(expected)) {
          return false;
        }
      }
      return true;
    });

    if (sort === 'newest') {
      filtered.sort(function (a, b) { return (b.doc.updated_at || 0) - (a.doc.updated_at || 0); });
    } else {
      filtered.sort(function (a, b) {
        if (b.score !== a.score) return b.score - a.score;
        return (b.doc.updated_at || 0) - (a.doc.updated_at || 0);
      });
    }

    var tookMs = Number((0.7 + Math.min(idx.docs.length * 0.14, 2.4) + (fuzzy ? 0.6 : 0)).toFixed(1));
    return {
      hits: filtered.slice(0, limit),
      total: filtered.length,
      took_ms: tookMs,
      facets: facets,
      tokenizer: idx.tokenizer
    };
  }

  // --- 2. User DB Turso FTS Mock Data ---

  var dbFtsStore = {};

  function ensureDbFts(dbId) {
    if (!dbFtsStore[dbId]) {
      dbFtsStore[dbId] = {
        indexes: [
          {
            name: 'idx_articles_fts',
            table: 'articles',
            columns: ['title', 'body'],
            tokenizer: 'ngram',
            rows_indexed: 128,
            size_bytes: 214528,
            created_at: now - 86400000 * 2
          },
          {
            name: 'idx_notes_fts',
            table: 'notes',
            columns: ['content'],
            tokenizer: 'simple',
            rows_indexed: 46,
            size_bytes: 53248,
            created_at: now - 86400000
          }
        ],
        sampleRows: {
          articles: [
            { id: 1, title: 'Turso MVCC 并发写入原理解析', body: 'Turso 通过 BEGIN CONCURRENT 与多版本快照实现无锁读与高并发写入，天然适合单节点 PaaS 场景。', updated_at: '2026-11-20 18:10:00' },
            { id: 2, title: '在 Ferropod 上部署静态站点与 Worker', body: '通过 ferropod deploy 推送静态资源到内容寻址 Blob，配合 ETag 与 Range 实现零拷贝流式分发。', updated_at: '2026-11-20 21:35:00' },
            { id: 3, title: 'Full-Text Search with CREATE INDEX USING fts', body: 'Turso 0.8 embeds Tantivy segments inside B-tree backing tables so FTS indexes stay consistent with SQL transactions.', updated_at: '2026-11-21 02:15:00' },
            { id: 4, title: '使用 Ngram 分词实现中英文混合检索', body: '当 SQL 内置 FTS 没有加载外部词典时，配置 tokenizer = ngram 可以直接支持中文短语与英文子串匹配。', updated_at: '2026-11-21 04:40:00' }
          ],
          notes: [
            { id: 10, content: 'Remember to benchmark --profile low-mem RSS with 15 MiB Tantivy writer buffer.', updated_at: '2026-11-20 15:00:00' },
            { id: 11, content: 'Daily logs rotate automatically and expire via fs::remove_file after 7 days.', updated_at: '2026-11-21 01:20:00' }
          ]
        }
      };
    }
    return dbFtsStore[dbId];
  }

  function listDbFtsIndexes(dbId) {
    return ensureDbFts(dbId).indexes.slice();
  }

  function buildCreateFtsSql(idx) {
    var cols = (idx.columns || []).join(', ');
    return 'CREATE INDEX ' + idx.name + ' ON ' + idx.table + ' USING fts (' + cols + ") WITH (tokenizer = '" + idx.tokenizer + "');";
  }

  function createDbFtsIndex(dbId, opts) {
    var store = ensureDbFts(dbId);
    var name = String(opts.name || '').trim();
    var table = String(opts.table || '').trim();
    var cols = String(opts.columns || '')
      .split(',')
      .map(function (c) { return c.trim(); })
      .filter(Boolean);
    if (!name || !table || !cols.length) {
      throw new Error('Index name, table, and at least one column are required');
    }
    if (store.indexes.some(function (i) { return i.name === name; })) {
      throw new Error('FTS index "' + name + '" already exists');
    }
    var item = {
      name: name,
      table: table,
      columns: cols,
      tokenizer: opts.tokenizer || 'ngram',
      rows_indexed: (store.sampleRows[table] || []).length,
      size_bytes: 16384,
      created_at: Date.now()
    };
    store.indexes.unshift(item);
    return item;
  }

  function dropDbFtsIndex(dbId, indexName) {
    var store = ensureDbFts(dbId);
    var i = store.indexes.findIndex(function (idx) { return idx.name === indexName; });
    if (i === -1) return false;
    store.indexes.splice(i, 1);
    return true;
  }

  function queryDbFts(dbId, indexName, query) {
    var store = ensureDbFts(dbId);
    var idx = store.indexes.find(function (i) { return i.name === indexName; }) || store.indexes[0];
    if (!idx) return { rows: [], sql: '', took_ms: 0.3 };
    var sourceRows = store.sampleRows[idx.table] || store.sampleRows.articles || [];
    var q = String(query || '').trim();
    var hits = [];
    sourceRows.forEach(function (r) {
      var docLike = {
        id: r.id,
        title: r.title || '',
        body: r.body || r.content || ''
      };
      var s = scoreDoc(docLike, q, false);
      if (s > 0) {
        hits.push({ row: r, score: s });
      }
    });
    hits.sort(function (a, b) { return b.score - a.score; });
    var safeQ = q.replace(/'/g, "''");
    var sql = q
      ? 'SELECT *, fts_score(' + idx.columns.join(', ') + ", '" + safeQ + "') AS score\nFROM " + idx.table +
        '\nWHERE fts_match(' + idx.columns.join(', ') + ", '" + safeQ + "')\nORDER BY score DESC\nLIMIT 20;"
      : 'SELECT * FROM ' + idx.table + ' LIMIT 20;';
    return {
      index: idx,
      rows: hits,
      sql: sql,
      took_ms: Number((0.9 + hits.length * 0.2).toFixed(1))
    };
  }

  // --- 3. Platform Daily Logs FTS Mock Data ---

  var mockLogEntries = [
    {
      ts: now - 120000,
      level: 'info',
      source: 'access',
      project_id: null,
      request_id: 'req_9f41a82c',
      message: 'GET /_api/v1/search/docs_kb/query?q=turso+mvcc 200 OK (1.8ms)',
      meta_json: '{"method":"GET","status":200,"latency_ms":1.8,"index":"docs_kb"}'
    },
    {
      ts: now - 280000,
      level: 'warn',
      source: 'runtime',
      project_id: null,
      request_id: 'req_8d10b4e1',
      message: 'Tantivy IndexWriter memory budget reached 14.2 MiB / 15 MiB (low-mem); triggering segment flush',
      meta_json: '{"profile":"low-mem","writer_mb":14.2,"budget_mb":15}'
    },
    {
      ts: now - 490000,
      level: 'error',
      source: 'queue',
      project_id: null,
      request_id: 'req_7c32e9a0',
      message: 'Job #4821 异步索引同步失败: upstream webhook timeout after 5000ms，已进入指数退避重试',
      meta_json: '{"queue":"search-sync","job_id":4821,"attempt":2,"next_backoff_secs":30}'
    },
    {
      ts: now - 820000,
      level: 'info',
      source: 'system',
      project_id: null,
      request_id: '',
      message: 'Daily log FTS index rotated: created /data/logs/fts_2026_11_21 (IndexRecordOption::Basic)',
      meta_json: '{"day":"2026_11_21","record_option":"basic","retention_days":7}'
    },
    {
      ts: now - 1540000,
      level: 'info',
      source: 'audit',
      project_id: null,
      request_id: 'req_6a02c11f',
      message: '管理员创建全文搜索索引 idx_products_catalog (tokenizer=jieba+ngram)',
      meta_json: '{"actor":"admin","action":"search.index.create","index":"products_catalog"}'
    },
    {
      ts: now - 2400000,
      level: 'warn',
      source: 'access',
      project_id: null,
      request_id: 'req_5e90d32b',
      message: 'POST /_api/v1/queues/ingest/messages returned 429 Too Many Requests (mpsc channel saturated 4096/4096)',
      meta_json: '{"status":429,"queue":"ingest","depth":4096}'
    },
    {
      ts: now - 3900000,
      level: 'info',
      source: 'cron',
      project_id: null,
      request_id: '',
      message: 'Maintenance tick completed: cleaned 142 expired KV keys, 3 orphan blobs, 1 expired log day',
      meta_json: '{"kv_deleted":142,"blobs_gc":3,"logs_dropped":1}'
    },
    {
      ts: now - 6200000,
      level: 'error',
      source: 'runtime',
      project_id: null,
      request_id: 'req_4b81f77a',
      message: 'Turso MVCC write conflict on dbs/app_main.db (BEGIN CONCURRENT), succeeded on retry #2',
      meta_json: '{"db":"app_main","retries":2,"duration_ms":6.4}'
    },
    {
      ts: now - 9800000,
      level: 'debug',
      source: 'system',
      project_id: null,
      request_id: '',
      message: 'LRU IndexReader cache hit ratio 96.4% (open_readers=3/6, profile=low-mem)',
      meta_json: '{"hit_ratio":0.964,"open_readers":3,"max_readers":6}'
    }
  ];

  function queryMockLogs(filters, scope) {
    var f = filters || {};
    var scopePid = parseScopeProject(scope);
    var effectivePid = scopePid || f.project_id || '';
    var q = String(f.q || '').trim();
    var since = f.windowSecs ? (Date.now() - Number(f.windowSecs) * 1000) : 0;
    var limit = Number(f.limit || 100);

    var matched = mockLogEntries.filter(function (e) {
      if (f.source && e.source !== f.source) return false;
      if (f.level && e.level !== f.level) return false;
      if (effectivePid && e.project_id !== effectivePid) return false;
      if (f.request_id && (!e.request_id || e.request_id.toLowerCase().indexOf(f.request_id.toLowerCase()) === -1)) return false;
      if (since && e.ts < since) return false;
      if (q) {
        var score = scoreDoc({ id: e.request_id, title: e.message, body: e.meta_json || '' }, q, false);
        if (score <= 0) return false;
      }
      return true;
    });

    return {
      entries: matched.slice(0, limit),
      total: matched.length,
      took_ms: Number((0.8 + matched.length * 0.15).toFixed(1)),
      index_name: 'fts_2026_11_21'
    };
  }

  window.FerroSearchMock = {
    highlightText: highlightText,
    listIndexes: listIndexes,
    createIndex: createIndex,
    deleteIndex: deleteIndex,
    listDocs: listDocs,
    upsertDoc: upsertDoc,
    deleteDoc: deleteDoc,
    commitIndex: commitIndex,
    search: search,
    listDbFtsIndexes: listDbFtsIndexes,
    buildCreateFtsSql: buildCreateFtsSql,
    createDbFtsIndex: createDbFtsIndex,
    dropDbFtsIndex: dropDbFtsIndex,
    queryDbFts: queryDbFts,
    queryMockLogs: queryMockLogs
  };
})();
