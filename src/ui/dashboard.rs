//! The live ratatui+crossterm dashboard — replaces the Python Textual
//! `NetdiagApp`. Redraws once a second (independent of `--interval`),
//! quits on `q`/`Esc`/Ctrl+C.

use std::collections::VecDeque;
use std::io::{self, Stdout};
use std::time::Duration;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::stats::Snapshot;
use crate::ui::{
    BucketColor, JITTER_BAD_MS, JITTER_WARN_MS, LATENCY_BAD_MS, LATENCY_WARN_MS, LOSS_BAD,
    LOSS_WARN, SPARK_CHARS, bucket_color,
};

const BG: Color = Color::Rgb(0x0a, 0x0e, 0x14);
const PANEL_BG: Color = Color::Rgb(0x10, 0x15, 0x1c);
const GREEN: Color = Color::Rgb(0x39, 0xff, 0x88);
const BLUE: Color = Color::Rgb(0x39, 0xc5, 0xff);
const YELLOW: Color = Color::Rgb(0xf8, 0xe4, 0x5c);
const RED: Color = Color::Rgb(0xff, 0x5f, 0x5f);

/// RAII guard so raw mode / the alternate screen are restored even if the
/// dashboard loop panics — the run must always fall through to the final
/// summary/diagnosis, never leave the terminal in raw mode with a
/// backtrace dumped into it.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

pub async fn run_dashboard(
    mut snapshot_rx: watch::Receiver<Vec<Snapshot>>,
    cancel: CancellationToken,
) -> io::Result<()> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let started_at = Instant::now();
    let mut events = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(1));

    loop {
        let snapshots = snapshot_rx.borrow().clone();
        draw(&mut terminal, &snapshots, started_at)?;

        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = ticker.tick() => {}
            _ = snapshot_rx.changed() => {}
            maybe_event = events.next() => {
                if should_quit(maybe_event) {
                    cancel.cancel();
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Raw mode intercepts Ctrl+C as an ordinary keypress rather than letting it
/// generate SIGINT, so Ctrl+C must be matched explicitly here alongside
/// `q`/`Esc` (mirroring the Python original's `action_quit` binding).
fn should_quit(maybe_event: Option<io::Result<Event>>) -> bool {
    let Some(Ok(Event::Key(key))) = maybe_event else {
        return false;
    };
    if key.kind != KeyEventKind::Press {
        return false;
    }
    let is_ctrl_c = key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL);
    matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) || is_ctrl_c
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    snapshots: &[Snapshot],
    started_at: Instant,
) -> io::Result<()> {
    terminal.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        frame.render_widget(status_bar(snapshots, started_at), chunks[0]);
        frame.render_widget(table(snapshots), chunks[1]);
    })?;
    Ok(())
}

fn status_bar(snapshots: &[Snapshot], started_at: Instant) -> Paragraph<'static> {
    let elapsed = started_at.elapsed().as_secs();
    let sent: u64 = snapshots.iter().map(|s| s.sent).sum();
    let down = snapshots
        .iter()
        .filter(|s| s.last_rtt.is_none() && s.sent > 0)
        .count();
    let now = chrono::Local::now().format("%H:%M:%S");
    let status = if down == 0 {
        "ALL HOPS UP".to_string()
    } else {
        format!("{down} HOP(S) DOWN")
    };
    let text = format!("● live  {now}  uptime {elapsed:>6}s  packets sent {sent:>6}  {status}");
    let style = if down == 0 {
        Style::default()
            .fg(GREEN)
            .bg(PANEL_BG)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(RED)
            .bg(PANEL_BG)
            .add_modifier(Modifier::BOLD)
    };
    Paragraph::new(text).style(style)
}

fn table(snapshots: &[Snapshot]) -> Table<'static> {
    let header = Row::new(vec![
        "",
        "Target",
        "Loss(win)",
        "Loss(all)",
        "Last",
        "Avg",
        "Jitter",
        "History",
    ])
    .style(
        Style::default()
            .fg(BLUE)
            .bg(PANEL_BG)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row<'static>> = snapshots.iter().map(build_row).collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Length(20),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Min(10),
    ];

    Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(BLUE))
                .title(" dping \u{2014} live packet-loss / latency dashboard "),
        )
        .style(Style::default().bg(BG))
}

fn build_row(s: &Snapshot) -> Row<'static> {
    let up = s.last_rtt.is_some();
    let dot_style = if up {
        Style::default().fg(GREEN).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(RED)
            .add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)
    };
    let name_style = if up {
        Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(RED).add_modifier(Modifier::BOLD)
    };

    let loss_win_style = style_for(bucket_color(Some(s.loss_pct_window), LOSS_WARN, LOSS_BAD));
    let loss_all_style = style_for(bucket_color(Some(s.loss_pct_session), LOSS_WARN, LOSS_BAD));

    let (last_text, last_style) = match s.last_rtt {
        Some(v) => (
            format!("{v:.1}"),
            style_for(bucket_color(Some(v), LATENCY_WARN_MS, LATENCY_BAD_MS)),
        ),
        None => (
            "loss".to_string(),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ),
    };
    let avg_text = s
        .avg_rtt
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "-".to_string());
    let avg_style = style_for(bucket_color(s.avg_rtt, LATENCY_WARN_MS, LATENCY_BAD_MS));
    let jitter_text = s
        .jitter
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "-".to_string());
    let jitter_style = style_for(bucket_color(s.jitter, JITTER_WARN_MS, JITTER_BAD_MS));

    let history = colored_sparkline(&s.window);

    Row::new(vec![
        Cell::from("●").style(dot_style),
        Cell::from(s.name.clone()).style(name_style),
        Cell::from(format!("{:.1}%", s.loss_pct_window)).style(loss_win_style),
        Cell::from(format!("{:.1}%", s.loss_pct_session)).style(loss_all_style),
        Cell::from(last_text).style(last_style),
        Cell::from(avg_text).style(avg_style),
        Cell::from(jitter_text).style(jitter_style),
        Cell::from(history),
    ])
}

fn style_for(color: BucketColor) -> Style {
    match color {
        BucketColor::Dim => Style::default().add_modifier(Modifier::DIM),
        BucketColor::Good => Style::default().fg(GREEN).add_modifier(Modifier::BOLD),
        BucketColor::Warn => Style::default().fg(YELLOW).add_modifier(Modifier::BOLD),
        BucketColor::Bad => Style::default().fg(RED).add_modifier(Modifier::BOLD),
    }
}

/// A heat-strip sparkline: each block colored green -> yellow -> red by how
/// high that sample sits within the window's own min/max range (not
/// against absolute thresholds), matching the Python original's
/// `_rich_sparkline`.
fn colored_sparkline(values: &VecDeque<Option<f64>>) -> Line<'static> {
    let samples: Vec<f64> = values.iter().filter_map(|v| *v).collect();
    if samples.is_empty() {
        return Line::from("");
    }
    let lo = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = if hi - lo == 0.0 { 1.0 } else { hi - lo };

    let spans: Vec<Span<'static>> = values
        .iter()
        .map(|v| match v {
            None => Span::styled("x", Style::default().fg(RED).add_modifier(Modifier::BOLD)),
            Some(v) => {
                let raw_idx = (((v - lo) / span) * (SPARK_CHARS.len() - 1) as f64) as usize;
                let idx = raw_idx.min(SPARK_CHARS.len() - 1);
                let frac = idx as f64 / (SPARK_CHARS.len() - 1) as f64;
                let color = if frac < 0.33 {
                    GREEN
                } else if frac < 0.66 {
                    YELLOW
                } else {
                    RED
                };
                Span::styled(SPARK_CHARS[idx].to_string(), Style::default().fg(color))
            }
        })
        .collect();
    Line::from(spans)
}
