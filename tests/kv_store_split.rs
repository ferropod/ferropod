//! `kv.db` isolation: user KV entries live outside `core.db`, legacy rows migrate once,
//! and snapshots include the new file.

use bytes::Bytes;
use turso::Value as TursoValue;

use ferropod::core::traits::KvStore;
use ferropod::storage::core_store::TursoCoreStore;
use ferropod::storage::kv_store::TursoKvStore;
use ferropod::storage::tx::connect_db;

async fn table_exists(db: &turso::Database, name: &str) -> bool {
    let conn = connect_db(db).unwrap();
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            [TursoValue::Text(name.to_string())],
        )
        .await
        .unwrap();
    rows.next().await.unwrap().is_some()
}

#[tokio::test]
async fn core_db_no_longer_hosts_kv_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let core = TursoCoreStore::open(&tmp.path().join("core.db"))
        .await
        .unwrap();
    let kv = TursoKvStore::open(&tmp.path().join("kv.db"), 128)
        .await
        .unwrap();

    kv.put("ns", "k", Bytes::from_static(b"v"), None, None, false)
        .await
        .unwrap();

    assert!(!table_exists(core.database(), "kv_entries").await);
    assert_eq!(kv.get("ns", "k").await.unwrap().unwrap().value, b"v");
}

#[tokio::test]
async fn legacy_core_kv_rows_are_migrated_once_and_table_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let core = TursoCoreStore::open(&tmp.path().join("core.db"))
        .await
        .unwrap();

    // Simulate a pre-split data directory: KV rows stored inside core.db.
    {
        let conn = connect_db(core.database()).unwrap();
        conn.execute_batch(
            "CREATE TABLE kv_entries (
                namespace TEXT NOT NULL,
                key TEXT NOT NULL,
                value BLOB NOT NULL,
                metadata TEXT,
                expires_at INTEGER,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (namespace, key)
            );
            CREATE INDEX idx_kv_expires ON kv_entries (expires_at);",
        )
        .await
        .unwrap();
        // More than one migration batch (500) to exercise the batching loop.
        for i in 0..1_203i64 {
            conn.execute(
                "INSERT INTO kv_entries (namespace, key, value, metadata, expires_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
                vec![
                    TursoValue::Text(format!("ns{}", i % 3)),
                    TursoValue::Text(format!("key{i:05}")),
                    TursoValue::Blob(format!("val{i}").into_bytes()),
                    TursoValue::Text("m".into()),
                    TursoValue::Integer(1_000 + i),
                ],
            )
            .await
            .unwrap();
        }
    }

    let kv = TursoKvStore::open(&tmp.path().join("kv.db"), 128)
        .await
        .unwrap();
    let moved = kv.migrate_from_legacy_core(core.database()).await.unwrap();
    assert_eq!(moved, 1_203);
    assert!(!table_exists(core.database(), "kv_entries").await);

    let got = kv.get("ns1", "key00001").await.unwrap().unwrap();
    assert_eq!(got.value, b"val1");
    assert_eq!(got.metadata.as_deref(), Some("m"));

    // Idempotent: second run is a no-op.
    assert_eq!(
        kv.migrate_from_legacy_core(core.database()).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn kv_snapshot_produces_kv_db_file() {
    let tmp = tempfile::tempdir().unwrap();
    let kv = TursoKvStore::open(&tmp.path().join("kv.db"), 128)
        .await
        .unwrap();
    kv.put("ns", "k", Bytes::from_static(b"v"), None, None, false)
        .await
        .unwrap();

    let out = tmp.path().join("snap").join("kv.db");
    kv.snapshot_to(&out).await.unwrap();
    assert!(out.exists());

    let restored = TursoKvStore::open(&out, 128).await.unwrap();
    assert_eq!(restored.get("ns", "k").await.unwrap().unwrap().value, b"v");
}
