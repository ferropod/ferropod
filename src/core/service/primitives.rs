use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock as StdRwLock};
use std::time::{Duration, Instant};

use tokio::sync::{RwLock, Semaphore};

use crate::core::model::{RouteEntry, UserDbMeta};
use crate::gateway::guard::SlidingWindowRateLimiter;

/// Bounded in-memory TTL cache backed by a short-lived mutex.
pub(crate) struct TtlCache<V> {
    ttl: Duration,
    max_entries: usize,
    entries: Mutex<HashMap<String, (Instant, V)>>,
}

impl<V: Clone> TtlCache<V> {
    pub(crate) fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            max_entries: max_entries.max(16),
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<V> {
        let guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .get(key)
            .filter(|(ts, _)| ts.elapsed() < self.ttl)
            .map(|(_, val)| val.clone())
    }

    pub(crate) fn insert(&self, key: String, val: V) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if guard.len() >= self.max_entries {
            guard.clear();
        }
        guard.insert(key, (Instant::now(), val));
    }

    #[allow(dead_code)]
    pub(crate) fn remove(&self, key: &str) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(key);
    }

    pub(crate) fn clear(&self) {
        let mut guard = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        guard.clear();
    }
}

/// Dynamic per-resource semaphore map bounded lazily on resource deletion.
pub(crate) struct KeyedSemaphores {
    map: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl KeyedSemaphores {
    pub(crate) fn new() -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn get_or_init(&self, key: &str, cap: usize) -> Arc<Semaphore> {
        let mut guard = self.map.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Semaphore::new(cap.max(1))))
            .clone()
    }

    pub(crate) fn remove(&self, key: &str) {
        let mut guard = self.map.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(key);
    }
}

/// Concurrency bulkheads, sliding-window rate limiter, and alert/cron throttles.
pub(crate) struct PlatformLimits {
    pub(crate) http_concurrency: Arc<Semaphore>,
    pub(crate) admin_concurrency: Arc<Semaphore>,
    pub(crate) app_concurrency: KeyedSemaphores,
    pub(crate) db_concurrency: KeyedSemaphores,
    pub(crate) search_concurrency: KeyedSemaphores,
    pub(crate) queue_concurrency: KeyedSemaphores,
    pub(crate) ai_concurrency: Arc<Semaphore>,
    pub(crate) alert_concurrency: Arc<Semaphore>,
    pub(crate) alert_cooldowns: Mutex<HashMap<String, Instant>>,
    pub(crate) in_flight_crons: Arc<Mutex<HashSet<String>>>,
    pub(crate) rate_limiter: Arc<SlidingWindowRateLimiter>,
}

/// Hot in-memory lookup caches for routing, AI responses, bucket headers, and user DB metadata.
pub(crate) struct PlatformCaches {
    pub(crate) routes: RwLock<HashMap<String, RouteEntry>>,
    pub(crate) ai_cache: RwLock<HashMap<String, (i64, serde_json::Value)>>,
    pub(crate) bucket_cache_control: StdRwLock<HashMap<String, Option<String>>>,
    pub(crate) db_meta: TtlCache<UserDbMeta>,
    pub(crate) db_row_counts: Mutex<HashMap<String, (Instant, usize)>>,
}
