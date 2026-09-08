//! `--plain` (also auto-selected when stdout isn't a TTY): print a
//! timestamped snapshot table once a second until cancelled.

use std::io::Write;
use std::time::Duration;

use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::stats::Snapshot;
use crate::ui::{format_row, header_line};

pub async fn run_plain(snapshot_rx: watch::Receiver<Vec<Snapshot>>, cancel: CancellationToken) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(Duration::from_secs(1)) => {}
        }

        let snapshots = snapshot_rx.borrow().clone();
        let width = crossterm::terminal::size()
            .map(|(cols, _)| cols as usize)
            .unwrap_or(100);
        let now = chrono::Local::now().format("%H:%M:%S");
        println!("\n{now}  {}", "-".repeat(width.saturating_sub(10).max(10)));
        println!("{}", header_line());
        for s in &snapshots {
            println!("{}", format_row(s));
        }
        let _ = std::io::stdout().flush();
    }
}
