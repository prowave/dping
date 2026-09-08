//! Single-ping execution with a two-layer timeout, plus the per-target
//! continuous ping loop run as a tokio task.

use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::platform::active;

/// Runs a single ping and returns the RTT in ms, or `None` if it was lost.
/// Two layers of timeout, matching the Python original: the OS ping
/// binary's own timeout flag, plus an outer guard here (`timeout + 2s`) that
/// force-kills a hung/misbehaving subprocess. Any spawn error (e.g. missing
/// `ping` binary) is treated as a lost ping, never propagated.
pub async fn ping_once(target: &str, timeout: Duration) -> Option<f64> {
    let mut cmd = active::ping_command(target, timeout);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::null());
    // Matches the Python original's explicit proc.kill()+proc.wait() on
    // outer-timeout expiry: if the outer tokio::time::timeout below fires,
    // the in-flight wait_with_output() future (and the Child it owns) is
    // dropped — kill_on_drop ensures that actually kills the subprocess
    // instead of leaving an orphaned ping process behind.
    cmd.kill_on_drop(true);

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None,
    };

    let outer_timeout = timeout + Duration::from_secs(2);
    match tokio::time::timeout(outer_timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            active::parse_ping_rtt(&stdout)
        }
        // Process error, or the outer timeout fired (kill_on_drop above
        // handles killing the subprocess in that case) — either way, lost.
        _ => None,
    }
}

/// One sample emitted by a `ping_loop` task, consumed by the aggregator.
pub struct PingSample {
    pub idx: usize,
    pub rtt: Option<f64>,
}

/// Pings `target` every `interval` until `cancel` fires. Racing the
/// ping+sleep against cancellation (rather than polling a flag, as the
/// Python original does) means a quit doesn't have to wait out a stale
/// interval sleep.
pub async fn ping_loop(
    idx: usize,
    target: String,
    interval: Duration,
    timeout: Duration,
    cancel: CancellationToken,
    tx: tokio::sync::mpsc::UnboundedSender<PingSample>,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = async {
                let start = Instant::now();
                let rtt = ping_once(&target, timeout).await;
                let _ = tx.send(PingSample { idx, rtt });
                let elapsed = start.elapsed();
                let remaining = interval.saturating_sub(elapsed);
                if !remaining.is_zero() {
                    tokio::time::sleep(remaining).await;
                }
            } => {}
        }
    }
}
