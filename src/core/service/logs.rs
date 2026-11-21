use futures_util::stream::Stream;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::core::model::{Caller, FerroError, LogEntry, LogQuery, Result, TokenScope};

use super::PlatformService;

/// Scoped live log subscription backed by `LogStore::subscribe()`.
pub struct LogSubscription {
    rx: broadcast::Receiver<LogEntry>,
    project_filter: Option<String>,
}

impl LogSubscription {
    fn matches(entry: &LogEntry, project_filter: Option<&str>) -> bool {
        match project_filter {
            Some(pid) => entry.project_id.as_deref() == Some(pid),
            None => true,
        }
    }

    /// Receive the next matching log entry, skipping lagged messages and entries outside scope.
    pub async fn recv(&mut self) -> Option<LogEntry> {
        loop {
            match self.rx.recv().await {
                Ok(entry) => {
                    if Self::matches(&entry, self.project_filter.as_deref()) {
                        return Some(entry);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Convert into an async stream for SSE live tailing.
    pub fn into_stream(self) -> impl Stream<Item = LogEntry> {
        let filter = self.project_filter;
        BroadcastStream::new(self.rx).filter_map(move |item| match item {
            Ok(entry) if Self::matches(&entry, filter.as_deref()) => Some(entry),
            _ => None,
        })
    }
}

impl PlatformService {
    pub async fn query_logs(&self, caller: &Caller, mut q: LogQuery) -> Result<Vec<LogEntry>> {
        if let TokenScope::Project(pid) = &caller.scope {
            if let Some(req_pid) = q.project_id.as_deref() {
                if req_pid != pid {
                    return Err(FerroError::Forbidden(format!(
                        "log access denied for project '{req_pid}'"
                    )));
                }
            } else {
                q.project_id = Some(pid.clone());
            }
        }
        self.logs.query(q).await
    }

    pub fn subscribe_logs(&self, caller: &Caller) -> LogSubscription {
        let project_filter = caller.project_id().map(str::to_string);
        LogSubscription {
            rx: self.logs.subscribe(),
            project_filter,
        }
    }
}
