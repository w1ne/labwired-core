//! Runtime-toggled wall-clock attribution for the simulation loop.
//!
//! # Why this exists
//!
//! Counters lie about cost. The BLE Pong lab arms ~2.1 M scheduler events per
//! 400 M cycles and **67 % of them belong to `i2c0`** — yet deleting the I²C
//! device entirely (same firmware, same everything) buys only ~1.14x wall.
//! Two thirds of the events are an eighth of the time. Anything that ranks
//! subsystems by how often they fire will point at the wrong one, which is
//! exactly how a shared-air BLE model got blamed for a slowdown that turned out
//! to be a 250 000-cycle frame cap in the host.
//!
//! So this module measures **nanoseconds**, not occurrences.
//!
//! # Why it is not a `#[cfg(feature)]`
//!
//! [`crate::quantum_trace`] is feature-gated and documents itself as "never in
//! a shipped build". That is the right call for a counter you only ever want
//! while bisecting locally. It is the wrong call here: the slowdown this module
//! was written to diagnose happened **in a deployed browser build**, where you
//! cannot attach `sample`, cannot rebuild with a feature flag, and cannot ask
//! the user to reproduce it natively. A profiler you have to recompile to use
//! is a profiler you do not have when it matters.
//!
//! It is therefore always compiled and gated by one thread-local `bool`. When
//! disabled the entire cost is [`enabled`] — a `Cell<bool>` read — on three
//! paths that already do far more work than that per call.
//!
//! # Clock
//!
//! `std::time::Instant` **panics** on `wasm32-unknown-unknown`, which is the
//! one target that most needs this. Rather than pull `wasm-bindgen` into
//! `labwired-core` (the crate deliberately keeps a lean wasm dependency tree —
//! see the `jit` feature comment in `Cargo.toml`), the host installs a
//! nanosecond clock with [`set_clock`]; the browser bridge passes
//! `performance.now`. Native gets a default `Instant`-based clock for free.
//!
//! ⚠️ With no clock installed on wasm, every duration reads **zero**. Zero must
//! never be mistaken for "this subsystem is free", so [`Snapshot::clock`]
//! records which clock produced the numbers and [`Report::render`] refuses to
//! print a breakdown without one. See `ops_null_coerces_to_zero_false_verdicts`
//! — a null that coerces to 0 produces a confident, wrong verdict.
//!
//! # Usage
//!
//! ```no_run
//! # use labwired_core::profile;
//! profile::start();
//! // ... run the machine ...
//! profile::stop();
//! # let names: Vec<String> = vec![];
//! println!("{}", profile::snapshot().report(&names).render());
//! ```

use std::cell::Cell;
use std::cell::RefCell;

/// A monotonic clock in nanoseconds. Need not share an epoch with anything —
/// only differences are ever taken.
pub type ClockFn = fn() -> u64;

/// Which clock produced a [`Snapshot`]. Carried so a reader can tell "measured
/// as zero" from "never measured".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clock {
    /// Native `std::time::Instant`.
    Native,
    /// A clock installed by the host via [`set_clock`] (browser
    /// `performance.now`).
    HostInstalled,
    /// No usable clock: `wasm32` with nothing installed. **All durations in the
    /// snapshot are zero and mean nothing.**
    None,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static CLOCK: Cell<Option<ClockFn>> = const { Cell::new(None) };
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Note which machine is about to record, so a report can say whether its
/// peripheral rows describe one chip or several merged together.
///
/// Attribution is keyed by peripheral INDEX, and indices are per-machine: on a
/// multi-chip lab stepping several machines on one thread (every Web Worker
/// lab), index 3 may be `i2c0` on one chip and `tim2` on the next. Merging
/// those into one row is not a rounding error, it is a category error — so the
/// count is tracked and [`Report::render`] refuses to stay quiet about it.
///
/// Costs one `u64` compare per scheduler drain, not per event.
pub fn set_machine(id: u64) {
    if !enabled() {
        return;
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.machines.last() != Some(&id) && !s.machines.contains(&id) {
            s.machines.push(id);
        }
    });
}

#[derive(Default)]
struct State {
    window_start_ns: u64,
    window_ns: u64,
    cpu_ns: u64,
    cpu_calls: u64,
    /// Inclusive: covers the per-peripheral handlers dispatched inside it.
    /// [`Report`] subtracts them to get the scheduler's own overhead.
    sched_ns: u64,
    sched_calls: u64,
    tick_ns: u64,
    tick_calls: u64,
    periph_ns: Vec<u64>,
    periph_calls: Vec<u64>,
    /// Distinct machines that recorded into this window, in first-seen order.
    machines: Vec<u64>,
}

impl State {
    fn bump_periph(&mut self, idx: usize, ns: u64) {
        if idx >= self.periph_ns.len() {
            self.periph_ns.resize(idx + 1, 0);
            self.periph_calls.resize(idx + 1, 0);
        }
        self.periph_ns[idx] += ns;
        self.periph_calls[idx] += 1;
    }
}

/// Install the nanosecond clock. Required on `wasm32`; optional natively.
pub fn set_clock(clock: ClockFn) {
    CLOCK.with(|c| c.set(Some(clock)));
}

fn active_clock() -> Clock {
    if CLOCK.with(|c| c.get()).is_some() {
        Clock::HostInstalled
    } else if cfg!(target_arch = "wasm32") {
        Clock::None
    } else {
        Clock::Native
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn default_now_ns() -> u64 {
    thread_local! {
        static BASE: std::time::Instant = std::time::Instant::now();
    }
    BASE.with(|b| b.elapsed().as_nanos() as u64)
}

/// No monotonic clock is reachable from `wasm32-unknown-unknown` without
/// `wasm-bindgen`. Returning a constant makes every duration zero, which
/// [`Clock::None`] then flags rather than letting it read as "free".
#[cfg(target_arch = "wasm32")]
fn default_now_ns() -> u64 {
    0
}

/// Current time in nanoseconds from whichever clock is installed.
#[inline]
pub fn now_ns() -> u64 {
    match CLOCK.with(|c| c.get()) {
        Some(f) => f(),
        None => default_now_ns(),
    }
}

/// Is attribution currently recording? One `Cell<bool>` read — this is the
/// whole cost when profiling is off.
#[inline]
pub fn enabled() -> bool {
    ENABLED.with(|e| e.get())
}

/// Begin a profiling window, clearing anything previously accumulated.
pub fn start() {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        *s = State {
            window_start_ns: now_ns(),
            ..State::default()
        };
    });
    ENABLED.with(|e| e.set(true));
}

/// End the profiling window. The snapshot survives until the next [`start`].
pub fn stop() {
    if !enabled() {
        return;
    }
    ENABLED.with(|e| e.set(false));
    let end = now_ns();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.window_ns = end.saturating_sub(s.window_start_ns);
    });
}

/// An open timing span. Obtained from [`span`], which yields `None` when
/// profiling is off so the caller pays nothing.
#[derive(Clone, Copy)]
pub struct Span(u64);

impl Span {
    #[inline]
    fn elapsed(self) -> u64 {
        now_ns().saturating_sub(self.0)
    }
}

/// Open a span, or `None` if profiling is off.
#[inline]
pub fn span() -> Option<Span> {
    if enabled() {
        Some(Span(now_ns()))
    } else {
        None
    }
}

macro_rules! record {
    ($name:ident, $ns:ident, $calls:ident) => {
        /// Close a span into this bucket. No-op if profiling was switched off
        /// mid-span.
        #[inline]
        pub fn $name(span: Option<Span>) {
            let Some(span) = span else { return };
            if !enabled() {
                return;
            }
            let ns = span.elapsed();
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.$ns += ns;
                s.$calls += 1;
            });
        }
    };
}

record!(record_cpu, cpu_ns, cpu_calls);
record!(record_sched, sched_ns, sched_calls);
record!(record_tick, tick_ns, tick_calls);

/// Close a span into one peripheral's bucket, keyed by its index in
/// `SystemBus::peripherals`. Names are resolved later, by [`Snapshot::report`].
#[inline]
pub fn record_peripheral(span: Option<Span>, idx: usize) {
    let Some(span) = span else { return };
    if !enabled() {
        return;
    }
    let ns = span.elapsed();
    STATE.with(|s| s.borrow_mut().bump_periph(idx, ns));
}

/// Raw, index-keyed attribution. Turn it into something readable with
/// [`Snapshot::report`], which needs the bus's peripheral names.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub clock: Clock,
    pub window_ns: u64,
    pub cpu_ns: u64,
    pub cpu_calls: u64,
    /// Inclusive of the peripheral handlers dispatched within it.
    pub sched_ns: u64,
    pub sched_calls: u64,
    pub tick_ns: u64,
    pub tick_calls: u64,
    pub periph_ns: Vec<u64>,
    pub periph_calls: Vec<u64>,
    /// How many distinct machines recorded into this window. Anything above 1
    /// means the peripheral rows are merged across chips and their names come
    /// from whichever machine rendered the report.
    pub machines: usize,
}

/// Read the current attribution. Safe to call while running.
pub fn snapshot() -> Snapshot {
    STATE.with(|s| {
        let s = s.borrow();
        let window_ns = if s.window_ns > 0 {
            s.window_ns
        } else {
            now_ns().saturating_sub(s.window_start_ns)
        };
        Snapshot {
            clock: active_clock(),
            window_ns,
            cpu_ns: s.cpu_ns,
            cpu_calls: s.cpu_calls,
            sched_ns: s.sched_ns,
            sched_calls: s.sched_calls,
            tick_ns: s.tick_ns,
            tick_calls: s.tick_calls,
            periph_ns: s.periph_ns.clone(),
            periph_calls: s.periph_calls.clone(),
            machines: s.machines.len(),
        }
    })
}

/// One attributed subsystem.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: String,
    pub ns: u64,
    pub calls: u64,
    pub percent: f64,
}

/// A named, ranked breakdown of a [`Snapshot`].
#[derive(Clone, Debug)]
pub struct Report {
    pub clock: Clock,
    pub window_ns: u64,
    /// Distinct machines merged into these rows; see [`set_machine`].
    pub machines: usize,
    /// Time inside the profiled window that no bucket claimed. Large
    /// unattributed time means the instrumentation is missing a hot path — it
    /// is a defect in this module, not a subsystem that is free.
    pub unattributed_ns: u64,
    pub rows: Vec<Row>,
}

impl Snapshot {
    /// Resolve peripheral indices against `names` (`SystemBus::peripherals`, in
    /// order) and rank every bucket by wall time.
    ///
    /// The scheduler row is made **exclusive**: `sched_ns` is measured
    /// inclusive of the handlers it dispatches, so the peripheral totals are
    /// subtracted out. Without that, `i2c0` would be counted twice and the
    /// percentages would sum past 100.
    pub fn report(&self, names: &[String]) -> Report {
        let periph_total: u64 = self.periph_ns.iter().sum();
        let sched_own = self.sched_ns.saturating_sub(periph_total);
        let mut rows = vec![
            Row {
                name: "cpu".to_string(),
                ns: self.cpu_ns,
                calls: self.cpu_calls,
                percent: 0.0,
            },
            Row {
                name: "scheduler".to_string(),
                ns: sched_own,
                calls: self.sched_calls,
                percent: 0.0,
            },
            Row {
                name: "peripheral-tick".to_string(),
                ns: self.tick_ns,
                calls: self.tick_calls,
                percent: 0.0,
            },
        ];
        for (idx, &ns) in self.periph_ns.iter().enumerate() {
            if ns == 0 {
                continue;
            }
            let name = names
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("peripheral[{idx}]"));
            rows.push(Row {
                name,
                ns,
                calls: self.periph_calls.get(idx).copied().unwrap_or(0),
                percent: 0.0,
            });
        }
        rows.retain(|r| r.ns > 0);
        let attributed: u64 = rows.iter().map(|r| r.ns).sum();
        let denom = self.window_ns.max(1) as f64;
        for row in &mut rows {
            row.percent = 100.0 * row.ns as f64 / denom;
        }
        rows.sort_by_key(|row| std::cmp::Reverse(row.ns));
        Report {
            clock: self.clock,
            window_ns: self.window_ns,
            machines: self.machines,
            unattributed_ns: self.window_ns.saturating_sub(attributed),
            rows,
        }
    }
}

impl Report {
    /// A fixed-width table. Refuses to print numbers taken without a clock.
    pub fn render(&self) -> String {
        if self.clock == Clock::None {
            return "profile: NO CLOCK INSTALLED — every duration would read 0. \
                    Call labwired_core::profile::set_clock() from the host \
                    (browser: performance.now) before profiling."
                .to_string();
        }
        if self.window_ns == 0 {
            return "profile: empty window (start() then run, then stop())".to_string();
        }
        let mut out = format!(
            "profile: {:.3} ms window, clock={:?}\n",
            self.window_ns as f64 / 1e6,
            self.clock
        );
        if self.machines > 1 {
            out.push_str(&format!(
                "  ⚠️ {} machines stepped on this thread — peripheral rows are \
                 MERGED across them and named from one chip's bus. Profile a \
                 single-chip lab to attribute per chip.\n",
                self.machines,
            ));
        }
        for row in &self.rows {
            out.push_str(&format!(
                "  {:>6.1}%  {:>10.3} ms  {:>12} calls  {}\n",
                row.percent,
                row.ns as f64 / 1e6,
                row.calls,
                row.name,
            ));
        }
        out.push_str(&format!(
            "  {:>6.1}%  {:>10.3} ms  {:>12}         unattributed\n",
            100.0 * self.unattributed_ns as f64 / self.window_ns as f64,
            self.unattributed_ns as f64 / 1e6,
            "-",
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deterministic clock, so the tests assert on arithmetic rather than on
    /// how long the host took.
    fn fake_now() -> u64 {
        thread_local! {
            static T: Cell<u64> = const { Cell::new(0) };
        }
        T.with(|t| {
            let v = t.get();
            t.set(v + 100);
            v
        })
    }

    #[test]
    fn disabled_by_default_and_records_nothing() {
        stop();
        let before = snapshot();
        record_cpu(span());
        let after = snapshot();
        assert_eq!(after.cpu_calls, before.cpu_calls);
        assert!(span().is_none(), "span() must be None while disabled");
    }

    #[test]
    fn scheduler_row_excludes_the_handlers_it_dispatched() {
        // 1000 ns in the drain, of which 800 was i2c0's handler: the scheduler
        // itself owns 200. Counting it inclusive would double-charge i2c0.
        let snap = Snapshot {
            clock: Clock::Native,
            window_ns: 1_000,
            cpu_ns: 0,
            cpu_calls: 0,
            sched_ns: 1_000,
            sched_calls: 1,
            tick_ns: 0,
            tick_calls: 0,
            periph_ns: vec![800],
            periph_calls: vec![1],
            machines: 1,
        };
        let report = snap.report(&["i2c0".to_string()]);
        let sched = report
            .rows
            .iter()
            .find(|r| r.name == "scheduler")
            .expect("scheduler row");
        assert_eq!(sched.ns, 200);
        let total: u64 = report.rows.iter().map(|r| r.ns).sum();
        assert_eq!(total, 1_000, "buckets must not double-count");
        assert_eq!(report.unattributed_ns, 0);
    }

    #[test]
    fn missing_clock_is_reported_not_rendered_as_free() {
        let snap = Snapshot {
            clock: Clock::None,
            window_ns: 0,
            cpu_ns: 0,
            cpu_calls: 0,
            sched_ns: 0,
            sched_calls: 0,
            tick_ns: 0,
            tick_calls: 0,
            periph_ns: vec![],
            periph_calls: vec![],
            machines: 0,
        };
        let rendered = snap.report(&[]).render();
        assert!(
            rendered.contains("NO CLOCK INSTALLED"),
            "a clockless snapshot must say so, never print 0% rows: {rendered}"
        );
    }

    #[test]
    fn merged_machines_are_never_reported_silently() {
        // Two chips on one thread: index 3 is `i2c0` on one and `tim2` on the
        // other. The rows are merged, and a reader who is not told that will
        // optimise the wrong peripheral.
        let snap = Snapshot {
            clock: Clock::Native,
            window_ns: 1_000,
            cpu_ns: 500,
            cpu_calls: 1,
            sched_ns: 0,
            sched_calls: 0,
            tick_ns: 0,
            tick_calls: 0,
            periph_ns: vec![],
            periph_calls: vec![],
            machines: 2,
        };
        let rendered = snap.report(&[]).render();
        assert!(
            rendered.contains("MERGED"),
            "a 2-machine window must say its rows are merged: {rendered}"
        );
    }

    #[test]
    fn attributes_to_the_named_peripheral() {
        set_clock(fake_now);
        start();
        record_peripheral(span(), 1);
        record_cpu(span());
        stop();
        let names = vec!["uart0".to_string(), "i2c0".to_string()];
        let report = snapshot().report(&names);
        assert!(
            report.rows.iter().any(|r| r.name == "i2c0" && r.calls == 1),
            "expected an i2c0 row: {report:?}"
        );
    }
}
