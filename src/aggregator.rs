//! The aggregator task is the sole owner of every target's `TargetStats` —
//! no shared mutex needed. It consumes ping samples from all ping tasks over
//! one channel, publishes a `Vec<Snapshot>` once a second for the UI, and
//! (optionally) forwards individual samples to the CSV logger task.

use tokio::sync::{mpsc, watch};
use tokio::time::{self, Duration};

use crate::pinger::PingSample;
use crate::stats::{Snapshot, TargetStats};

/// One CSV row's worth of data, timestamped when the sample was recorded.
pub struct LogSample {
    pub timestamp: f64,
    pub target: String,
    pub name: String,
    pub rtt: Option<f64>,
}

/// Runs until every ping task has dropped its `PingSample` sender (i.e. all
/// have observed cancellation and exited), then returns the final
/// `Vec<Snapshot>` for `analyzer::{summarize,diagnose}`. Takes ownership of
/// the already-constructed `TargetStats` (the aggregator is their sole
/// owner for the rest of the run — no `Arc`/`Mutex` needed).
pub async fn run_aggregator(
    mut stats: Vec<TargetStats>,
    mut rx: mpsc::UnboundedReceiver<PingSample>,
    snapshot_tx: watch::Sender<Vec<Snapshot>>,
    log_tx: Option<mpsc::UnboundedSender<LogSample>>,
) -> Vec<Snapshot> {
    let mut ticker = time::interval(Duration::from_secs(1));
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            sample = rx.recv() => {
                match sample {
                    Some(PingSample { idx, rtt }) => {
                        stats[idx].record(rtt);
                        if let Some(log_tx) = &log_tx {
                            let _ = log_tx.send(LogSample {
                                timestamp: now_unix_secs(),
                                target: stats[idx].target.clone(),
                                name: stats[idx].name.clone(),
                                rtt,
                            });
                        }
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {
                let _ = snapshot_tx.send(stats.iter().map(TargetStats::snapshot).collect());
            }
        }
    }

    let final_snapshots: Vec<Snapshot> = stats.iter().map(TargetStats::snapshot).collect();
    let _ = snapshot_tx.send(final_snapshots.clone());
    final_snapshots
}

fn now_unix_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
