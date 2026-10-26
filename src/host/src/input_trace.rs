//! `VESTRA_DEBUG_KEYS=1`: how long a bound input takes on its way to the
//! source. The window's event hook starts a trace at a key or button press;
//! each hop after it (Slint callback, binding, playback worker, libmpv, the
//! next published `source.state`, the completion) logs its time since the
//! press to stderr as `keys: trace #N +12.3 ms <hop>`. One trace at a time:
//! the next press starts a new one. Off, every call returns at once.
#![forbid(unsafe_code)]

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Hops later than this after the press belong to no trace.
const SPAN: Duration = Duration::from_secs(5);

struct Trace {
    id: u64,
    start: Instant,
}
static TRACE: Mutex<Option<Trace>> = Mutex::new(None);

pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_KEYS").is_some_and(|v| v == "1"))
}

/// A press: a new trace starts now.
pub fn begin(what: &str) {
    if !enabled() {
        return;
    }
    let mut trace = TRACE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = trace.as_ref().map_or(1, |trace| trace.id + 1);
    *trace = Some(Trace {
        id,
        start: Instant::now(),
    });
    diag!("keys: trace #{id} +0.0 ms {what}");
}

/// One hop of the current trace, if a press started one recently.
pub fn mark(hop: &str) {
    if !enabled() {
        return;
    }
    let trace = TRACE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(trace) = trace.as_ref()
        && let Some(elapsed) = Some(trace.start.elapsed()).filter(|e| *e <= SPAN)
    {
        diag!(
            "keys: trace #{} +{:.1} ms {hop}",
            trace.id,
            elapsed.as_secs_f64() * 1000.
        );
    }
}

/// A hop of the press that led here if one started within half a second (a
/// plugin's command calling back), else a trace of its own (a plugin button).
pub fn follow(hop: &str) {
    if !enabled() {
        return;
    }
    let recent = TRACE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .is_some_and(|trace| trace.start.elapsed() <= Duration::from_millis(500));
    if recent { mark(hop) } else { begin(hop) }
}
