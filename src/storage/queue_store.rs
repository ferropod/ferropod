use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use chrono::Utc;
use tokio::sync::{mpsc, oneshot, Notify};
use turso::{transaction::TransactionBehavior, Database, Value as TursoValue};

use crate::core::model::{
    EnqueueOpts, FerroError, Job, JobStatus, QueueConfig, QueueStats, Result,
};
use crate::core::traits::QueueStore;
use crate::storage::tx::{
    checkpoint_db, connect_db, copy_db_with_sidecars, ensure_resource_group_column, map_turso_err,
    open_turso_db, with_retry,
};

const QUEUE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS queues (
    name TEXT PRIMARY KEY,
    project_id TEXT,
    consumer_project_id TEXT,
    max_concurrency INTEGER NOT NULL DEFAULT 4,
    rate_limit_per_sec INTEGER,
    batch_size INTEGER NOT NULL DEFAULT 20,
    lease_secs INTEGER NOT NULL DEFAULT 60,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    created_at INTEGER NOT NULL,
    resource_group TEXT
);

CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    queue TEXT NOT NULL,
    payload BLOB NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    priority INTEGER NOT NULL DEFAULT 0,
    run_at INTEGER NOT NULL,
    lease_until INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    dedup_key TEXT,
    last_error TEXT,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_jobs_poll
    ON jobs (queue, status, run_at, priority DESC, id);
CREATE INDEX IF NOT EXISTS idx_jobs_lease
    ON jobs (status, lease_until);
CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedup
    ON jobs (queue, dedup_key) WHERE dedup_key IS NOT NULL AND status != 'dead';
"#;

struct IngestItem {
    queue: String,
    payload: Vec<u8>,
    opts: EnqueueOpts,
    created_ms: i64,
    reply: oneshot::Sender<Result<i64>>,
}

/// Turso MVCC queue reservoir (`queue.db`) with bounded micro-batch ingestion and backpressure.
#[derive(Clone)]
pub struct TursoQueueStore {
    db: Database,
    db_path: PathBuf,
    ingest_tx: mpsc::Sender<IngestItem>,
    pub notify: Arc<Notify>,
}

impl TursoQueueStore {
    pub async fn open(path: &Path) -> Result<Self> {
        let db = open_turso_db(path).await?;
        let conn = connect_db(&db)?;
        conn.execute_batch(QUEUE_SCHEMA)
            .await
            .map_err(map_turso_err)?;

        ensure_resource_group_column(&conn, "queues").await?;

        let (ingest_tx, ingest_rx) = mpsc::channel::<IngestItem>(4_096);
        let notify = Arc::new(Notify::new());
        tokio::spawn(run_batch_ingestor(db.clone(), ingest_rx, notify.clone()));

        Ok(Self {
            db,
            db_path: path.to_path_buf(),
            ingest_tx,
            notify,
        })
    }

    pub async fn checkpoint(&self) {
        checkpoint_db(&self.db).await;
    }
}

async fn run_batch_ingestor(db: Database, mut rx: mpsc::Receiver<IngestItem>, notify: Arc<Notify>) {
    const MAX_BATCH: usize = 200;
    const MAX_WAIT: Duration = Duration::from_millis(10);

    let mut batch: Vec<IngestItem> = Vec::with_capacity(MAX_BATCH);
    while let Some(first) = rx.recv().await {
        batch.push(first);
        let deadline = tokio::time::Instant::now() + MAX_WAIT;

        while batch.len() < MAX_BATCH {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(item)) => batch.push(item),
                Ok(None) | Err(_) => break,
            }
        }

        let results = insert_batch_with_retry(&db, &batch).await;
        let mut any_ok = false;
        match results {
            Ok(ids) => {
                for (item, id_res) in batch.drain(..).zip(ids.into_iter()) {
                    if id_res.is_ok() {
                        any_ok = true;
                    }
                    let _ = item.reply.send(id_res);
                }
            }
            Err(err) => {
                let msg = err.to_string();
                for item in batch.drain(..) {
                    let _ = item
                        .reply
                        .send(Err(FerroError::Internal(anyhow::anyhow!(msg.clone()))));
                }
            }
        }
        if any_ok {
            notify.notify_waiters();
        }
    }
}

async fn insert_batch_with_retry(db: &Database, batch: &[IngestItem]) -> Result<Vec<Result<i64>>> {
    with_retry(|| {
        let db = db.clone();
        async move {
            let mut conn = db.connect()?;
            let tx = conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .await?;
            let mut out = Vec::with_capacity(batch.len());

            for item in batch {
                let delay_ms = item.opts.delay_secs.unwrap_or(0) as i64 * 1_000;
                let run_at = item.created_ms + delay_ms;
                let priority = item.opts.priority.unwrap_or(0) as i64;
                let max_attempts = item.opts.max_attempts.unwrap_or(5) as i64;
                let dedup = item.opts.dedup_key.clone();

                let res = tx
                    .execute(
                        "INSERT INTO jobs (queue, payload, status, priority, run_at, lease_until, attempts, max_attempts, dedup_key, created_at)
                         VALUES (?1, ?2, 'pending', ?3, ?4, 0, 0, ?5, ?6, ?7)",
                        vec![
                            TursoValue::Text(item.queue.clone()),
                            TursoValue::Blob(item.payload.clone()),
                            TursoValue::Integer(priority),
                            TursoValue::Integer(run_at),
                            TursoValue::Integer(max_attempts),
                            dedup.clone().map(TursoValue::Text).unwrap_or(TursoValue::Null),
                            TursoValue::Integer(item.created_ms),
                        ],
                    )
                    .await;

                match res {
                    Ok(_) => {
                        out.push(Ok(tx.last_insert_rowid()));
                    }
                    Err(turso::Error::Constraint(_)) if dedup.is_some() => {
                        let dk = dedup.unwrap();
                        let mut existing = tx
                            .query(
                                "SELECT id FROM jobs WHERE queue = ?1 AND dedup_key = ?2 AND status != 'dead' LIMIT 1",
                                (item.queue.as_str(), dk.as_str()),
                            )
                            .await?;
                        if let Some(row) = existing.next().await? {
                            let existing_id: i64 = row.get(0)?;
                            out.push(Ok(existing_id));
                        } else {
                            out.push(Err(FerroError::Conflict("duplicate dedup_key".into())));
                        }
                    }
                    Err(e) => return Err(e),
                }
            }

            tx.commit().await?;
            Ok(out)
        }
    })
    .await
}

#[async_trait]
impl QueueStore for TursoQueueStore {
    async fn create_queue(&self, cfg: QueueConfig) -> Result<()> {
        with_retry(|| {
            let db = self.db.clone();
            let cfg = cfg.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "INSERT INTO queues (name, project_id, consumer_project_id, max_concurrency, rate_limit_per_sec, batch_size, lease_secs, max_attempts, created_at, resource_group)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
",
                    vec![
                        TursoValue::Text(cfg.name),
                        cfg.project_id.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        cfg.consumer_project_id.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(cfg.max_concurrency),
                        cfg.rate_limit_per_sec.map(TursoValue::Integer).unwrap_or(TursoValue::Null),
                        TursoValue::Integer(cfg.batch_size),
                        TursoValue::Integer(cfg.lease_secs),
                        TursoValue::Integer(cfg.max_attempts),
                        TursoValue::Integer(cfg.created_at),
                        cfg.group.map(TursoValue::Text).unwrap_or(TursoValue::Null),
                    ],
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn update_queue(&self, cfg: QueueConfig) -> Result<()> {
        let affected = with_retry(|| {
            let db = self.db.clone();
            let cfg = cfg.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE queues SET max_concurrency = ?1, rate_limit_per_sec = ?2,
                     batch_size = ?3, lease_secs = ?4, max_attempts = ?5
                     WHERE name = ?6 AND project_id IS ?7",
                    vec![
                        TursoValue::Integer(cfg.max_concurrency),
                        cfg.rate_limit_per_sec
                            .map(TursoValue::Integer)
                            .unwrap_or(TursoValue::Null),
                        TursoValue::Integer(cfg.batch_size),
                        TursoValue::Integer(cfg.lease_secs),
                        TursoValue::Integer(cfg.max_attempts),
                        TursoValue::Text(cfg.name),
                        cfg.project_id
                            .map(TursoValue::Text)
                            .unwrap_or(TursoValue::Null),
                    ],
                )
                .await
            }
        })
        .await?;
        if affected == 0 {
            return Err(FerroError::NotFound(
                "queue no longer exists or ownership changed".into(),
            ));
        }
        Ok(())
    }

    async fn get_queue(&self, name: &str) -> Result<Option<QueueConfig>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, consumer_project_id, max_concurrency, rate_limit_per_sec, batch_size, lease_secs, max_attempts, created_at, resource_group
                 FROM queues WHERE name = ?1 LIMIT 1",
                [name],
            )
            .await
            .map_err(map_turso_err)?;
        if let Some(row) = rows.next().await.map_err(map_turso_err)? {
            Ok(Some(QueueConfig {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                consumer_project_id: opt_text(&row, 2)?,
                max_concurrency: row.get(3).map_err(map_turso_err)?,
                rate_limit_per_sec: opt_i64(&row, 4)?,
                batch_size: row.get(5).map_err(map_turso_err)?,
                lease_secs: row.get(6).map_err(map_turso_err)?,
                max_attempts: row.get(7).map_err(map_turso_err)?,
                created_at: row.get(8).map_err(map_turso_err)?,
                group: opt_text(&row, 9)?,
            }))
        } else {
            Ok(None)
        }
    }

    async fn list_queues(&self) -> Result<Vec<QueueConfig>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT name, project_id, consumer_project_id, max_concurrency, rate_limit_per_sec, batch_size, lease_secs, max_attempts, created_at, resource_group
                 FROM queues ORDER BY name ASC",
                (),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(QueueConfig {
                name: row.get(0).map_err(map_turso_err)?,
                project_id: opt_text(&row, 1)?,
                consumer_project_id: opt_text(&row, 2)?,
                max_concurrency: row.get(3).map_err(map_turso_err)?,
                rate_limit_per_sec: opt_i64(&row, 4)?,
                batch_size: row.get(5).map_err(map_turso_err)?,
                lease_secs: row.get(6).map_err(map_turso_err)?,
                max_attempts: row.get(7).map_err(map_turso_err)?,
                created_at: row.get(8).map_err(map_turso_err)?,
                group: opt_text(&row, 9)?,
            });
        }
        Ok(out)
    }

    async fn enqueue(&self, queue: &str, payload: Bytes, opts: EnqueueOpts) -> Result<i64> {
        let (tx, rx) = oneshot::channel();
        let item = IngestItem {
            queue: queue.to_string(),
            payload: payload.to_vec(),
            opts,
            created_ms: Utc::now().timestamp_millis(),
            reply: tx,
        };
        self.ingest_tx.try_send(item).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                FerroError::RateLimited("queue ingestion buffer is full".into())
            }
            mpsc::error::TrySendError::Closed(_) => {
                FerroError::Internal(anyhow::anyhow!("queue batch ingestor closed"))
            }
        })?;
        rx.await
            .map_err(|_| FerroError::Internal(anyhow::anyhow!("queue ingest reply dropped")))?
    }

    async fn claim_batch(&self, queue: &str, limit: usize, lease_secs: i64) -> Result<Vec<Job>> {
        let now_ms = Utc::now().timestamp_millis();
        let lease_until = now_ms + lease_secs * 1_000;
        let q = queue.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let q = q.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT id, queue, payload, status, priority, run_at, lease_until, attempts, max_attempts, dedup_key, last_error, created_at
                         FROM jobs
                         WHERE queue = ?1 AND status = 'pending' AND run_at <= ?2
                         ORDER BY priority DESC, run_at ASC, id ASC
                         LIMIT ?3",
                        (q.as_str(), now_ms, limit as i64),
                    )
                    .await?;

                let mut jobs = Vec::new();
                while let Some(row) = rows.next().await? {
                    let mut job = row_to_job_turso(&row)?;
                    job.status = JobStatus::Running;
                    job.attempts += 1;
                    job.lease_until = lease_until;
                    jobs.push(job);
                }
                drop(rows);

                let mut claimed = Vec::with_capacity(jobs.len());
                for job in jobs {
                    let affected = tx
                        .execute(
                            "UPDATE jobs SET status = 'running', attempts = attempts + 1, lease_until = ?1
                             WHERE id = ?2 AND status = 'pending'",
                            (lease_until, job.id),
                        )
                        .await?;
                    if affected == 1 {
                        claimed.push(job);
                    }
                }
                tx.commit().await?;
                Ok(claimed)
            }
        })
        .await
    }

    async fn ack_batch(&self, job_ids: &[i64]) -> Result<()> {
        if job_ids.is_empty() {
            return Ok(());
        }
        let ids = job_ids.to_vec();
        with_retry(|| {
            let db = self.db.clone();
            let ids = ids.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                for id in ids {
                    tx.execute("DELETE FROM jobs WHERE id = ?1", [id]).await?;
                }
                tx.commit().await
            }
        })
        .await
    }

    async fn nack(
        &self,
        job_id: i64,
        retry_after_secs: u64,
        err: &str,
    ) -> Result<Option<JobStatus>> {
        let now_ms = Utc::now().timestamp_millis();
        let next_run_at = now_ms + retry_after_secs as i64 * 1_000;
        let err_s = err.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let err_s = err_s.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT attempts, max_attempts FROM jobs WHERE id = ?1",
                        [job_id],
                    )
                    .await?;
                let Some(row) = rows.next().await? else {
                    tx.commit().await?;
                    return Ok(None);
                };
                let attempts: i64 = row.get(0)?;
                let max_attempts: i64 = row.get(1)?;
                drop(rows);

                let next_status = if attempts >= max_attempts {
                    tx.execute(
                        "UPDATE jobs SET status = 'dead', lease_until = 0, last_error = ?1 WHERE id = ?2",
                        (err_s, job_id),
                    )
                    .await?;
                    JobStatus::Dead
                } else {
                    tx.execute(
                        "UPDATE jobs SET status = 'pending', run_at = ?1, lease_until = 0, last_error = ?2 WHERE id = ?3",
                        (next_run_at, err_s, job_id),
                    )
                    .await?;
                    JobStatus::Pending
                };
                tx.commit().await?;
                Ok(Some(next_status))
            }
        })
        .await
    }

    async fn mark_dead(&self, job_id: i64, err: &str) -> Result<()> {
        let err_s = err.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let err_s = err_s.clone();
            async move {
                let conn = db.connect()?;
                conn.execute(
                    "UPDATE jobs SET status = 'dead', lease_until = 0, last_error = ?1 WHERE id = ?2",
                    (err_s, job_id),
                )
                .await
            }
        })
        .await?;
        Ok(())
    }

    async fn reap_expired(&self) -> Result<u64> {
        let now_ms = Utc::now().timestamp_millis();
        with_retry(|| {
            let db = self.db.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT id, attempts, max_attempts FROM jobs WHERE status = 'running' AND lease_until < ?1 LIMIT 500",
                        [now_ms],
                    )
                    .await?;
                let mut expired = Vec::new();
                while let Some(row) = rows.next().await? {
                    let id: i64 = row.get(0)?;
                    let attempts: i64 = row.get(1)?;
                    let max_attempts: i64 = row.get(2)?;
                    expired.push((id, attempts >= max_attempts));
                }
                drop(rows);

                let mut count = 0u64;
                for (id, dead) in expired {
                    if dead {
                        count += tx
                            .execute(
                                "UPDATE jobs SET status = 'dead', lease_until = 0, last_error = 'lease expired' WHERE id = ?1",
                                [id],
                            )
                            .await?;
                    } else {
                        count += tx
                            .execute(
                                "UPDATE jobs SET status = 'pending', run_at = ?1, lease_until = 0, last_error = 'lease expired' WHERE id = ?2",
                                (now_ms, id),
                            )
                            .await?;
                    }
                }
                tx.commit().await?;
                Ok(count)
            }
        })
        .await
    }

    async fn inspect(&self, queue: &str, status: JobStatus, limit: usize) -> Result<Vec<Job>> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT id, queue, payload, status, priority, run_at, lease_until, attempts, max_attempts, dedup_key, last_error, created_at
                 FROM jobs WHERE queue = ?1 AND status = ?2 ORDER BY id ASC LIMIT ?3",
                (queue, status.as_str(), limit.min(500) as i64),
            )
            .await
            .map_err(map_turso_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            out.push(row_to_job(&row)?);
        }
        Ok(out)
    }

    async fn dlq_replay(&self, queue: &str, job_ids: Option<&[i64]>) -> Result<u64> {
        let now_ms = Utc::now().timestamp_millis();
        let q = queue.to_string();
        let ids_owned = job_ids.map(|s| s.to_vec());
        with_retry(|| {
            let db = self.db.clone();
            let q = q.clone();
            let ids_owned = ids_owned.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut updated = 0u64;
                match ids_owned {
                    Some(ids) => {
                        for id in ids {
                            updated += tx
                                .execute(
                                    "UPDATE jobs SET status = 'pending', attempts = 0, run_at = ?1, lease_until = 0 WHERE queue = ?2 AND id = ?3 AND status = 'dead'",
                                    (now_ms, q.as_str(), id),
                                )
                                .await?;
                        }
                    }
                    None => {
                        updated += tx
                            .execute(
                                "UPDATE jobs SET status = 'pending', attempts = 0, run_at = ?1, lease_until = 0 WHERE queue = ?2 AND status = 'dead'",
                                (now_ms, q.as_str()),
                            )
                            .await?;
                    }
                }
                tx.commit().await?;
                Ok(updated)
            }
        })
        .await
    }

    async fn purge(&self, queue: &str) -> Result<u64> {
        let q = queue.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let q = q.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM jobs WHERE queue = ?1", [q]).await
            }
        })
        .await
    }

    async fn delete_queue(&self, name: &str) -> Result<()> {
        let q = name.to_string();
        with_retry(|| {
            let db = self.db.clone();
            let q = q.clone();
            async move {
                let conn = db.connect()?;
                conn.execute("DELETE FROM jobs WHERE queue = ?1", [q.clone()])
                    .await?;
                conn.execute("DELETE FROM queues WHERE name = ?1", [q])
                    .await
            }
        })
        .await?;
        Ok(())
    }

    async fn cleanup_expired_batch(
        &self,
        pending_max_age_secs: i64,
        dlq_max_age_secs: i64,
        batch_limit: usize,
    ) -> Result<u64> {
        let now_ms = Utc::now().timestamp_millis();
        let pending_cutoff = now_ms - pending_max_age_secs * 1_000;
        let dlq_cutoff = now_ms - dlq_max_age_secs * 1_000;
        with_retry(|| {
            let db = self.db.clone();
            async move {
                let mut conn = db.connect()?;
                let tx = conn
                    .transaction_with_behavior(TransactionBehavior::Concurrent)
                    .await?;
                let mut rows = tx
                    .query(
                        "SELECT id FROM jobs
                         WHERE (status = 'pending' AND created_at < ?1)
                            OR (status = 'dead' AND created_at < ?2)
                         LIMIT ?3",
                        (pending_cutoff, dlq_cutoff, batch_limit as i64),
                    )
                    .await?;
                let mut ids = Vec::new();
                while let Some(row) = rows.next().await? {
                    ids.push(row.get::<i64>(0)?);
                }
                drop(rows);

                let mut deleted = 0u64;
                for id in ids {
                    deleted += tx.execute("DELETE FROM jobs WHERE id = ?1", [id]).await?;
                }
                tx.commit().await?;
                Ok(deleted)
            }
        })
        .await
    }

    async fn stats(&self, queue: &str) -> Result<QueueStats> {
        let conn = connect_db(&self.db)?;
        let mut rows = conn
            .query(
                "SELECT status, COUNT(*) FROM jobs WHERE queue = ?1 GROUP BY status",
                [queue],
            )
            .await
            .map_err(map_turso_err)?;
        let mut stats = QueueStats {
            queue: queue.to_string(),
            ..Default::default()
        };
        while let Some(row) = rows.next().await.map_err(map_turso_err)? {
            let st: String = row.get(0).map_err(map_turso_err)?;
            let cnt: i64 = row.get(1).map_err(map_turso_err)?;
            match st.as_str() {
                "pending" => stats.pending = cnt,
                "running" => stats.running = cnt,
                "dead" => stats.dead = cnt,
                _ => {}
            }
        }
        Ok(stats)
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

fn opt_text(row: &turso::Row, idx: usize) -> Result<Option<String>> {
    match row.get_value(idx).map_err(map_turso_err)? {
        TursoValue::Null => Ok(None),
        TursoValue::Text(s) => Ok(Some(s)),
        other => Err(FerroError::Internal(anyhow::anyhow!(
            "unexpected column {idx} type for text: {other:?}"
        ))),
    }
}

fn opt_i64(row: &turso::Row, idx: usize) -> Result<Option<i64>> {
    match row.get_value(idx).map_err(map_turso_err)? {
        TursoValue::Null => Ok(None),
        TursoValue::Integer(i) => Ok(Some(i)),
        other => Err(FerroError::Internal(anyhow::anyhow!(
            "unexpected column {idx} type for i64: {other:?}"
        ))),
    }
}

fn row_to_job_turso(row: &turso::Row) -> turso::Result<Job> {
    let status_s: String = row.get(3)?;
    let status = match status_s.as_str() {
        "pending" => JobStatus::Pending,
        "running" => JobStatus::Running,
        _ => JobStatus::Dead,
    };
    let dedup_key = match row.get_value(9)? {
        TursoValue::Null => None,
        TursoValue::Text(s) => Some(s),
        _ => None,
    };
    let last_error = match row.get_value(10)? {
        TursoValue::Null => None,
        TursoValue::Text(s) => Some(s),
        _ => None,
    };
    Ok(Job {
        id: row.get(0)?,
        queue: row.get(1)?,
        payload: row.get(2)?,
        status,
        priority: row.get::<i64>(4)? as i32,
        run_at: row.get(5)?,
        lease_until: row.get(6)?,
        attempts: row.get::<i64>(7)? as i32,
        max_attempts: row.get::<i64>(8)? as i32,
        dedup_key,
        last_error,
        created_at: row.get(11)?,
    })
}

fn row_to_job(row: &turso::Row) -> Result<Job> {
    row_to_job_turso(row).map_err(map_turso_err)
}
