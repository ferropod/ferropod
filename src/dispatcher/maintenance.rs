use std::time::Duration;

use tokio::sync::watch;
use tracing::{debug, warn};

use crate::core::service::PlatformService;

/// Periodically runs maintenance (KV TTL, queue reaper/retention, blob GC, log rotation),
/// polls queue consumers, and triggers due cron jobs.
pub async fn run_maintenance_loop(
    svc: PlatformService,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut maint_interval = tokio::time::interval(Duration::from_secs(
        svc.config().maintenance_interval_secs.max(5),
    ));
    maint_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut dispatch_interval = tokio::time::interval(Duration::from_secs(1));
    dispatch_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = maint_interval.tick() => {
                svc.run_maintenance_once().await;
            }
            _ = dispatch_interval.tick() => {
                if let Err(e) = svc.run_queue_dispatch_once().await {
                    warn!(error = %e, "queue dispatcher tick failed");
                }
                if let Err(e) = svc.run_due_cron_jobs_once().await {
                    warn!(error = %e, "cron scheduler tick failed");
                }
            }
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    debug!("maintenance loop stopping for shutdown");
                    break;
                }
            }
        }
    }
}
