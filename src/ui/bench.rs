//! Benchmark scenarios from docs/benchmarks.md, "Next image" and
//! "Scrolling". The variables are read once at startup; without them
//! nothing here runs and no file is written.
use super::{
    actions,
    app::{invalidate, with_state, State},
    commands::Command,
    document,
    view::Zoom,
    worker::Key,
};
use std::{
    collections::HashMap,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        Graphics::Dwm::{DwmFlush, DwmGetCompositionTimingInfo, DWM_TIMING_INFO},
        Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING},
        System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
    },
};

const IDLE: Duration = Duration::from_millis(500);
const SCROLL_TIME: Duration = Duration::from_millis(5000);
const VIEWPORTS_PER_SECOND: f32 = 2.0;
const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Scenario {
    NextImage { steps: u32 },
    Scroll,
}

#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Until the first-content marker.
    Starting,
    Idle(Instant),
    Stepping { start_qpc: i64, from: PathBuf, since: Instant },
    Scrolling { start: Instant, last: Instant },
    Done,
}

#[derive(Debug)]
pub(super) struct Bench {
    scenario: Scenario,
    out: PathBuf,
    autoclose: bool,
    /// A setup problem, reported once the scenario would start.
    error: Option<String>,
    phase: Phase,
    records: Vec<String>,
    blank_since: HashMap<Key, Instant>,
}

impl Bench {
    pub(super) fn from_env() -> Option<Self> {
        let var = |name: &str| std::env::var(name).ok();
        parse(var("PFW_BENCH_SCENARIO"), var("PFW_BENCH_SCENARIO_OUT"), var("PFW_BENCH_STEPS"), var("PFW_BENCH_AUTOCLOSE"))
    }
}

/// Reads the scenario variables. None means no benchmark run.
fn parse(scenario: Option<String>, out: Option<String>, steps: Option<String>, autoclose: Option<String>) -> Option<Bench> {
    let out = PathBuf::from(out.filter(|o| !o.is_empty())?);
    let (scenario, error) = match scenario?.as_str() {
        "next-image" => match steps.and_then(|s| s.trim().parse::<u32>().ok()).filter(|n| (1..=1000).contains(n)) {
            Some(steps) => (Scenario::NextImage { steps }, None),
            None => (Scenario::NextImage { steps: 0 }, Some("PFW_BENCH_STEPS must be an integer from 1 to 1000.".to_string())),
        },
        "scroll" => (Scenario::Scroll, None),
        other => (Scenario::Scroll, Some(format!("Unknown scenario \"{other}\". Use next-image or scroll."))),
    };
    Some(Bench {
        scenario,
        out,
        autoclose: autoclose.as_deref() == Some("1"),
        error,
        phase: Phase::Starting,
        records: Vec::new(),
        blank_since: HashMap::new(),
    })
}

fn qpc() -> i64 {
    let mut counter = 0;
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
    }
    counter
}

fn frequency() -> i64 {
    let mut frequency = 0;
    unsafe {
        let _ = QueryPerformanceFrequency(&mut frequency);
    }
    frequency
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Writes `<path>.tmp`, then renames it over `path`, so a reader never sees
/// a partial file.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    let wide = |p: &std::ffi::OsStr| p.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let (from, to) = (wide(&tmp), wide(path.as_os_str()));
    unsafe { MoveFileExW(PCWSTR(from.as_ptr()), PCWSTR(to.as_ptr()), MOVEFILE_REPLACE_EXISTING) }.map_err(|e| e.to_string())
}

impl Bench {
    fn report(&self, error: Option<&str>, extra: &str) -> String {
        let (name, list) = match self.scenario {
            Scenario::NextImage { .. } => ("next-image", "steps"),
            Scenario::Scroll => ("scroll", "frames"),
        };
        let status = if error.is_some() { "error" } else { "ok" };
        format!(
            "{{\"scenario\":\"{name}\",\"status\":\"{status}\",\"error\":{},\"qpc_frequency\":{},{extra}\"{list}\":[{}]}}",
            error.map_or("null".to_string(), json_string),
            frequency(),
            self.records.join(",")
        )
    }

    /// Writes the result and closes the window when asked to.
    fn finish(&mut self, hwnd: HWND, error: Option<&str>, extra: &str) {
        let report = self.report(error, extra);
        self.phase = Phase::Done;
        if write_atomic(&self.out, &report).is_ok() && self.autoclose {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn extra(state: &State, doc: super::widgets::Rect) -> String {
    let mut timing = DWM_TIMING_INFO { cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    // The whole-desktop timing needs a null window (Windows 8.1 and later).
    let refresh = unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut timing) }
        .ok()
        .filter(|_| timing.rateRefresh.uiDenominator > 0)
        .map(|_| timing.rateRefresh.uiNumerator as f64 / timing.rateRefresh.uiDenominator as f64)
        .unwrap_or(0.0);
    format!(
        "\"duration_ms\":{},\"speed_viewports_per_second\":{VIEWPORTS_PER_SECOND},\"viewport_width\":{},\"viewport_height\":{},\"dpi\":{},\"refresh_hz\":{refresh:.2},",
        SCROLL_TIME.as_millis(),
        doc.width().round(),
        doc.height().round(),
        (96.0 * state.scale).round()
    )
}

/// Timed steps, from the window's timer. Runs outside the state borrow,
/// because "next image" goes through the same command as the Right key.
pub(super) unsafe fn tick(hwnd: HWND) {
    let step = with_state(|state| {
        let ready = state.content_drawn && !state.pending;
        let bench = state.bench.as_mut()?;
        if let (Phase::Starting, true) = (&bench.phase, ready) {
            if let Some(error) = bench.error.clone() {
                bench.finish(hwnd, Some(&error), "");
                return None;
            }
            match bench.scenario {
                Scenario::NextImage { .. } => bench.phase = Phase::Idle(Instant::now() + IDLE),
                Scenario::Scroll => {
                    let now = Instant::now();
                    bench.phase = Phase::Scrolling { start: now, last: now };
                    document::set_zoom(state, Zoom::FitWidth, None);
                    document::go_to_page(state, 0, false);
                    invalidate(hwnd);
                }
            }
            return None;
        }
        let displayed = state.displayed.as_ref().map(|d| d.0.clone());
        let bench = state.bench.as_mut()?;
        match &bench.phase {
            Phase::Idle(until) if Instant::now() >= *until && !state.pending => {
                let from = displayed?;
                bench.phase = Phase::Stepping { start_qpc: qpc(), from, since: Instant::now() };
                Some(true)
            }
            Phase::Stepping { from, since, .. } => {
                // Nothing in flight and the same file on screen: the step failed.
                if !state.pending && displayed.as_ref() == Some(from) {
                    let error = format!("The next image did not open. {}", state.status);
                    bench.finish(hwnd, Some(&error), "");
                } else if since.elapsed() > TIMEOUT {
                    bench.finish(hwnd, Some("The next image did not show within 15 seconds."), "");
                }
                None
            }
            _ => None,
        }
    })
    .flatten();
    if step == Some(true) {
        actions::execute(hwnd, Command::Next, true);
    }
}

/// Before drawing: the scroll scenario moves by elapsed time × speed,
/// through the same function as the mouse wheel.
pub(super) fn before_draw(state: &mut State, doc: super::widgets::Rect) {
    let Some(Phase::Scrolling { start, last }) = state.bench.as_ref().map(|b| b.phase.clone()) else {
        return;
    };
    let now = Instant::now();
    let dy = (now - last).as_secs_f32() * VIEWPORTS_PER_SECOND * doc.height();
    if let Some(bench) = state.bench.as_mut() {
        bench.phase = Phase::Scrolling { start, last: now };
    }
    document::scroll_by(state, 0.0, dy, false);
}

/// After the frame was presented. Returns true when the scenario needs
/// another frame at once.
pub(super) unsafe fn after_present(hwnd: HWND, state: &mut State, doc: super::widgets::Rect, drew: bool) -> bool {
    let present = qpc();
    let Some(phase) = state.bench.as_ref().map(|b| b.phase.clone()) else {
        return false;
    };
    match phase {
        Phase::Stepping { start_qpc, from, .. } => {
            let Some((path, _)) = state.displayed.clone() else {
                return false;
            };
            if !drew || state.pending || path == from || state.frame.is_none() {
                return false;
            }
            if DwmFlush().is_err() {
                return false;
            }
            let shown = qpc();
            let (frame, from_predecode) = (state.frame.as_ref().unwrap(), state.from_predecode);
            let record = format!(
                "{{\"index\":{},\"file\":{},\"start_qpc\":{start_qpc},\"shown_qpc\":{shown},\"source_width\":{},\"source_height\":{},\"from_predecode\":{from_predecode}}}",
                state.bench.as_ref().map_or(0, |b| b.records.len()) + 1,
                json_string(&path.display().to_string()),
                frame.source_width,
                frame.source_height
            );
            let Some(bench) = state.bench.as_mut() else {
                return false;
            };
            bench.records.push(record);
            let Scenario::NextImage { steps } = bench.scenario else {
                return false;
            };
            if bench.records.len() as u32 >= steps {
                bench.finish(hwnd, None, "\"idle_before_step_ms\":500,");
            } else {
                bench.phase = Phase::Idle(Instant::now() + IDLE);
            }
            false
        }
        Phase::Scrolling { start, .. } => {
            let now = Instant::now();
            let top = document::geometry_in(state, doc).map_or(0.0, |g| g.top.max(0.0));
            let stats = state.stats.clone();
            let extra = extra(state, doc);
            let scale = state.scale;
            let Some(bench) = state.bench.as_mut() else {
                return false;
            };
            bench.blank_since.retain(|key, _| stats.blank.contains(key));
            for key in &stats.blank {
                bench.blank_since.entry(*key).or_insert(now);
            }
            let oldest = bench.blank_since.values().map(|t| (now - *t).as_secs_f64() * 1000.0).fold(0.0, f64::max);
            bench.records.push(format!(
                "{{\"present_qpc\":{present},\"scroll_y\":{:.1},\"visible_tiles\":{},\"blank_tiles\":{},\"oldest_blank_ms\":{oldest:.1}}}",
                top / scale,
                stats.visible,
                stats.blank.len()
            ));
            if start.elapsed() >= SCROLL_TIME {
                bench.finish(hwnd, None, &extra);
                return false;
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn scenarios_need_both_variables_and_valid_steps() {
        assert!(parse(None, s("out.json"), None, None).is_none(), "no scenario, no benchmark work");
        assert!(parse(s("scroll"), None, None, None).is_none());
        let next = parse(s("next-image"), s("out.json"), s("25"), s("1")).unwrap();
        assert_eq!((next.scenario.clone(), next.autoclose, next.error.clone()), (Scenario::NextImage { steps: 25 }, true, None));
        assert!(parse(s("next-image"), s("o"), s("0"), None).unwrap().error.is_some());
        assert!(parse(s("next-image"), s("o"), s("1001"), None).unwrap().error.is_some());
        assert!(parse(s("next-image"), s("o"), None, None).unwrap().error.is_some());
        assert!(parse(s("zoom"), s("o"), None, None).unwrap().error.is_some());
        assert!(!parse(s("scroll"), s("o"), None, s("0")).unwrap().autoclose);
    }

    #[test]
    fn reports_are_json_and_written_atomically() {
        let dir = std::env::temp_dir().join(format!("pfw-bench-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("scroll.json");
        let mut bench = parse(s("scroll"), Some(out.display().to_string()), None, None).unwrap();
        bench.records.push("{\"present_qpc\":1}".into());
        let report = bench.report(Some("Said \"no\" at C:\\x"), "\"dpi\":96,");
        assert!(report.starts_with("{\"scenario\":\"scroll\",\"status\":\"error\",\"error\":\"Said \\\"no\\\" at C:\\\\x\",\"qpc_frequency\":"));
        assert!(report.ends_with("\"dpi\":96,\"frames\":[{\"present_qpc\":1}]}"));
        write_atomic(&out, "first").unwrap();
        write_atomic(&out, &report).unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), report);
        assert!(!dir.join("scroll.json.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
        assert_eq!(json_string("a\nb"), "\"a\\u000ab\"");
    }
}
