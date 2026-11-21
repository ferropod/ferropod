use std::time::Duration;

use tokio::sync::watch;
use tracing::debug;

use crate::core::service::PlatformService;

/// Periodically cleans expired KV TTLs, reaps expired queue leases, expires old jobs/DLQ,
/// unlinks due `blob_gc` files, and drops old daily log databases.
pub async fn run_maintenance_loop(
    svc: PlatformService,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(
        svc.config().maintenance_interval_secs.max(5),
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                svc.run_maintenance_once().await;
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
