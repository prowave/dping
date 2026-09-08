//! Turn per-target session stats into a plain-English diagnosis.
//!
//! `snapshots` is expected to be ordered gateway -> hop2 -> ... -> anchors,
//! i.e. walking outward from the local machine toward the internet, so "the
//! first target with loss" tells you which segment to blame.

use crate::stats::Snapshot;

/// Percent packet loss considered significant.
pub const LOSS_THRESHOLD: f64 = 2.0;
/// stdev of RTT (ms) considered "high" jitter.
pub const JITTER_THRESHOLD_MS: f64 = 30.0;

pub fn summarize(snapshots: &[Snapshot]) -> String {
    let header = format!(
        "{:<24}{:>6}{:>8}{:>9}{:>9}",
        "Target", "Sent", "Loss%", "Avg ms", "Jitter"
    );
    let mut lines = vec![header.clone(), "-".repeat(header.len())];
    for s in snapshots {
        let avg = s
            .avg_rtt
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "-".to_string());
        let jit = s
            .jitter
            .map(|v| format!("{v:.1}"))
            .unwrap_or_else(|| "-".to_string());
        lines.push(format!(
            "{:<24}{:>6}{:>7.1}%{:>9}{:>9}",
            s.name, s.sent, s.loss_pct_session, avg, jit
        ));
    }
    lines.join("\n")
}

pub fn diagnose(snapshots: &[Snapshot]) -> String {
    if snapshots.is_empty() || snapshots.iter().all(|s| s.sent == 0) {
        return "No data collected.".to_string();
    }

    let lossy: Vec<&Snapshot> = snapshots
        .iter()
        .filter(|s| s.sent > 0 && s.loss_pct_session > LOSS_THRESHOLD)
        .collect();

    if lossy.is_empty() {
        let jittery: Vec<&Snapshot> = snapshots
            .iter()
            .filter(|s| s.jitter.is_some_and(|j| j > JITTER_THRESHOLD_MS))
            .collect();
        if !jittery.is_empty() {
            let names = jittery
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            return format!(
                "No significant packet loss detected, but high jitter on {names}. \
That can still feel 'spotty' (voice/video breakup, stalls) even \
without hard drops - worth checking for congestion or bufferbloat."
            );
        }
        return "No significant packet loss or jitter detected during this run.".to_string();
    }

    let first_loss = lossy[0];
    // Identity comparison via pointer, mirroring Python's `first_loss is
    // stats_list[-1]` — first_loss is a reference into snapshots, so
    // std::ptr::eq against the last element is the faithful equivalent.
    let idx = snapshots
        .iter()
        .position(|s| std::ptr::eq(s, first_loss))
        .expect("first_loss came from snapshots");
    let upstream_of_first = &snapshots[..idx];

    let mut verdict = if idx == 0 {
        format!(
            "Loss starts right at {} ({}) - {:.1}% loss. That's your gateway itself; \
worth a second look even though it's new hardware.",
            first_loss.name, first_loss.target, first_loss.loss_pct_session
        )
    } else if snapshots.len() > 2
        && std::ptr::eq(first_loss, snapshots.last().unwrap())
        && upstream_of_first
            .iter()
            .all(|s| s.loss_pct_session <= LOSS_THRESHOLD)
    {
        format!(
            "Every hop up through your ISP looks clean, but the public anchor \
{} ({}) shows {:.1}% loss - likely a destination/peering \
issue rather than your connection.",
            first_loss.name, first_loss.target, first_loss.loss_pct_session
        )
    } else {
        format!(
            "Loss starts at {} ({}), {:.1}% - that's upstream of your gateway \
(ISP network / last-mile), not your local network.",
            first_loss.name, first_loss.target, first_loss.loss_pct_session
        )
    };

    let others = &lossy[1..];
    if !others.is_empty() {
        let extra = others
            .iter()
            .map(|s| format!("{} ({:.1}%)", s.name, s.loss_pct_session))
            .collect::<Vec<_>>()
            .join(", ");
        verdict.push_str(" Additional loss also seen at: ");
        verdict.push_str(&extra);
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(name: &str, target: &str, sent: u64, received: u64, jitter: Option<f64>) -> Snapshot {
        let loss_pct_session = if sent == 0 {
            0.0
        } else {
            100.0 * (sent - received) as f64 / sent as f64
        };
        Snapshot {
            name: name.to_string(),
            target: target.to_string(),
            sent,
            last_rtt: None,
            avg_rtt: None,
            jitter,
            loss_pct_session,
            loss_pct_window: 0.0,
            window: Default::default(),
        }
    }

    fn clean(name: &str, target: &str) -> Snapshot {
        snap(name, target, 100, 100, None)
    }

    fn lossy(name: &str, target: &str, loss_pct: f64) -> Snapshot {
        // sent=100, received chosen to produce exactly loss_pct
        let received = (100.0 - loss_pct).round() as u64;
        snap(name, target, 100, received, None)
    }

    #[test]
    fn no_data_collected() {
        let snaps = vec![snap("gateway", "10.0.0.1", 0, 0, None)];
        assert_eq!(diagnose(&snaps), "No data collected.");
        assert_eq!(diagnose(&[]), "No data collected.");
    }

    #[test]
    fn loss_at_gateway_idx0() {
        let snaps = vec![
            lossy("gateway", "10.0.0.1", 10.0),
            clean("cloudflare(1.1.1.1)", "1.1.1.1"),
            clean("google(8.8.8.8)", "8.8.8.8"),
        ];
        let out = diagnose(&snaps);
        assert!(
            out.starts_with("Loss starts right at gateway (10.0.0.1) - 10.0% loss."),
            "{out}"
        );
    }

    #[test]
    fn last_entry_anchor_only_branch() {
        let snaps = vec![
            clean("gateway", "10.0.0.1"),
            clean("hop2(10.0.0.2)", "10.0.0.2"),
            lossy("google(8.8.8.8)", "8.8.8.8", 15.0),
        ];
        let out = diagnose(&snaps);
        assert!(
            out.starts_with("Every hop up through your ISP looks clean, but the public anchor google(8.8.8.8) (8.8.8.8) shows 15.0% loss"),
            "{out}"
        );
    }

    #[test]
    fn last_entry_lossy_but_earlier_hop_also_lossy_falls_through_to_generic() {
        let snaps = vec![
            clean("gateway", "10.0.0.1"),
            lossy("hop2(10.0.0.2)", "10.0.0.2", 5.0),
            lossy("google(8.8.8.8)", "8.8.8.8", 15.0),
        ];
        let out = diagnose(&snaps);
        // first_loss is hop2 (path order), idx=1, not 0, and hop2 is not the
        // last entry, so this must be the generic middle-hop wording.
        assert!(
            out.starts_with("Loss starts at hop2(10.0.0.2) (10.0.0.2), 5.0% - that's upstream"),
            "{out}"
        );
        assert!(
            out.contains("Additional loss also seen at: google(8.8.8.8) (15.0%)"),
            "{out}"
        );
    }

    #[test]
    fn len_two_edge_case_does_not_take_anchor_branch() {
        let snaps = vec![
            clean("gateway", "10.0.0.1"),
            lossy("cloudflare(1.1.1.1)", "1.1.1.1", 10.0),
        ];
        let out = diagnose(&snaps);
        // len(stats_list) > 2 guard fails (len==2), so even though the last
        // entry is the lossy one, it must fall to the generic wording.
        assert!(
            out.starts_with(
                "Loss starts at cloudflare(1.1.1.1) (1.1.1.1), 10.0% - that's upstream"
            ),
            "{out}"
        );
        assert!(!out.contains("Every hop up through your ISP"));
    }

    #[test]
    fn generic_middle_hop_branch() {
        let snaps = vec![
            clean("gateway", "10.0.0.1"),
            lossy("hop2(10.0.0.2)", "10.0.0.2", 8.0),
            clean("cloudflare(1.1.1.1)", "1.1.1.1"),
            clean("google(8.8.8.8)", "8.8.8.8"),
        ];
        let out = diagnose(&snaps);
        assert!(
            out.starts_with(
                "Loss starts at hop2(10.0.0.2) (10.0.0.2), 8.0% - that's upstream of your gateway"
            ),
            "{out}"
        );
    }

    #[test]
    fn jitter_only_wording_multi_name() {
        let snaps = vec![
            snap("gateway", "10.0.0.1", 100, 100, Some(40.0)),
            snap("hop2(10.0.0.2)", "10.0.0.2", 100, 100, Some(5.0)),
            snap("cloudflare(1.1.1.1)", "1.1.1.1", 100, 100, Some(35.0)),
        ];
        let out = diagnose(&snaps);
        assert!(out.starts_with("No significant packet loss detected, but high jitter on gateway, cloudflare(1.1.1.1)."), "{out}");
    }

    #[test]
    fn clean_run_wording() {
        let snaps = vec![
            clean("gateway", "10.0.0.1"),
            clean("cloudflare(1.1.1.1)", "1.1.1.1"),
        ];
        assert_eq!(
            diagnose(&snaps),
            "No significant packet loss or jitter detected during this run."
        );
    }

    #[test]
    fn multi_lossy_additional_suffix_in_path_order() {
        let snaps = vec![
            lossy("gateway", "10.0.0.1", 5.0),
            lossy("hop2(10.0.0.2)", "10.0.0.2", 8.0),
            clean("cloudflare(1.1.1.1)", "1.1.1.1"),
        ];
        let out = diagnose(&snaps);
        assert!(out.starts_with("Loss starts right at gateway (10.0.0.1) - 5.0% loss."));
        assert!(out.ends_with("Additional loss also seen at: hop2(10.0.0.2) (8.0%)"));
    }

    #[test]
    fn summarize_header_and_row_widths() {
        let snaps = vec![clean("gateway", "10.0.0.1")];
        let out = summarize(&snaps);
        assert!(out.starts_with("Target                    Sent   Loss%   Avg ms   Jitter"));
    }
}
