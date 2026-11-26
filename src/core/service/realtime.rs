use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::{broadcast, OwnedSemaphorePermit, RwLock, Semaphore};
use uuid::Uuid;

use crate::core::model::{
    Caller, FerroError, RealtimeChannelInfo, RealtimeMessage, Result, TokenScope,
};

use super::PlatformService;

const CHANNEL_BUFFER_CAPACITY: usize = 64;
pub const MAX_REALTIME_SSE_GLOBAL: usize = 512;
pub const MAX_REALTIME_SSE_PER_CHANNEL: usize = 128;
pub const MAX_REALTIME_PAYLOAD_BYTES: usize = 64 * 1024;
pub const DEFAULT_REALTIME_PUBLISH_RPM: u32 = 600;

struct RealtimeChannelEntry {
    tx: broadcast::Sender<RealtimeMessage>,
    published_total: u64,
    last_published_at_ms: Option<i64>,
}

/// Bounded in-memory Pub-Sub hub backing `/_realtime/{channel}/sse`.
pub struct RealtimeHub {
    max_channels: usize,
    channels: RwLock<HashMap<String, RealtimeChannelEntry>>,
    sse_slots: Arc<Semaphore>,
}

impl RealtimeHub {
    pub fn new(max_channels: usize) -> Self {
        Self {
            max_channels: max_channels.max(16),
            channels: RwLock::new(HashMap::new()),
            sse_slots: Arc::new(Semaphore::new(MAX_REALTIME_SSE_GLOBAL)),
        }
    }

    async fn get_or_create_sender(
        &self,
        channel: &str,
    ) -> Result<broadcast::Sender<RealtimeMessage>> {
        {
            let guard = self.channels.read().await;
            if let Some(entry) = guard.get(channel) {
                return Ok(entry.tx.clone());
            }
        }
        let mut guard = self.channels.write().await;
        if let Some(entry) = guard.get(channel) {
            return Ok(entry.tx.clone());
        }
        if guard.len() >= self.max_channels {
            guard.retain(|_, entry| entry.tx.receiver_count() > 0);
        }
        if guard.len() >= self.max_channels {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, entry)| entry.last_published_at_ms.unwrap_or(0))
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
        }
        let (tx, _) = broadcast::channel(CHANNEL_BUFFER_CAPACITY);
        guard.insert(
            channel.to_string(),
            RealtimeChannelEntry {
                tx: tx.clone(),
                published_total: 0,
                last_published_at_ms: None,
            },
        );
        Ok(tx)
    }

    pub async fn subscribe(
        &self,
        channel: &str,
    ) -> Result<(broadcast::Receiver<RealtimeMessage>, OwnedSemaphorePermit)> {
        let permit = self.sse_slots.clone().try_acquire_owned().map_err(|_| {
            FerroError::RateLimited("realtime global SSE connection limit reached".into())
        })?;
        let tx = self.get_or_create_sender(channel).await?;
        if tx.receiver_count() >= MAX_REALTIME_SSE_PER_CHANNEL {
            return Err(FerroError::RateLimited(format!(
                "realtime channel '{channel}' subscriber limit ({MAX_REALTIME_SSE_PER_CHANNEL}) reached"
            )));
        }
        Ok((tx.subscribe(), permit))
    }

    pub async fn publish(
        &self,
        channel: &str,
        event: &str,
        data: serde_json::Value,
    ) -> Result<RealtimeMessage> {
        let now_ms = Utc::now().timestamp_millis();
        let msg = RealtimeMessage {
            id: format!("rt_{}", &Uuid::new_v4().simple().to_string()[..12]),
            channel: channel.to_string(),
            event: event.to_string(),
            data,
            ts: now_ms,
        };
        let tx = self.get_or_create_sender(channel).await?;
        let mut guard = self.channels.write().await;
        if let Some(entry) = guard.get_mut(channel) {
            entry.published_total = entry.published_total.saturating_add(1);
            entry.last_published_at_ms = Some(now_ms);
        }
        drop(guard);
        let _ = tx.send(msg.clone());
        Ok(msg)
    }

    pub async fn list_channels(&self, project_prefix: Option<&str>) -> Vec<RealtimeChannelInfo> {
        let guard = self.channels.read().await;
        let mut list: Vec<RealtimeChannelInfo> = guard
            .iter()
            .filter(|(name, _)| match project_prefix {
                Some(pid) => name.as_str() == pid || name.starts_with(&format!("{pid}:")),
                None => true,
            })
            .map(|(name, entry)| RealtimeChannelInfo {
                channel: name.clone(),
                subscribers: entry.tx.receiver_count(),
                published_total: entry.published_total,
                last_published_at_ms: entry.last_published_at_ms,
            })
            .collect();
        list.sort_by(|a, b| a.channel.cmp(&b.channel));
        list
    }
}

fn validate_channel_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 128 {
        return Err(FerroError::InvalidArgument(
            "realtime channel name must be between 1 and 128 characters".into(),
        ));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/'))
        || trimmed.contains("..")
    {
        return Err(FerroError::InvalidArgument(format!(
            "invalid realtime channel name '{raw}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn ensure_channel_scope(caller: &Caller, channel: &str) -> Result<()> {
    match &caller.scope {
        TokenScope::Admin | TokenScope::Readonly => Ok(()),
        TokenScope::Project(pid) => {
            if channel == pid || channel.starts_with(&format!("{pid}:")) {
                Ok(())
            } else {
                Err(FerroError::Forbidden(format!(
                    "project token '{pid}' cannot access realtime channel '{channel}'"
                )))
            }
        }
    }
}

impl PlatformService {
    pub async fn realtime_subscribe(
        &self,
        caller: &Caller,
        channel: &str,
    ) -> Result<(broadcast::Receiver<RealtimeMessage>, OwnedSemaphorePermit)> {
        let clean = validate_channel_name(channel)?;
        ensure_channel_scope(caller, &clean)?;
        self.realtime.subscribe(&clean).await
    }

    pub async fn realtime_publish(
        &self,
        caller: &Caller,
        channel: &str,
        event: Option<&str>,
        data: serde_json::Value,
    ) -> Result<RealtimeMessage> {
        if matches!(caller.scope, TokenScope::Readonly) {
            return Err(FerroError::Forbidden(
                "readonly scope cannot publish realtime messages".into(),
            ));
        }
        let clean = validate_channel_name(channel)?;
        ensure_channel_scope(caller, &clean)?;
        self.enforce_service_rate_limit(
            &format!("rt-pub:{clean}:{}", caller.actor),
            None,
            DEFAULT_REALTIME_PUBLISH_RPM,
            &format!("realtime channel '{clean}'"),
        )?;
        let payload_len = data.to_string().len();
        if payload_len > MAX_REALTIME_PAYLOAD_BYTES {
            return Err(FerroError::InvalidArgument(format!(
                "realtime payload size ({payload_len} bytes) exceeds maximum ({MAX_REALTIME_PAYLOAD_BYTES} bytes)"
            )));
        }
        let ev_name = event
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("message");
        self.realtime.publish(&clean, ev_name, data).await
    }

    pub async fn realtime_list_channels(
        &self,
        caller: &Caller,
    ) -> Result<Vec<RealtimeChannelInfo>> {
        if !caller.scope.allows_read() {
            return Err(FerroError::Forbidden("insufficient scope".into()));
        }
        Ok(self.realtime.list_channels(caller.project_id()).await)
    }
}
