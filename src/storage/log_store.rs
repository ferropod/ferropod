use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{NaiveDate, TimeZone, Utc};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};

use crate::core::model::{FerroError, LogEntry, LogQuery, Result};
use crate::core::traits::LogStore;
use crate::storage::core_store::escape_like;
use crate::storage::tx::{checkpoint_db, connect_db, map_turso_err, open_turso_db};

const RECENT_CACHE_CAP: usize = 2048;

const LOG_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    project_id TEXT,
    request_id TEXT,
    source TEXT NOT NULL,
    level TEXT NOT NULL,
    message TEXT NOT NULL,
    meta_json TEXT
);
CREATE INDEX IF NOT EXISTS idx_logs_ts ON logs (ts DESC);
CREATE INDEX IF NOT EXISTS idx_logs_project_ts ON logs (project_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_logs_req ON logs (request_id);
"#;

enum LogCmd {
    Entry(LogEntry),
    Flush(oneshot::Sender<()>),
}

type DayDbCache = Arc<Mutex<HashMap<NaiveDate, Database>>>;
type RecentLogCache = Arc<RwLock<VecDeque<LogEntry>>>;

/// Non-blocking daily rotating Turso log store with bounded mpsc, hot in-memory ring buffer, and SSE broadcast fan-out.
#[derive(Clone)]
pub struct DailyLogStore {
    logs_dir: PathBuf,
    cmd_tx: mpsc::Sender<LogCmd>,
    broadcast_tx: broadcast::Sender<LogEntry>,
    day_dbs: DayDbCache,
    recent_cache: RecentLogCache,
    disk_has_more: Arc<AtomicBool>,
    last_warn_sec: Arc<AtomicI64>,
    dropped_count: Arc<AtomicU64>,
}

impl DailyLogStore {
    pub async fn open(logs_dir: PathBuf, channel_capacity: usize) -> Result<Self> {
        tokio::fs::create_dir_all(&logs_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("failed to create logs dir: {e}")))?;

        let mut existing_files = false;
        if let Ok(mut rd) = tokio::fs::read_dir(&logs_dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if let Some(name) = entry.path().file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("logs_") && name.ends_with(".db") {
                        existing_files = true;
                        break;
                    }
                }
            }
        }

        let (cmd_tx, cmd_rx) = mpsc::channel::<LogCmd>(channel_capacity.max(128));
        let (broadcast_tx, _) = broadcast::channel::<LogEntry>(512);
        let day_dbs: DayDbCache = Arc::new(Mutex::new(HashMap::new()));
        let recent_cache: RecentLogCache =
            Arc::new(RwLock::new(VecDeque::with_capacity(RECENT_CACHE_CAP)));
        let disk_has_more = Arc::new(AtomicBool::new(existing_files));

        tokio::spawn(run_log_writer(
            logs_dir.clone(),
            day_dbs.clone(),
            recent_cache.clone(),
            disk_has_more.clone(),
            cmd_rx,
        ));

        Ok(Self {
            logs_dir,
            cmd_tx,
            broadcast_tx,
            day_dbs,
            recent_cache,
            disk_has_more,
            last_warn_sec: Arc::new(AtomicI64::new(0)),
            dropped_count: Arc::new(AtomicU64::new(0)),
        })
    }

    fn day_file_path(logs_dir: &Path, day: NaiveDate) -> PathBuf {
        logs_dir.join(format!("logs_{}.db", day.format("%Y_%m_%d")))
    }

    fn parse_day_from_path(path: &Path) -> Option<NaiveDate> {
        let name = path.file_name()?.to_str()?;
        let date_str = name.strip_prefix("logs_")?.strip_suffix(".db")?;
        NaiveDate::parse_from_str(date_str, "%Y_%m_%d").ok()
    }
}

fn matches_log_query(e: &LogEntry, q: &LogQuery, terms_lower: &[String]) -> bool {
    if let Some(pid) = &q.project_id {
        if e.project_id.as_deref() != Some(pid.as_str()) {
            return false;
        }
    }
    if let Some(rid) = &q.request_id {
        if e.request_id.as_deref() != Some(rid.as_str()) {
            return false;
        }
    }
    if let Some(src) = &q.source {
        if &e.source != src {
            return false;
        }
    }
    if let Some(lvl) = &q.level {
        if &e.level != lvl {
            return false;
        }
    }
    if let Some(since) = q.since_ts {
        if e.ts < since {
            return false;
        }
    }
    if !terms_lower.is_empty() {
        let msg_low = e.message.to_lowercase();
        let meta_low = e.meta_json.as_deref().unwrap_or("").to_lowercase();
        let req_low = e.request_id.as_deref().unwrap_or("").to_lowercase();
        for term in terms_lower {
            if !msg_low.contains(term) && !meta_low.contains(term) && !req_low.contains(term) {
                return false;
            }
        }
    }
    true
}

async fn open_day_db(logs_dir: &Path, day: NaiveDate) -> Result<Database> {
    let path = DailyLogStore::day_file_path(logs_dir, day);
    let db = open_turso_db(&path).await?;
    let conn = connect_db(&db)?;
    conn.execute_batch(LOG_SCHEMA)
        .await
        .map_err(map_turso_err)?;
    Ok(db)
}

async fn get_or_open_day_db(
    logs_dir: &Path,
    day_dbs: &DayDbCache,
    day: NaiveDate,
) -> Result<Database> {
    let mut guard = day_dbs.lock().await;
    if let Some(db) = guard.get(&day) {
        return Ok(db.clone());
    }
    if guard.len() >= 8 {
        if let Some(&oldest_day) = guard.keys().min() {
            if let Some(old_db) = guard.remove(&oldest_day) {
                checkpoint_db(&old_db).await;
            }
        }
    }
    let db = open_day_db(logs_dir, day).await?;
    guard.insert(day, db.clone());
    Ok(db)
}

async fn run_log_writer(
    logs_dir: PathBuf,
    day_dbs: DayDbCache,
    recent_cache: RecentLogCache,
    disk_has_more: Arc<AtomicBool>,
    mut rx: mpsc::Receiver<LogCmd>,
) {
    const MAX_BATCH: usize = 1024;
    const CHECKPOINT_EVERY: usize = 5_000;
    const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

    let mut current_day: Option<(NaiveDate, Database)> = None;
    let mut buffer: Vec<LogEntry> = Vec::with_capacity(MAX_BATCH);
    let mut written_since_checkpoint: usize = 0;
    let mut next_synthetic_id: i64 = 1;
    let mut tick = tokio::time::interval(FLUSH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_cmd = rx.recv() => {
                match maybe_cmd {
                    Some(LogCmd::Entry(entry)) => {
                        buffer.push(entry);
                        let mut pending_acks = Vec::new();
                        while buffer.len() < MAX_BATCH {
                            match rx.try_recv() {
                                Ok(LogCmd::Entry(e)) => buffer.push(e),
                                Ok(LogCmd::Flush(ack)) => pending_acks.push(ack),
                                Err(_) => break,
                            }
                        }
                        if buffer.len() >= MAX_BATCH || !pending_acks.is_empty() {
                            flush_entries(
                                &logs_dir,
                                &day_dbs,
                                &recent_cache,
                                &disk_has_more,
                                &mut next_synthetic_id,
                                &mut current_day,
                                &mut buffer,
                                &mut written_since_checkpoint,
                                CHECKPOINT_EVERY,
                            )
                            .await;
                            for ack in pending_acks {
                                let _ = ack.send(());
                            }
                        }
                    }
                    Some(LogCmd::Flush(ack)) => {
                        let mut pending_acks = vec![ack];
                        while let Ok(cmd) = rx.try_recv() {
                            match cmd {
                                LogCmd::Entry(entry) => {
                                    buffer.push(entry);
                                    if buffer.len() >= MAX_BATCH {
                                        flush_entries(
                                            &logs_dir,
                                            &day_dbs,
                                            &recent_cache,
                                            &disk_has_more,
                                            &mut next_synthetic_id,
                                            &mut current_day,
                                            &mut buffer,
                                            &mut written_since_checkpoint,
                                            CHECKPOINT_EVERY,
                                        )
                                        .await;
                                    }
                                }
                                LogCmd::Flush(other_ack) => pending_acks.push(other_ack),
                            }
                        }
                        flush_entries(
                            &logs_dir,
                            &day_dbs,
                            &recent_cache,
                            &disk_has_more,
                            &mut next_synthetic_id,
                            &mut current_day,
                            &mut buffer,
                            &mut written_since_checkpoint,
                            CHECKPOINT_EVERY,
                        )
                        .await;
                        for a in pending_acks {
                            let _ = a.send(());
                        }
                    }
                    None => {
                        flush_entries(
                            &logs_dir,
                            &day_dbs,
                            &recent_cache,
                            &disk_has_more,
                            &mut next_synthetic_id,
                            &mut current_day,
                            &mut buffer,
                            &mut written_since_checkpoint,
                            CHECKPOINT_EVERY,
                        )
                        .await;
                        if let Some((_, db)) = current_day.take() {
                            checkpoint_db(&db).await;
                        }
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if !buffer.is_empty() {
                    flush_entries(
                        &logs_dir,
                        &day_dbs,
                        &recent_cache,
                        &disk_has_more,
                        &mut next_synthetic_id,
                        &mut current_day,
                        &mut buffer,
                        &mut written_since_checkpoint,
                        CHECKPOINT_EVERY,
                    )
                    .await;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn flush_entries(
    logs_dir: &Path,
    day_dbs: &DayDbCache,
    recent_cache: &RecentLogCache,
    disk_has_more: &Arc<AtomicBool>,
    next_synthetic_id: &mut i64,
    current_day: &mut Option<(NaiveDate, Database)>,
    buffer: &mut Vec<LogEntry>,
    written_since_checkpoint: &mut usize,
    checkpoint_every: usize,
) {
    if buffer.is_empty() {
        return;
    }

    // Populate hot in-memory ring buffer first (newest at front)
    if let Ok(mut cache) = recent_cache.write() {
        for entry in buffer.iter() {
            let mut cached = entry.clone();
            if cached.id.is_none() {
                cached.id = Some(*next_synthetic_id);
                *next_synthetic_id += 1;
            }
            cache.push_front(cached);
            if cache.len() > RECENT_CACHE_CAP {
                cache.pop_back();
                disk_has_more.store(true, Ordering::Relaxed);
            }
        }
    }

    let today = Utc::now().date_naive();
    let need_open = match current_day {
        Some((d, _)) => *d != today,
        None => true,
    };
    if need_open {
        if let Some((_, old_db)) = current_day.take() {
            checkpoint_db(&old_db).await;
            *written_since_checkpoint = 0;
        }
        match get_or_open_day_db(logs_dir, day_dbs, today).await {
            Ok(db) => *current_day = Some((today, db)),
            Err(e) => {
                tracing::error!(error = %e, "failed to open daily log database");
                buffer.clear();
                return;
            }
        }
    }

    let Some((_, db)) = current_day.as_ref() else {
        buffer.clear();
        return;
    };

    let Ok(mut conn) = connect_db(db) else {
        buffer.clear();
        return;
    };
    let Ok(tx) = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .await
    else {
        buffer.clear();
        return;
    };

    let batch_count = buffer.len();
    for e in buffer.drain(..) {
        let _ = tx
            .execute(
                "INSERT INTO logs (ts, project_id, request_id, source, level, message, meta_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                vec![
                    TursoValue::Integer(e.ts),
                    e.project_id
                        .map(TursoValue::Text)
                        .unwrap_or(TursoValue::Null),
                    e.request_id
                        .map(TursoValue::Text)
                        .unwrap_or(TursoValue::Null),
                    TursoValue::Text(e.source),
                    TursoValue::Text(e.level),
                    TursoValue::Text(e.message),
                    e.meta_json
                        .map(TursoValue::Text)
                        .unwrap_or(TursoValue::Null),
                ],
            )
            .await;
    }
    if tx.commit().await.is_ok() {
        *written_since_checkpoint += batch_count;
        if *written_since_checkpoint >= checkpoint_every {
            checkpoint_db(db).await;
            *written_since_checkpoint = 0;
        }
    }
}

#[async_trait]
impl LogStore for DailyLogStore {
    fn emit(&self, entry: LogEntry) {
        let _ = self.broadcast_tx.send(entry.clone());
        match self.cmd_tx.try_send(LogCmd::Entry(entry)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(LogCmd::Entry(dropped))) => {
                if dropped.level != "debug" {
                    let dropped_total = self.dropped_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let now_sec = Utc::now().timestamp();
                    let prev_sec = self.last_warn_sec.load(Ordering::Relaxed);
                    if now_sec - prev_sec >= 5
                        && self
                            .last_warn_sec
                            .compare_exchange(
                                prev_sec,
                                now_sec,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            )
                            .is_ok()
                    {
                        tracing::warn!(
                            source = %dropped.source,
                            dropped_total,
                            "log channel saturated, dropping log entries"
                        );
                    }
                }
            }
            Err(_) => {}
        }
    }

    fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.broadcast_tx.subscribe()
    }

    async fn query(&self, q: LogQuery) -> Result<Vec<LogEntry>> {
        let _ = self.flush().await;

        let limit = q.limit.unwrap_or(100).min(500);
        let terms_lower: Vec<String> =
            q.q.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|raw_q| {
                    raw_q
                        .split_whitespace()
                        .take(16)
                        .map(|t| t.to_lowercase())
                        .collect()
                })
                .unwrap_or_default();

        if let Ok(cache) = self.recent_cache.read() {
            let mut cached_matches: Vec<LogEntry> = Vec::with_capacity(limit.min(cache.len()));
            for entry in cache.iter() {
                if matches_log_query(entry, &q, &terms_lower) {
                    cached_matches.push(entry.clone());
                    if cached_matches.len() >= limit {
                        break;
                    }
                }
            }
            if cached_matches.len() >= limit || !self.disk_has_more.load(Ordering::Relaxed) {
                return Ok(cached_matches);
            }
        }

        let mut files: Vec<PathBuf> = Vec::new();
        let mut rd = tokio::fs::read_dir(&self.logs_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("read logs dir: {e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| FerroError::Internal(anyhow!("iterate logs dir: {e}")))?
        {
            let p = entry.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("logs_") && name.ends_with(".db") {
                    files.push(p);
                }
            }
        }
        files.sort();
        files.reverse();

        let mut collected = Vec::new();

        for path in files {
            if collected.len() >= limit {
                break;
            }
            let db = if let Some(day) = Self::parse_day_from_path(&path) {
                get_or_open_day_db(&self.logs_dir, &self.day_dbs, day).await?
            } else {
                open_turso_db(&path).await?
            };
            let conn = connect_db(&db)?;

            let mut clauses = vec!["1=1".to_string()];
            let mut params: Vec<TursoValue> = Vec::new();
            let mut idx = 1;

            if let Some(pid) = &q.project_id {
                clauses.push(format!("project_id = ?{idx}"));
                params.push(TursoValue::Text(pid.clone()));
                idx += 1;
            }
            if let Some(rid) = &q.request_id {
                clauses.push(format!("request_id = ?{idx}"));
                params.push(TursoValue::Text(rid.clone()));
                idx += 1;
            }
            if let Some(src) = &q.source {
                clauses.push(format!("source = ?{idx}"));
                params.push(TursoValue::Text(src.clone()));
                idx += 1;
            }
            if let Some(lvl) = &q.level {
                clauses.push(format!("level = ?{idx}"));
                params.push(TursoValue::Text(lvl.clone()));
                idx += 1;
            }
            if let Some(since) = q.since_ts {
                clauses.push(format!("ts >= ?{idx}"));
                params.push(TursoValue::Integer(since));
                idx += 1;
            }
            if let Some(raw_q) = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                for term in raw_q.split_whitespace().take(16) {
                    let pattern = format!("%{}%", escape_like(&term.to_lowercase()));
                    clauses.push(format!(
                        "(LOWER(message) LIKE ?{idx} ESCAPE '\\' OR LOWER(COALESCE(meta_json, '')) LIKE ?{idx} ESCAPE '\\' OR LOWER(COALESCE(request_id, '')) LIKE ?{idx} ESCAPE '\\')"
                    ));
                    params.push(TursoValue::Text(pattern));
                    idx += 1;
                }
            }

            let remaining = (limit - collected.len()) as i64;
            let sql = format!(
                "SELECT id, ts, project_id, request_id, source, level, message, meta_json
                 FROM logs WHERE {} ORDER BY ts DESC, id DESC LIMIT ?{idx}",
                clauses.join(" AND ")
            );
            params.push(TursoValue::Integer(remaining));

            let mut rows = match conn.query(&sql, params).await {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(%err, file = %path.display(), "daily log query failed");
                    continue;
                }
            };
            while let Some(row) = rows.next().await.map_err(map_turso_err)? {
                collected.push(LogEntry {
                    id: Some(row.get(0).map_err(map_turso_err)?),
                    ts: row.get(1).map_err(map_turso_err)?,
                    project_id: opt_text(&row, 2)?,
                    request_id: opt_text(&row, 3)?,
                    source: row.get(4).map_err(map_turso_err)?,
                    level: row.get(5).map_err(map_turso_err)?,
                    message: row.get(6).map_err(map_turso_err)?,
                    meta_json: opt_text(&row, 7)?,
                });
            }
        }

        Ok(collected)
    }

    async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(LogCmd::Flush(tx)).await.is_ok() {
            let _ = rx.await;
        }
        Ok(())
    }

    async fn cleanup_old_files(&self, retention_days: u32) -> Result<u64> {
        let cutoff = Utc::now().date_naive() - chrono::Duration::days(retention_days as i64);
        let mut rd = tokio::fs::read_dir(&self.logs_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("read logs dir: {e}")))?;
        let mut removed = 0u64;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| FerroError::Internal(anyhow!("iterate logs dir: {e}")))?
        {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // Match `logs_YYYY_MM_DD.db` (and clean accompanying -wal/-shm/-log files)
            if let Some(stem) = name.strip_prefix("logs_") {
                if let Some(date_part) = stem.split(".db").next() {
                    if let Ok(file_date) = NaiveDate::parse_from_str(date_part, "%Y_%m_%d") {
                        if file_date < cutoff {
                            self.day_dbs.lock().await.remove(&file_date);
                            if tokio::fs::remove_file(&path).await.is_ok() {
                                removed += 1;
                            }
                        }
                    }
                }
            }
        }
        if removed > 0 {
            if let Ok(mut cache) = self.recent_cache.write() {
                cache.retain(|e| {
                    Utc.timestamp_millis_opt(e.ts)
                        .single()
                        .map(|dt| dt.date_naive() >= cutoff)
                        .unwrap_or(true)
                });
            }
        }
        Ok(removed)
    }
}

fn opt_text(row: &turso::Row, idx: usize) -> Result<Option<String>> {
    match row.get_value(idx).map_err(map_turso_err)? {
        TursoValue::Null => Ok(None),
        TursoValue::Text(s) => Ok(Some(s)),
        other => Err(FerroError::Internal(anyhow!(
            "unexpected log column {idx} type: {other:?}"
        ))),
    }
}
