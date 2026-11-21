use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};
use uuid::Uuid;

use crate::core::model::{
    FerroError, Result, SearchFieldSchema, SearchHit, SearchIndexMeta, SearchQueryInput,
    SearchQueryResult,
};
use crate::core::traits::SearchStore;
use crate::storage::core_store::{opt_text, opt_text_param};
use crate::storage::tx::{
    checkpoint_db, connect_db, copy_db_with_sidecars, map_turso_err, open_turso_db, with_retry,
};

const SEARCH_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS search_indexes (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    project_id TEXT,
    tokenizer TEXT NOT NULL DEFAULT 'jieba+ngram',
    segments INTEGER NOT NULL DEFAULT 1,
    disk_bytes INTEGER NOT NULL DEFAULT 16384,
    writer_budget_mb INTEGER NOT NULL DEFAULT 15,
    schema_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_search_name_proj
    ON search_indexes (name, COALESCE(project_id, ''));

CREATE TABLE IF NOT EXISTS search_docs (
    index_id TEXT NOT NULL,
    doc_id TEXT NOT NULL,
    doc_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (index_id, doc_id)
);
CREATE INDEX IF NOT EXISTS idx_search_docs_updated
    ON search_docs (index_id, updated_at DESC);
"#;

/// Turso-backed persistent search index and document store (`search.db`) using bounded-memory
/// BM25-like + CJK bigram + optional Levenshtein fuzzy scoring.
#[derive(Clone)]
pub struct TursoSearchStore {
    db: Database,
    db_path: PathBuf,
}

impl TursoSearchStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = open_turso_db(path).await?;
        let conn = connect_db(&db)?;
        conn.execute_batch(SEARCH_SCHEMA)
            .await
            .map_err(map_turso_err)?;
        Ok(Self {
            db,
            db_path: path.to_path_buf(),
        })
    }

    fn default_schema(tokenizer: &str) -> Vec<SearchFieldSchema> {
        vec![
            SearchFieldSchema {
                name: "id".into(),
                field_type: "keyword".into(),
                tokenizer: "raw".into(),
                stored: true,
                indexed: true,
                fast: true,
            },
            SearchFieldSchema {
                name: "title".into(),
                field_type: "text".into(),
                tokenizer: tokenizer.into(),
                stored: true,
                indexed: true,
                fast: false,
            },
            SearchFieldSchema {
                name: "body".into(),
                field_type: "text".into(),
                tokenizer: tokenizer.into(),
                stored: true,
                indexed: true,
                fast: false,
            },
            SearchFieldSchema {
                name: "category".into(),
                field_type: "facet".into(),
                tokenizer: "raw".into(),
                stored: true,
                indexed: true,
                fast: true,
            },
        ]
    }

    fn row_to_index_meta(row: &turso::Row) -> Result<SearchIndexMeta> {
        let schema_raw: String = row.get(7).map_err(map_turso_err)?;
        let schema: Vec<SearchFieldSchema> = serde_json::from_str(&schema_raw)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode search schema_json: {e}")))?;
        Ok(SearchIndexMeta {
            id: row.get(0).map_err(map_turso_err)?,
            name: row.get(1).map_err(map_turso_err)?,
            project_id: opt_text(row, 2)?,
            tokenizer: row.get(3).map_err(map_turso_err)?,
            segments: row.get::<i64>(4).map_err(map_turso_err)?.max(1) as usize,
            disk_bytes: row.get::<i64>(5).map_err(map_turso_err)?.max(0) as u64,
            writer_budget_mb: row.get::<i64>(6).map_err(map_turso_err)?.max(1) as usize,
            schema,
            created_at: row.get(8).map_err(map_turso_err)?,
            updated_at: row.get(9).map_err(map_turso_err)?,
            doc_count: row.get::<i64>(10).map_err(map_turso_err)?.max(0) as usize,
        })
    }
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

fn extract_cjk_bigrams(term: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut run: Vec<char> = Vec::new();
    for ch in term.chars() {
        if is_cjk(ch) {
            run.push(ch);
        } else {
            if run.len() >= 2 {
                for w in run.windows(2) {
                    out.push(w.iter().collect());
                }
            }
            run.clear();
        }
    }
    if run.len() >= 2 {
        for w in run.windows(2) {
            out.push(w.iter().collect());
        }
    }
    out
}

fn edit_distance_one(a: &str, b: &str) -> bool {
    let ab: Vec<char> = a.chars().collect();
    let bb: Vec<char> = b.chars().collect();
    if ab.len().abs_diff(bb.len()) > 1 {
        return false;
    }
    if ab == bb {
        return true;
    }
    let (mut i, mut j, mut diffs) = (0usize, 0usize, 0usize);
    while i < ab.len() && j < bb.len() {
        if ab[i] == bb[j] {
            i += 1;
            j += 1;
        } else {
            diffs += 1;
            if diffs > 1 {
                return false;
            }
            if ab.len() > bb.len() {
                i += 1;
            } else if bb.len() > ab.len() {
                j += 1;
            } else {
                i += 1;
                j += 1;
            }
        }
    }
    if i < ab.len() || j < bb.len() {
        diffs += 1;
    }
    diffs <= 1
}

fn extract_ascii_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            cur.push(ch);
        } else {
            if cur.len() >= 3 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.len() >= 3 {
        out.push(cur);
    }
    out
}

fn flatten_json_text(val: &serde_json::Value, out: &mut String) {
    match val {
        serde_json::Value::String(s) => {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(s);
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                flatten_json_text(v, out);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "updated_at" || k == "created_at" {
                    continue;
                }
                flatten_json_text(v, out);
            }
        }
        serde_json::Value::Number(n) => {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&n.to_string());
        }
        serde_json::Value::Bool(b) => {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(if *b { "true" } else { "false" });
        }
        serde_json::Value::Null => {}
    }
}

/// Compute BM25-like + CJK bigram + optional Levenshtein-1 score for a JSON document or row.
pub fn score_json_document(doc: &serde_json::Value, query: &str, fuzzy: bool) -> f64 {
    let raw = query.trim();
    if raw.is_empty() {
        return 1.0;
    }
    let primary_terms: Vec<String> = raw
        .split_whitespace()
        .map(|s| s.to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    if primary_terms.is_empty() {
        return 1.0;
    }

    let title = doc
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    let tags = match doc.get("tags") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase(),
        Some(serde_json::Value::String(s)) => s.to_lowercase(),
        _ => String::new(),
    };
    let mut body = doc
        .get("body")
        .or_else(|| doc.get("description"))
        .or_else(|| doc.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();

    let mut fallback = String::new();
    flatten_json_text(doc, &mut fallback);
    let fallback_lower = fallback.to_lowercase();
    if body.is_empty() {
        body = fallback_lower.clone();
    }

    let ascii_words = extract_ascii_words(&fallback_lower);
    let mut score = 0.0f64;
    let mut matched_primary = 0usize;

    for term in &primary_terms {
        let mut hit = false;
        if title.contains(term.as_str()) {
            score += 4.2;
            hit = true;
        }
        if !tags.is_empty() && tags.contains(term.as_str()) {
            score += 2.6;
            hit = true;
        }
        if body.contains(term.as_str()) {
            let count = body.matches(term.as_str()).take(4).count();
            score += 1.4 + (1.0 + count as f64).ln() * 1.1;
            hit = true;
        } else if fallback_lower.contains(term.as_str()) {
            score += 1.2;
            hit = true;
        }
        if !hit
            && fuzzy
            && term.len() >= 3
            && term
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && ascii_words.iter().any(|w| edit_distance_one(term, w))
        {
            score += 1.25;
            hit = true;
        }
        if hit {
            matched_primary += 1;
        }
    }

    if matched_primary == 0 {
        let mut bigrams = Vec::new();
        for term in &primary_terms {
            for bi in extract_cjk_bigrams(term) {
                if !bigrams.contains(&bi) && !primary_terms.contains(&bi) {
                    bigrams.push(bi);
                }
            }
        }
        let mut bi_hits = 0usize;
        for bi in &bigrams {
            if title.contains(bi.as_str()) {
                score += 1.6;
                bi_hits += 1;
            } else if body.contains(bi.as_str()) || fallback_lower.contains(bi.as_str()) {
                score += 0.8;
                bi_hits += 1;
            }
        }
        if bi_hits == 0 {
            return 0.0;
        }
    }

    let norm = 1.0 / (1.0 + (1.0 + (body.len().max(40) as f64) / 120.0).log10());
    ((score * norm) * 100.0).round() / 100.0
}

#[async_trait]
impl SearchStore for TursoSearchStore {
    async fn create_index(
        &self,
        name: &str,
        project_id: Option<&str>,
        tokenizer: &str,
        schema: Option<Vec<SearchFieldSchema>>,
    ) -> Result<SearchIndexMeta> {
        let now = Utc::now().timestamp_millis();
        let clean_slug: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let id = format!(
            "idx_{}_{}",
            clean_slug,
            &Uuid::new_v4().simple().to_string()[..6]
        );
        let schema_vec = schema.unwrap_or_else(|| Self::default_schema(tokenizer));
        let schema_json = serde_json::to_string(&schema_vec)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("encode search schema: {e}")))?;

        let meta = SearchIndexMeta {
            id: id.clone(),
            name: name.to_string(),
            project_id: project_id.map(str::to_string),
            tokenizer: tokenizer.to_string(),
            doc_count: 0,
            segments: 1,
            disk_bytes: 16384,
            writer_budget_mb: 15,
            created_at: now,
            updated_at: now,
            schema: schema_vec,
        };

        let m = meta.clone();
        with_retry(|| {
            let db = self.db.clone();
            let m = m.clone();
            let schema_json = schema_json.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO search_indexes (id, name, project_id, tokenizer, segments, disk_bytes, writer_budget_mb, schema_json, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    vec![
                        TursoValue::Text(m.id),
                        TursoValue::Text(m.name),
                        opt_text_param(m.project_id),
                        TursoValue::Text(m.tokenizer),
                        TursoValue::Integer(m.segments as i64),
                        TursoValue::Integer(m.disk_bytes as i64),
                        TursoValue::Integer(m.writer_budget_mb as i64),
                        TursoValue::Text(schema_json),
                        TursoValue::Integer(m.created_at),
                        TursoValue::Integer(m.updated_at),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(meta)
    }

    async fn get_index(&self, id_or_name: &str) -> Result<Option<SearchIndexMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT s.id, s.name, s.project_id, s.tokenizer, s.segments, s.disk_bytes, s.writer_budget_mb,
                        s.schema_json, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM search_docs d WHERE d.index_id = s.id)
                 FROM search_indexes s
                 WHERE s.id = ?1 OR s.name = ?1
                 ORDER BY CASE WHEN s.id = ?1 THEN 0 ELSE 1 END
                 LIMIT 1",
                [id_or_name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(Self::row_to_index_meta(&row)?))
        } else {
            Ok(None)
        }
    }

    async fn list_indexes(&self) -> Result<Vec<SearchIndexMeta>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT s.id, s.name, s.project_id, s.tokenizer, s.segments, s.disk_bytes, s.writer_budget_mb,
                        s.schema_json, s.created_at, s.updated_at,
                        (SELECT COUNT(*) FROM search_docs d WHERE d.index_id = s.id)
                 FROM search_indexes s
                 ORDER BY s.updated_at DESC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(Self::row_to_index_meta(&row)?);
        }
        Ok(out)
    }

    async fn delete_index(&self, id: &str) -> Result<()> {
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                tx.execute(
                    "DELETE FROM search_docs WHERE index_id = ?1",
                    [id_s.as_str()],
                )
                .await?;
                tx.execute("DELETE FROM search_indexes WHERE id = ?1", [id_s.as_str()])
                    .await?;
                tx.commit().await
            }
        })
        .await?;
        Ok(())
    }

    async fn commit_index(&self, id: &str) -> Result<SearchIndexMeta> {
        let now = Utc::now().timestamp_millis();
        let id_s = id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let id_s = id_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE search_indexes
                     SET segments = CASE WHEN segments > 2 THEN 2 ELSE 1 END, updated_at = ?1
                     WHERE id = ?2",
                    (now, id_s),
                )
                .await
            }
        })
        .await?;
        self.get_index(id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("search index '{id}'")))
    }

    async fn list_docs(&self, index_id: &str) -> Result<Vec<serde_json::Value>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT doc_json FROM search_docs WHERE index_id = ?1 ORDER BY updated_at DESC",
                [index_id],
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let raw: String = row.get(0).map_err(map_turso_err)?;
            let doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode search doc: {e}")))?;
            out.push(doc);
        }
        Ok(out)
    }

    async fn list_docs_page(
        &self,
        index_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let limit = limit.clamp(1, 200) as i64;
        let offset = i64::try_from(offset)
            .map_err(|_| FerroError::InvalidArgument("offset is too large".into()))?;
        let conn = connect_db(&self.db)?;
        let mut rows = conn.query(
            "SELECT doc_json FROM search_docs WHERE index_id = ?1 ORDER BY updated_at DESC, doc_id ASC LIMIT ?2 OFFSET ?3",
            (index_id, limit, offset),
        ).await.map_err(map_turso_err)?;
        let mut docs = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let raw: String = row.get(0).map_err(map_turso_err)?;
            docs.push(
                serde_json::from_str(&raw)
                    .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode search doc: {e}")))?,
            );
        }
        Ok(docs)
    }

    async fn upsert_doc(
        &self,
        index_id: &str,
        mut doc: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let Some(obj) = doc.as_object_mut() else {
            return Err(FerroError::InvalidArgument(
                "document must be a JSON object".into(),
            ));
        };
        let doc_id = match obj.get("id") {
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            Some(v) if !v.is_null() => v.to_string(),
            _ => {
                let generated = format!("doc-{}", &Uuid::new_v4().simple().to_string()[..8]);
                obj.insert("id".into(), serde_json::Value::String(generated.clone()));
                generated
            }
        };
        let now = Utc::now().timestamp_millis();
        obj.insert("updated_at".into(), serde_json::json!(now));
        let doc_json = serde_json::to_string(&doc)
            .map_err(|e| FerroError::Internal(anyhow::anyhow!("encode search doc: {e}")))?;
        let added_bytes = (doc_json.len() * 4).max(512) as i64;
        let idx_s = index_id.to_string();
        let doc_id_s = doc_id.clone();
        let json_s = doc_json;

        with_retry(|| {
            let db = self.db.clone();
            let idx_s = idx_s.clone();
            let doc_id_s = doc_id_s.clone();
            let json_s = json_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                tx.execute(
                    "INSERT INTO search_docs (index_id, doc_id, doc_json, updated_at)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(index_id, doc_id) DO UPDATE SET
                        doc_json = excluded.doc_json,
                        updated_at = excluded.updated_at",
                    (idx_s.as_str(), doc_id_s.as_str(), json_s.as_str(), now),
                )
                .await?;
                tx.execute(
                    "UPDATE search_indexes
                     SET segments = MIN(segments + 1, 12),
                         disk_bytes = disk_bytes + ?1,
                         updated_at = ?2
                     WHERE id = ?3",
                    (added_bytes, now, idx_s.as_str()),
                )
                .await?;
                tx.commit().await
            }
        })
        .await?;
        Ok(doc)
    }

    async fn delete_doc(&self, index_id: &str, doc_id: &str) -> Result<bool> {
        let now = Utc::now().timestamp_millis();
        let idx_s = index_id.to_string();
        let doc_s = doc_id.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let idx_s = idx_s.clone();
            let doc_s = doc_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let affected = tx
                    .execute(
                        "DELETE FROM search_docs WHERE index_id = ?1 AND doc_id = ?2",
                        (idx_s.as_str(), doc_s.as_str()),
                    )
                    .await?;
                if affected > 0 {
                    tx.execute(
                        "UPDATE search_indexes SET updated_at = ?1 WHERE id = ?2",
                        (now, idx_s.as_str()),
                    )
                    .await?;
                }
                tx.commit().await?;
                Ok(affected > 0)
            }
        })
        .await
    }

    async fn search(&self, index_id: &str, input: SearchQueryInput) -> Result<SearchQueryResult> {
        let started = Instant::now();
        let meta = self
            .get_index(index_id)
            .await?
            .ok_or_else(|| FerroError::NotFound(format!("search index '{index_id}'")))?;
        let limit = input.limit.unwrap_or(20).clamp(1, 200);
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT doc_json FROM search_docs WHERE index_id = ?1 ORDER BY updated_at DESC",
                [meta.id.as_str()],
            )
            .await
            .map_err(map_turso_err)?;
        let mut facets: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut hits: Vec<SearchHit> = Vec::new();
        let mut total = 0usize;

        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let raw: String = row.get(0).map_err(map_turso_err)?;
            let doc: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| FerroError::Internal(anyhow::anyhow!("decode search doc: {e}")))?;
            let score = score_json_document(&doc, &input.query, input.fuzzy);
            if score <= 0.0 {
                continue;
            }
            for field in meta
                .schema
                .iter()
                .filter(|field| field.field_type == "facet")
            {
                let key = match doc.get(&field.name) {
                    Some(serde_json::Value::String(value)) if !value.is_empty() => {
                        Some(value.clone())
                    }
                    Some(serde_json::Value::Number(value)) => Some(value.to_string()),
                    Some(serde_json::Value::Bool(value)) => Some(value.to_string()),
                    _ => None,
                };
                if let Some(key) = key {
                    *facets
                        .entry(field.name.clone())
                        .or_default()
                        .entry(key)
                        .or_default() += 1;
                }
            }
            let matches = input.facets.iter().all(|(key, expected)| {
                if expected.is_empty() {
                    return true;
                }
                match doc.get(key) {
                    Some(serde_json::Value::String(value)) => value == expected,
                    Some(serde_json::Value::Number(value)) => value.to_string() == *expected,
                    Some(serde_json::Value::Bool(value)) => value.to_string() == *expected,
                    _ => false,
                }
            });
            if !matches {
                continue;
            }
            total += 1;
            hits.push(SearchHit { doc, score });
            hits.sort_by(|a, b| {
                let updated = || {
                    let a_time = a
                        .doc
                        .get("updated_at")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    let b_time = b
                        .doc
                        .get("updated_at")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    b_time.cmp(&a_time)
                };
                if input.sort.as_deref() == Some("updated_desc") {
                    updated()
                } else {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(updated)
                }
            });
            hits.truncate(limit);
        }

        Ok(SearchQueryResult {
            hits,
            total,
            took_ms: ((started.elapsed().as_secs_f64() * 1000.0) * 10.0).round() / 10.0,
            facets,
            tokenizer: meta.tokenizer,
        })
    }

    async fn checkpoint(&self) -> Result<()> {
        checkpoint_db(&self.db).await;
        Ok(())
    }

    async fn snapshot_to(&self, dest_path: &Path) -> Result<()> {
        checkpoint_db(&self.db).await;
        copy_db_with_sidecars(&self.db_path, dest_path).await
    }
}
