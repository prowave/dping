//! Shared display helpers for both UI modes (plain-text and the live
//! dashboard). Thresholds here are the loose, "does it look bad at a
//! glance" set — intentionally separate from `analyzer.rs`'s stricter
//! session-wide diagnosis thresholds; do not unify them.

pub mod dashboard;
pub mod plain;

use crate::stats::Snapshot;

pub const SPARK_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

pub const LOSS_WARN: f64 = 0.0;
pub const LOSS_BAD: f64 = 5.0;
pub const LATENCY_WARN_MS: f64 = 20.0;
pub const LATENCY_BAD_MS: f64 = 50.0;
pub const JITTER_WARN_MS: f64 = 10.0;
pub const JITTER_BAD_MS: f64 = 30.0;

pub fn header_line() -> String {
    format!(
        "{:<20}{:>10}{:>10}{:>8}{:>8}{:>8}  History",
        "Target", "Loss(win)", "Loss(all)", "Last", "Avg", "Jitter"
    )
}

/// Render RTT samples (`None` = loss) as a compact block-character trend,
/// scaled to the window's own min/max range. Empty string if every sample
/// in `values` is a loss (or there are none).
pub fn sparkline(values: &std::collections::VecDeque<Option<f64>>, lost_char: char) -> String {
    let samples: Vec<f64> = values.iter().filter_map(|v| *v).collect();
    if samples.is_empty() {
        return String::new();
    }
    let lo = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = if hi - lo == 0.0 { 1.0 } else { hi - lo };

    values
        .iter()
        .map(|v| match v {
            None => lost_char,
            Some(v) => {
                let idx = (((v - lo) / span) * (SPARK_CHARS.len() - 1) as f64) as usize;
                SPARK_CHARS[idx.min(SPARK_CHARS.len() - 1)]
            }
        })
        .collect()
}

pub fn format_row(s: &Snapshot) -> String {
    let last = s
        .last_rtt
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "loss".to_string());
    let avg = s
        .avg_rtt
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "-".to_string());
    let jit = s
        .jitter
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "-".to_string());
    let spark = sparkline(&s.window, 'x');
    format!(
        "{:<20}{:>9.1}%{:>9.1}%{:>8}{:>8}{:>8}  {}",
        s.name, s.loss_pct_window, s.loss_pct_session, last, avg, jit, spark
    )
}

/// green / yellow / red bucket for a value against warn/bad thresholds,
/// used by the dashboard's cell colorizers.
pub fn bucket_color(value: Option<f64>, warn: f64, bad: f64) -> BucketColor {
    match value {
        None => BucketColor::Dim,
        Some(v) if v <= warn => BucketColor::Good,
        Some(v) if v <= bad => BucketColor::Warn,
        Some(_) => BucketColor::Bad,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketColor {
    Dim,
    Good,
    Warn,
    Bad,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn sparkline_empty_when_all_lost() {
        let values: VecDeque<Option<f64>> = VecDeque::from([None, None]);
        assert_eq!(sparkline(&values, 'x'), "");
    }

    #[test]
    fn sparkline_marks_loss_and_scales_range() {
        let values: VecDeque<Option<f64>> = VecDeque::from([Some(0.0), None, Some(10.0)]);
        let out = sparkline(&values, 'x');
        assert_eq!(out.chars().next(), Some(SPARK_CHARS[0]));
        assert_eq!(out.chars().nth(1), Some('x'));
        assert_eq!(out.chars().nth(2), Some(SPARK_CHARS[7]));
    }

    #[test]
    fn sparkline_flat_values_use_first_char() {
        let values: VecDeque<Option<f64>> = VecDeque::from([Some(5.0), Some(5.0)]);
        let out = sparkline(&values, 'x');
        assert_eq!(out, format!("{}{}", SPARK_CHARS[0], SPARK_CHARS[0]));
    }

    #[test]
    fn bucket_color_thresholds() {
        assert_eq!(bucket_color(None, 0.0, 5.0), BucketColor::Dim);
        assert_eq!(bucket_color(Some(0.0), 0.0, 5.0), BucketColor::Good);
        assert_eq!(bucket_color(Some(3.0), 0.0, 5.0), BucketColor::Warn);
        assert_eq!(bucket_color(Some(6.0), 0.0, 5.0), BucketColor::Bad);
    }
}
