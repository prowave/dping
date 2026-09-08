use std::path::PathBuf;

use clap::Parser;

/// Diagnose where packet loss / latency spikes occur between you and the internet.
#[derive(Parser, Debug)]
#[command(name = "dping")]
pub struct Args {
    /// Comma-separated extra hosts/IPs to include
    #[arg(long)]
    pub targets: Option<String>,

    /// Seconds between pings per target
    #[arg(long, default_value_t = 1.0)]
    pub interval: f64,

    /// Rolling sample window for 'recent loss' stats
    #[arg(long, default_value_t = 60)]
    pub window: usize,

    /// Per-ping timeout in seconds
    #[arg(long, default_value_t = 1.0)]
    pub timeout: f64,

    /// Skip auto hop discovery; just ping gateway + anchors + --targets
    #[arg(long = "no-traceroute", default_value_t = false)]
    pub no_traceroute: bool,

    /// Auto-stop after N seconds (default: run until Ctrl+C / q)
    #[arg(long)]
    pub duration: Option<f64>,

    /// Write a CSV of every ping sample to this path
    #[arg(long)]
    pub log: Option<PathBuf>,

    /// Print periodic snapshots instead of the live dashboard
    #[arg(long, default_value_t = false)]
    pub plain: bool,
}
