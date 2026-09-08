mod aggregator;
mod analyzer;
mod cli;
mod discovery;
mod logger;
mod pinger;
mod platform;
mod stats;
mod target;
mod ui;

use std::io::IsTerminal;
use std::time::Duration;

use clap::Parser;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use aggregator::LogSample;
use cli::Args;
use pinger::PingSample;
use stats::TargetStats;

#[tokio::main]
async fn main() {
    let code = run().await;
    std::process::exit(code);
}

async fn run() -> i32 {
    let args = Args::parse();
    let targets = target::build_targets(&args).await;
    if targets.is_empty() {
        eprintln!("no targets to ping - aborting");
        return 1;
    }

    let stats: Vec<TargetStats> = targets
        .into_iter()
        .map(|t| TargetStats::new(t.name, t.address, args.window))
        .collect();
    let initial_snapshots = stats.iter().map(TargetStats::snapshot).collect();

    let (snapshot_tx, snapshot_rx) = watch::channel(initial_snapshots);
    let (ping_tx, ping_rx) = mpsc::unbounded_channel::<PingSample>();

    let mut log_tx_for_aggregator: Option<mpsc::UnboundedSender<LogSample>> = None;
    let mut logger_handle = None;
    if let Some(path) = args.log.clone() {
        let (tx, rx) = mpsc::unbounded_channel();
        match logger::spawn_logger(path, rx) {
            Ok(handle) => {
                log_tx_for_aggregator = Some(tx);
                logger_handle = Some(handle);
            }
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        }
    }

    let cancel = CancellationToken::new();
    let interval = Duration::from_secs_f64(args.interval.max(0.0));
    let timeout = Duration::from_secs_f64(args.timeout.max(0.0));

    let mut ping_handles = Vec::with_capacity(stats.len());
    for (idx, s) in stats.iter().enumerate() {
        let target = s.target.clone();
        let tx = ping_tx.clone();
        let cancel = cancel.clone();
        ping_handles.push(tokio::spawn(pinger::ping_loop(
            idx, target, interval, timeout, cancel, tx,
        )));
    }
    // Drop the original sender so `ping_rx` only stays open as long as at
    // least one ping task is still running.
    drop(ping_tx);

    let aggregator_handle = tokio::spawn(aggregator::run_aggregator(
        stats,
        ping_rx,
        snapshot_tx,
        log_tx_for_aggregator,
    ));

    if let Some(duration) = args.duration {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(duration.max(0.0))).await;
            cancel.cancel();
        });
    }

    // Belt-and-suspenders: covers --plain mode (no raw terminal mode, so a
    // real SIGINT still reaches the process normally). The dashboard's raw
    // mode disables signal generation for Ctrl+C, so it matches the
    // keypress explicitly instead (see ui::dashboard::should_quit).
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.cancel();
            }
        });
    }

    let use_plain = args.plain || !std::io::stdout().is_terminal();
    if use_plain {
        ui::plain::run_plain(snapshot_rx, cancel.clone()).await;
    } else if let Err(e) = ui::dashboard::run_dashboard(snapshot_rx, cancel.clone()).await {
        eprintln!("dashboard error: {e}");
    }

    // Idempotent: covers the case where the UI exited via some path other
    // than the cancellation token (e.g. a dashboard I/O error above).
    cancel.cancel();

    let join_deadline = Duration::from_secs_f64(args.timeout.max(0.0) + 2.0);
    let _ = tokio::time::timeout(join_deadline, futures::future::join_all(ping_handles)).await;

    let final_snapshots = match aggregator_handle.await {
        Ok(snapshots) => snapshots,
        Err(e) => {
            eprintln!("warning: stats aggregator task failed: {e}");
            Vec::new()
        }
    };

    if let Some(handle) = logger_handle {
        let _ = handle.await;
    }

    println!();
    println!("{}", analyzer::summarize(&final_snapshots));
    println!();
    println!("{}", analyzer::diagnose(&final_snapshots));

    0
}
