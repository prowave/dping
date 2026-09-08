//! CSV logger task: one row per individual ping sample (not per UI
//! snapshot), matching the Python original's `on_sample` callback exactly.
//! Runs on a blocking task since `csv`/`File` I/O is synchronous; the
//! channel from the aggregator is unbounded so slow disk I/O never applies
//! backpressure to ping tasks or the aggregator.

use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::aggregator::LogSample;

/// Opens `path` and writes the CSV header immediately (so a run that's
/// cancelled before any sample arrives still produces a valid, header-only
/// file). Returns the task handle to join at shutdown, and `None` (with an
/// error printed) if the file couldn't be opened — matching Python's
/// behavior of letting an unopenable `--log` path fail loudly at startup
/// rather than silently continuing without logging.
pub fn spawn_logger(
    path: PathBuf,
    mut rx: mpsc::UnboundedReceiver<LogSample>,
) -> anyhow::Result<JoinHandle<()>> {
    let file = std::fs::File::create(&path)
        .map_err(|e| anyhow::anyhow!("could not open --log file {}: {e}", path.display()))?;

    let handle = tokio::task::spawn_blocking(move || {
        let mut writer = csv::Writer::from_writer(file);
        let _ = writer.write_record(["timestamp", "target", "name", "rtt_ms"]);
        let _ = writer.flush();
        while let Some(sample) = rx.blocking_recv() {
            let rtt_field = sample.rtt.map(|v| v.to_string()).unwrap_or_default();
            let _ = writer.write_record([
                sample.timestamp.to_string(),
                sample.target,
                sample.name,
                rtt_field,
            ]);
            let _ = writer.flush();
        }
    });

    Ok(handle)
}
