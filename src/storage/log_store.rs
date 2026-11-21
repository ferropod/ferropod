use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use tokio::sync::{broadcast, mpsc, oneshot};
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};

use crate::core::model::{FerroError, LogEntry, LogQuery, Result};
use crate::core::traits::LogStore;
use crate::storage::core_store::escape_like;
use crate::storage::tx::{checkpoint_db, connect_db, map_turso_err, open_turso_db};

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
CREATE INDEX IF NOT EXISTS idx_logs_project_ts ON logs (project_id, ts DESC);
CREATE INDEX IF NOT EXISTS idx_logs_req ON logs (request_id);
"#;

enum LogCmd {
    Entry(LogEntry),
    Flush(oneshot::Sender<()>),
}

/// Non-blocking daily rotating Turso log store with bounded mpsc and SSE broadcast fan-out.
#[derive(Clone)]
pub struct DailyLogStore {
    logs_dir: PathBuf,
    cmd_tx: mpsc::Sender<LogCmd>,
    broadcast_tx: broadcast::Sender<LogEntry>,
}

impl DailyLogStore {
    pub async fn open(logs_dir: PathBuf, channel_capacity: usize) -> Result<Self> {
        tokio::fs::create_dir_all(&logs_dir)
            .await
            .map_err(|e| FerroError::Internal(anyhow!("failed to create logs dir: {e}")))?;

        let (cmd_tx, cmd_rx) = mpsc::channel::<LogCmd>(channel_capacity.max(128));
        let (broadcast_tx, _) = broadcast::channel::<LogEntry>(512);

        tokio::spawn(run_log_writer(logs_dir.clone(), cmd_rx));

        Ok(Self {
            logs_dir,
            cmd_tx,
            broadcast_tx,
        })
    }

    fn day_file_path(logs_dir: &Path, day: NaiveDate) -> PathBuf {
        logs_dir.join(format!("logs_{}.db", day.format("%Y_%m_%d")))
    }
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

async fn run_log_writer(logs_dir: PathBuf, mut rx: mpsc::Receiver<LogCmd>) {
    const MAX_BATCH: usize = 200;
    const FLUSH_INTERVAL: Duration = Duration::from_millis(500);

    let mut current_day: Option<(NaiveDate, Database)> = None;
    let mut buffer: Vec<LogEntry> = Vec::with_capacity(MAX_BATCH);
    let mut tick = tokio::time::interval(FLUSH_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_cmd = rx.recv() => {
                match maybe_cmd {
                    Some(LogCmd::Entry(entry)) => {
                        buffer.push(entry);
                        if buffer.len() >= MAX_BATCH {
                            flush_entries(&logs_dir, &mut current_day, &mut buffer).await;
                        }
                    }
                    Some(LogCmd::Flush(ack)) => {
                        while let Ok(cmd) = rx.try_recv() {
                            if let LogCmd::Entry(entry) = cmd {
                                buffer.push(entry);
                            }
                        }
                        flush_entries(&logs_dir, &mut current_day, &mut buffer).await;
                        let _ = ack.send(());
                    }
                    None => {
                        flush_entries(&logs_dir, &mut current_day, &mut buffer).await;
                        if let Some((_, db)) = current_day.take() {
                            checkpoint_db(&db).await;
                        }
                        break;
                    }
                }
            }
            _ = tick.tick() => {
                if !buffer.is_empty() {
                    flush_entries(&logs_dir, &mut current_day, &mut buffer).await;
                }
            }
        }
    }
}

async fn flush_entries(
    logs_dir: &Path,
    current_day: &mut Option<(NaiveDate, Database)>,
    buffer: &mut Vec<LogEntry>,
) {
    if buffer.is_empty() {
        return;
    }
    let today = Utc::now().date_naive();
    let need_open = match current_day {
        Some((d, _)) => *d != today,
        None => true,
    };
    if need_open {
        if let Some((_, old_db)) = current_day.take() {
            checkpoint_db(&old_db).await;
        }
        match open_day_db(logs_dir, today).await {
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
    let _ = tx.commit().await;
}

#[async_trait]
impl LogStore for DailyLogStore {
    fn emit(&self, entry: LogEntry) {
        let _ = self.broadcast_tx.send(entry.clone());
        match self.cmd_tx.try_send(LogCmd::Entry(entry)) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(LogCmd::Entry(dropped))) => {
                if dropped.level != "debug" {
                    tracing::warn!(source = %dropped.source, "log channel saturated, dropping log entry");
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

        let limit = q.limit.unwrap_or(100).min(500);
        let mut collected = Vec::new();

        for path in files {
            if collected.len() >= limit {
                break;
            }
            let db = open_turso_db(&path).await?;
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
                for term in raw_q.split_whitespace() {
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
                Err(_) => continue,
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
                        if file_date < cutoff && tokio::fs::remove_file(&path).await.is_ok() {
                            removed += 1;
                        }
                    }
                }
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
