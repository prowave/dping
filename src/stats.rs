use std::collections::VecDeque;

/// Tracks ping results for a single target: a rolling window for "recent"
/// stats plus session-wide totals for the end-of-run summary.
///
/// `jitter`/`avg_rtt` are computed over the unbounded, session-lifetime
/// `rtts` list, never the rolling `window` — they keep reflecting the whole
/// run even after old samples have been evicted from the window.
pub struct TargetStats {
    pub name: String,
    pub target: String,
    window: VecDeque<Option<f64>>,
    window_cap: usize,
    sent: u64,
    received: u64,
    rtts: Vec<f64>,
    last_rtt: Option<f64>,
}

impl TargetStats {
    pub fn new(name: String, target: String, window_cap: usize) -> Self {
        TargetStats {
            name,
            target,
            window: VecDeque::with_capacity(window_cap.max(1)),
            window_cap: window_cap.max(1),
            sent: 0,
            received: 0,
            rtts: Vec::new(),
            last_rtt: None,
        }
    }

    pub fn record(&mut self, rtt: Option<f64>) {
        self.sent += 1;
        self.window.push_back(rtt);
        if self.window.len() > self.window_cap {
            self.window.pop_front();
        }
        if let Some(v) = rtt {
            self.received += 1;
            self.rtts.push(v);
        }
        self.last_rtt = rtt;
    }

    pub fn loss_pct_session(&self) -> f64 {
        if self.sent == 0 {
            0.0
        } else {
            100.0 * (self.sent - self.received) as f64 / self.sent as f64
        }
    }

    pub fn loss_pct_window(&self) -> f64 {
        if self.window.is_empty() {
            return 0.0;
        }
        let lost = self.window.iter().filter(|r| r.is_none()).count();
        100.0 * lost as f64 / self.window.len() as f64
    }

    pub fn avg_rtt(&self) -> Option<f64> {
        if self.rtts.is_empty() {
            None
        } else {
            Some(self.rtts.iter().sum::<f64>() / self.rtts.len() as f64)
        }
    }

    /// Population standard deviation of every session RTT (statistics.pstdev
    /// equivalent) — requires more than one sample, matching the Python original.
    pub fn jitter(&self) -> Option<f64> {
        if self.rtts.len() <= 1 {
            return None;
        }
        let mean = self.avg_rtt().unwrap();
        let variance =
            self.rtts.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / self.rtts.len() as f64;
        Some(variance.sqrt())
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            name: self.name.clone(),
            target: self.target.clone(),
            sent: self.sent,
            last_rtt: self.last_rtt,
            avg_rtt: self.avg_rtt(),
            jitter: self.jitter(),
            loss_pct_session: self.loss_pct_session(),
            loss_pct_window: self.loss_pct_window(),
            window: self.window.clone(),
        }
    }
}

/// A cheap, cloneable point-in-time view of a `TargetStats`, used by the UI
/// (via a `watch` channel) and returned as the aggregator's final state for
/// `analyzer::{summarize,diagnose}`.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub name: String,
    pub target: String,
    pub sent: u64,
    pub last_rtt: Option<f64>,
    pub avg_rtt: Option<f64>,
    pub jitter: Option<f64>,
    pub loss_pct_session: f64,
    pub loss_pct_window: f64,
    pub window: VecDeque<Option<f64>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_eviction_respects_cap() {
        let mut s = TargetStats::new("t".into(), "1.2.3.4".into(), 3);
        for i in 0..5 {
            s.record(Some(i as f64));
        }
        let window = s.snapshot().window;
        assert_eq!(window.len(), 3);
        assert_eq!(
            window.iter().copied().collect::<Vec<_>>(),
            vec![Some(2.0), Some(3.0), Some(4.0)]
        );
    }

    #[test]
    fn jitter_and_avg_reflect_full_session_after_window_eviction() {
        let mut s = TargetStats::new("t".into(), "1.2.3.4".into(), 2);
        // Five samples, window cap 2 -> window only keeps the last two, but
        // rtts (and thus avg/jitter) must reflect all five.
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            s.record(Some(v));
        }
        assert_eq!(s.snapshot().window.len(), 2);
        assert_eq!(s.avg_rtt(), Some(30.0));

        // pstdev of [10,20,30,40,50] is sqrt(200) ~= 14.1421356
        let jitter = s.jitter().unwrap();
        assert!((jitter - 200f64.sqrt()).abs() < 1e-9);

        // Sanity: this must differ from the stdev of just the window contents [40,50].
        let window_only_mean: f64 = 45.0;
        let window_only_var =
            ((40.0 - window_only_mean).powi(2) + (50.0 - window_only_mean).powi(2)) / 2.0;
        assert!((jitter - window_only_var.sqrt()).abs() > 1e-6);
    }

    #[test]
    fn loss_pct_window_vs_session_diverge_once_losses_leave_window() {
        let mut s = TargetStats::new("t".into(), "1.2.3.4".into(), 2);
        s.record(None); // lost, sent=1
        s.record(None); // lost, sent=2
        s.record(Some(5.0)); // ok, sent=3 -> window now [None, Some(5.0)]
        s.record(Some(5.0)); // ok, sent=4 -> window now [Some(5.0), Some(5.0)]

        assert_eq!(s.loss_pct_window(), 0.0);
        assert_eq!(s.loss_pct_session(), 50.0);
    }

    #[test]
    fn jitter_none_with_one_or_zero_samples() {
        let mut s = TargetStats::new("t".into(), "1.2.3.4".into(), 5);
        assert_eq!(s.jitter(), None);
        s.record(Some(10.0));
        assert_eq!(s.jitter(), None);
        s.record(Some(20.0));
        assert!(s.jitter().is_some());
    }
}
