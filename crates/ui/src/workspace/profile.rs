//! A developer aid for measuring the board. Set `LOOJI_FPS=1` and the app draws continuously and writes a line to stderr
//! each second: how many frames were drawn, how long the longest gap between two frames was, and how long the main
//! view took to build on average and at most. With the aid off, a frame costs one check of a cached flag.

use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use gpui::Window;

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOOJI_FPS").is_some())
}

struct Totals {
    second_started: Instant,
    last_frame: Option<Instant>,
    frames: u32,
    longest_gap: Duration,
    build_total: Duration,
    build_longest: Duration,
}

fn totals() -> &'static Mutex<Totals> {
    static TOTALS: OnceLock<Mutex<Totals>> = OnceLock::new();
    TOTALS.get_or_init(|| {
        Mutex::new(Totals {
            second_started: Instant::now(),
            last_frame: None,
            frames: 0,
            longest_gap: Duration::ZERO,
            build_total: Duration::ZERO,
            build_longest: Duration::ZERO,
        })
    })
}

/// Counts a frame and asks for the next one. Hold the returned guard until the view has been built, and it records how
/// long that took.
pub(super) fn frame(window: &mut Window) -> Option<Build> {
    if !enabled() {
        return None;
    }
    window.request_animation_frame();
    let now = Instant::now();
    if let Ok(mut totals) = totals().lock() {
        if let Some(last) = totals.last_frame {
            totals.longest_gap = totals.longest_gap.max(now - last);
        }
        totals.last_frame = Some(now);
        totals.frames += 1;
        if now - totals.second_started >= Duration::from_secs(1) {
            let average =
                totals.build_total.as_secs_f64() * 1000.0 / f64::from(totals.frames.max(1));
            eprintln!(
                "looji fps: {} frames, longest gap {:.1} ms, view build {:.1} ms average and {:.1} ms at most",
                totals.frames,
                totals.longest_gap.as_secs_f64() * 1000.0,
                average,
                totals.build_longest.as_secs_f64() * 1000.0,
            );
            totals.second_started = now;
            totals.frames = 0;
            totals.longest_gap = Duration::ZERO;
            totals.build_total = Duration::ZERO;
            totals.build_longest = Duration::ZERO;
        }
    }
    Some(Build(now))
}

/// Records how long the view took to build, when it's dropped.
pub(super) struct Build(Instant);

impl Drop for Build {
    fn drop(&mut self) {
        let took = self.0.elapsed();
        if let Ok(mut totals) = totals().lock() {
            totals.build_total += took;
            totals.build_longest = totals.build_longest.max(took);
        }
    }
}
