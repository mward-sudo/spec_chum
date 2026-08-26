//! Cheap rolling perf counters for the headless living-room embed.
//!
//! Always records (a few integers per tick). Logging / HUD is gated by
//! `SPEC_CHUM_ROOM_PERF=1` (or non-empty).

use std::env;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

static PERF_LOG: AtomicBool = AtomicBool::new(false);
static PERF_LOG_INIT: AtomicBool = AtomicBool::new(false);

fn perf_log_enabled() -> bool {
    if !PERF_LOG_INIT.swap(true, Ordering::Relaxed) {
        let on = env::var_os("SPEC_CHUM_ROOM_PERF")
            .map(|v| !v.is_empty() && v != "0")
            .unwrap_or(false);
        PERF_LOG.store(on, Ordering::Relaxed);
    }
    PERF_LOG.load(Ordering::Relaxed)
}

/// Per-handle tick timing (updated on the room caller's thread).
#[derive(Debug, Default)]
pub struct RoomPerf {
    pub ticks: u64,
    pub last_tick_us: u64,
    pub max_tick_us: u64,
    /// Sum of last [`WINDOW`] tick durations for a short rolling average.
    window_sum_us: u64,
    window_n: u64,
    window_max_us: u64,
    last_log: Option<Instant>,
}

const WINDOW: u64 = 50;

impl RoomPerf {
    pub fn record_tick(&mut self, elapsed_us: u64) {
        self.ticks = self.ticks.saturating_add(1);
        self.last_tick_us = elapsed_us;
        self.max_tick_us = self.max_tick_us.max(elapsed_us);

        if self.window_n >= WINDOW {
            self.window_sum_us = 0;
            self.window_n = 0;
            self.window_max_us = 0;
        }
        self.window_sum_us = self.window_sum_us.saturating_add(elapsed_us);
        self.window_n = self.window_n.saturating_add(1);
        self.window_max_us = self.window_max_us.max(elapsed_us);

        if !perf_log_enabled() {
            return;
        }
        let should_log = self.ticks == 1
            || self
                .last_log
                .map(|t| t.elapsed().as_secs_f32() >= 1.0)
                .unwrap_or(true);
        if !should_log {
            return;
        }
        self.last_log = Some(Instant::now());
        let avg = self.window_sum_us.checked_div(self.window_n).unwrap_or(0);
        eprintln!(
            "spec_chum_room_perf: ticks={} last={:.2}ms avg{}={:.2}ms max_win={:.2}ms max_all={:.2}ms",
            self.ticks,
            elapsed_us as f64 / 1000.0,
            self.window_n,
            avg as f64 / 1000.0,
            self.window_max_us as f64 / 1000.0,
            self.max_tick_us as f64 / 1000.0,
        );
    }

    pub fn avg_window_us(&self) -> u64 {
        self.window_sum_us.checked_div(self.window_n).unwrap_or(0)
    }

    pub fn window_max_us(&self) -> u64 {
        self.window_max_us
    }

    pub fn window_n(&self) -> u64 {
        self.window_n
    }
}

/// Process-wide: last tick thread name hint (main vs background) set by Swift via FFI.
static LAST_TICK_THREAD_HINT: AtomicU64 = AtomicU64::new(0);

/// `1` = caller tagged as AppKit main, `2` = room queue, `0` = unset.
pub fn set_tick_thread_hint(hint: u64) {
    LAST_TICK_THREAD_HINT.store(hint, Ordering::Relaxed);
}

pub fn tick_thread_hint() -> u64 {
    LAST_TICK_THREAD_HINT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_window_resets() {
        let mut p = RoomPerf::default();
        for _ in 0..WINDOW {
            p.record_tick(1000);
        }
        assert_eq!(p.window_n(), WINDOW);
        p.record_tick(2000);
        assert_eq!(p.window_n(), 1);
        assert_eq!(p.last_tick_us, 2000);
    }
}
