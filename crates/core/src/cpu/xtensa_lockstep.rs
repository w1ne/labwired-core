// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Lockstep correctness harness for the Xtensa LX7 interpreter vs JIT
//! (labwired-core #124, Phase 3.6.1).
//!
//! ## Why
//!
//! Phase 3.6 of the JIT roadmap emits native code for hot basic blocks
//! containing windowed CALLn / RETW. The roadmap risk-2 says
//! windowed-register semantics breakage probability is ~60%, and any
//! divergence between interp and JIT state would be silent until a panel
//! refresh or a stack walk picks up a wrong value much later. This module
//! is the correctness gate that future Phase 3.6 emit code must pass
//! before being trusted.
//!
//! ## Design — record then replay
//!
//! The original spec suggested two `XtensaLx7` instances pointing at a
//! single shared `SystemBus`. `SystemBus` is a plain owned struct (not
//! behind a trait object or `Rc<RefCell<…>>`), so genuine bus sharing
//! would need either an invasive Rc-RefCell retrofit across every
//! peripheral or a duplicate-and-reconcile dance per step. Both are
//! large surface-area changes that don't pay off for a v1 harness.
//!
//! Instead this harness uses the alternate approach the spec calls out:
//!
//!   1. Build a `Machine<XtensaLx7>` via a caller-supplied factory.
//!   2. Run it under [`ExecMode::Interpreter`] for `steps`, capturing a
//!      [`CpuState`] after every step into a `Vec`.
//!   3. Build a fresh `Machine<XtensaLx7>` from the same factory.
//!   4. Run it under [`ExecMode::Jit`] for the same `steps`, capturing
//!      states again.
//!   5. Diff the two traces step-by-step; first mismatch → bail with a
//!      [`DivergenceReport`] carrying both sides plus the PC trail.
//!
//! Memory cost is `~80 bytes × steps`; at 100k steps the harness keeps
//! ~8 MB which is fine for everything except the multi-hundred-million-cycle
//! e2e test (which we'd never lockstep-trace anyway — bug isolation runs
//! happen on shorter windows).
//!
//! ## What "JIT path" means today
//!
//! At the time this harness landed, no JIT emit code exists on `main` —
//! the pilot from #126 was still draft. [`ExecMode::Jit`] therefore runs
//! the exact same `Machine::step()` as [`ExecMode::Interpreter`]. The
//! traces compare equal byte-for-byte, which is the *correct* behaviour:
//! the harness must report zero divergence for the no-op case so that
//! when real emit code arrives, any divergence is unambiguously the
//! JIT's fault and not noise from the harness itself.
//!
//! Phase 3.6.x callers will swap [`ExecMode::Jit`] to actually route
//! through their dispatch (a `JitDispatcher::step` shim, or a feature-
//! flagged code path inside `XtensaLx7::step`). The trace-capture and
//! diff logic stays as-is.

use crate::cpu::xtensa_sr::{CCOUNT, LBEG, LCOUNT, LEND, SAR};
use crate::{Cpu, Machine, SimResult, SimulationError};
use std::fmt;

/// Per-step CPU state snapshot used for lockstep comparison.
///
/// Captures the architectural fields the JIT must keep in sync with the
/// interpreter. Excludes things that vary by execution strategy without
/// affecting program semantics (e.g. internal fetch cache, decoder
/// statistics, observer counters).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CpuState {
    pub step: u64,
    pub pc: u32,
    pub ps: u32,
    pub sar: u32,
    pub lbeg: u32,
    pub lend: u32,
    pub lcount: u32,
    pub ccount: u32,
    /// Logical AR registers a0..a15 (post-windowing view — what the
    /// program actually sees), not the 64-entry physical file.
    pub ar: [u32; 16],
}

impl fmt::Debug for CpuState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "CpuState {{")?;
        writeln!(f, "  step:   {}", self.step)?;
        writeln!(f, "  pc:     0x{:08x}", self.pc)?;
        writeln!(f, "  ps:     0x{:08x}", self.ps)?;
        writeln!(f, "  sar:    0x{:08x}", self.sar)?;
        writeln!(f, "  lbeg:   0x{:08x}", self.lbeg)?;
        writeln!(f, "  lend:   0x{:08x}", self.lend)?;
        writeln!(f, "  lcount: 0x{:08x}", self.lcount)?;
        writeln!(f, "  ccount: 0x{:08x}", self.ccount)?;
        for (i, v) in self.ar.iter().enumerate() {
            writeln!(f, "  a{:<2}    0x{:08x}", i, v)?;
        }
        write!(f, "}}")
    }
}

impl CpuState {
    /// Capture the architectural state of `cpu`. Reads logical AR regs
    /// (not physical) so a window-rotate followed by a window-rotate-back
    /// shows up as a no-op rather than a noisy diff.
    pub fn capture<C: Cpu + LockstepObservable>(step: u64, cpu: &C) -> Self {
        let mut ar = [0u32; 16];
        for (i, slot) in ar.iter_mut().enumerate() {
            *slot = cpu.get_register(i as u8);
        }
        Self {
            step,
            pc: cpu.get_pc(),
            ps: cpu.lockstep_ps(),
            sar: cpu.lockstep_sr(SAR),
            lbeg: cpu.lockstep_sr(LBEG),
            lend: cpu.lockstep_sr(LEND),
            lcount: cpu.lockstep_sr(LCOUNT),
            ccount: cpu.lockstep_sr(CCOUNT),
            ar,
        }
    }
}

/// Side-channel CPU accessor for the lockstep harness. The base `Cpu`
/// trait exposes PC + GP-regs but not PS / SR — both live in CPU-specific
/// fields. Adding this small trait avoids leaking Xtensa internals into
/// the cross-arch `Cpu` API while still letting the harness probe the
/// fields it needs to compare.
pub trait LockstepObservable {
    /// Raw `PS` register value (Processor State).
    fn lockstep_ps(&self) -> u32;
    /// Read special register `sr_id`. Numeric IDs match
    /// `crate::cpu::xtensa_sr` constants.
    fn lockstep_sr(&self, sr_id: u16) -> u32;
}

impl LockstepObservable for crate::cpu::XtensaLx7 {
    fn lockstep_ps(&self) -> u32 {
        self.ps.as_raw()
    }
    fn lockstep_sr(&self, sr_id: u16) -> u32 {
        // Route via XtensaSrFile directly — WINDOWBASE/WINDOWSTART live
        // on the AR file, but the lockstep diff only consumes SAR / LBEG
        // / LEND / LCOUNT / CCOUNT which all live on the SR file.
        self.sr.read(sr_id)
    }
}

/// Which execution strategy a trace was captured under. Today both
/// variants run `Machine::step()`; the discriminant is preserved on the
/// recorded trace so a future Phase 3.6 patch can route `Jit` through
/// dispatch + emit without losing the audit trail of *what* produced
/// each state vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecMode {
    Interpreter,
    Jit,
}

/// Comparison tolerance. `CCOUNT` is allowed to drift by `ccount_tolerance`
/// cycles in either direction because a JIT that batches a basic block of
/// N interp-equivalent instructions may bump CCOUNT once per BB exit
/// rather than once per instruction. Everything else must match exactly.
#[derive(Clone, Copy, Debug)]
pub struct ComparePolicy {
    pub ccount_tolerance: u32,
}

impl Default for ComparePolicy {
    fn default() -> Self {
        // Spec: "accept ±1 CCOUNT but flag larger gaps".
        Self {
            ccount_tolerance: 1,
        }
    }
}

/// Produced when the two traces disagree.
#[derive(Debug)]
pub struct DivergenceReport {
    pub step: u64,
    pub field: &'static str,
    pub interp: CpuState,
    pub jit: CpuState,
    /// Last `pc_trail_len` PCs from the interpreter trace, oldest →
    /// newest, ending at the diverging step. Useful for "where did the
    /// JIT get pulled off the rails?" investigations.
    pub pc_trail: Vec<u32>,
}

impl fmt::Display for DivergenceReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "lockstep DIVERGENCE @ step {} in field `{}`",
            self.step, self.field
        )?;
        writeln!(f, "interp = {:#?}", self.interp)?;
        writeln!(f, "jit    = {:#?}", self.jit)?;
        writeln!(f, "pc trail (last {}):", self.pc_trail.len())?;
        for pc in &self.pc_trail {
            writeln!(f, "  0x{:08x}", pc)?;
        }
        Ok(())
    }
}

/// Compare two traces step-by-step. First mismatch wins. CCOUNT diffs
/// within `policy.ccount_tolerance` are tolerated; larger gaps fail.
///
/// `DivergenceReport` is ~260 bytes so we return it boxed to keep
/// `Result` discriminant size sensible — callers shouldn't be paying
/// for an unboxed 260-byte enum on the hot path.
pub fn compare_traces(
    interp: &[CpuState],
    jit: &[CpuState],
    policy: ComparePolicy,
) -> Result<(), Box<DivergenceReport>> {
    if interp.len() != jit.len() {
        // Length mismatch — both should have hit the same fault/halt at
        // the same step. Report at the first step that exists on one side
        // but not the other.
        let short = interp.len().min(jit.len());
        let last_common = if short == 0 {
            CpuState {
                step: 0,
                pc: 0,
                ps: 0,
                sar: 0,
                lbeg: 0,
                lend: 0,
                lcount: 0,
                ccount: 0,
                ar: [0; 16],
            }
        } else {
            interp[short - 1]
        };
        return Err(Box::new(DivergenceReport {
            step: short as u64,
            field: "trace_length",
            interp: *interp.get(short).unwrap_or(&last_common),
            jit: *jit.get(short).unwrap_or(&last_common),
            pc_trail: pc_trail(interp, short),
        }));
    }
    for (i, (a, b)) in interp.iter().zip(jit.iter()).enumerate() {
        if let Some(field) = first_diff(a, b, policy) {
            return Err(Box::new(DivergenceReport {
                step: i as u64,
                field,
                interp: *a,
                jit: *b,
                pc_trail: pc_trail(interp, i),
            }));
        }
    }
    Ok(())
}

fn first_diff(a: &CpuState, b: &CpuState, policy: ComparePolicy) -> Option<&'static str> {
    if a.pc != b.pc {
        return Some("pc");
    }
    if a.ps != b.ps {
        return Some("ps");
    }
    if a.sar != b.sar {
        return Some("sar");
    }
    if a.lbeg != b.lbeg {
        return Some("lbeg");
    }
    if a.lend != b.lend {
        return Some("lend");
    }
    if a.lcount != b.lcount {
        return Some("lcount");
    }
    // CCOUNT tolerance — accept |Δ| ≤ tolerance; flag larger gaps.
    let ccount_delta = a.ccount.abs_diff(b.ccount);
    if ccount_delta > policy.ccount_tolerance {
        return Some("ccount");
    }
    for (i, (x, y)) in a.ar.iter().zip(b.ar.iter()).enumerate() {
        if x != y {
            // Static slice of names so we can return &'static str. We
            // accept a small string-table lookup cost since this only
            // fires on actual divergence.
            const NAMES: [&str; 16] = [
                "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "a10", "a11", "a12",
                "a13", "a14", "a15",
            ];
            return Some(NAMES[i]);
        }
    }
    None
}

fn pc_trail(trace: &[CpuState], end: usize) -> Vec<u32> {
    const TRAIL_LEN: usize = 16;
    let start = end.saturating_sub(TRAIL_LEN);
    trace[start..end.min(trace.len())]
        .iter()
        .map(|s| s.pc)
        .collect()
}

/// Heartbeat interval — emits a stderr line every N steps so a
/// long-running record session shows progress without flooding output.
const HEARTBEAT_INTERVAL: u64 = 100_000;

/// Run `machine` for `steps` instructions under `mode`, capturing one
/// [`CpuState`] per step. Stops early if a step fails (returns whatever
/// states were captured before the error so the caller can still inspect
/// the lead-up).
///
/// `post_step` runs after each successful step — used by tests to inject
/// the "deliberate divergence" mutation. Pass `|_, _| Ok(())` if you
/// don't need it.
pub fn record<C, F>(
    machine: &mut Machine<C>,
    mode: ExecMode,
    steps: u64,
    mut post_step: F,
) -> (Vec<CpuState>, Option<SimulationError>)
where
    C: Cpu + LockstepObservable,
    F: FnMut(ExecMode, &mut Machine<C>) -> SimResult<()>,
{
    let mut trace = Vec::with_capacity(steps as usize);
    for i in 0..steps {
        // Today both modes route through the same Machine::step. The
        // match is preserved so Phase 3.6 can splice JIT dispatch into
        // the `Jit` arm without touching the trace-capture loop.
        let result = match mode {
            ExecMode::Interpreter => machine.step(),
            ExecMode::Jit => machine.step(),
        };
        if let Err(e) = result {
            return (trace, Some(e));
        }
        if let Err(e) = post_step(mode, machine) {
            return (trace, Some(e));
        }
        trace.push(CpuState::capture(i, &machine.cpu));
        if (i + 1) % HEARTBEAT_INTERVAL == 0 {
            let s = trace.last().expect("just pushed");
            eprintln!(
                "[lockstep:{mode:?}] step={} pc=0x{:08x} ccount=0x{:08x}",
                i + 1,
                s.pc,
                s.ccount,
            );
        }
    }
    (trace, None)
}

/// High-level driver: build the machine twice, record both traces, diff.
///
/// `factory` constructs and primes a fresh `Machine` — load firmware,
/// install thunks, seed SP, whatever the caller's setup needs. Called
/// once for the interpreter pass and once for the JIT pass.
///
/// `inject` is called on the JIT pass only, after the JIT machine is
/// built but before the run starts. Use it to deliberately perturb JIT
/// state in self-tests of the harness (e.g. "force a wrong value into
/// a8; confirm divergence is detected"). Pass `|_| Ok(())` for the
/// production correctness gate.
pub struct LockstepRunner<C: Cpu + LockstepObservable, F>
where
    F: FnMut() -> SimResult<Machine<C>>,
{
    factory: F,
    steps: u64,
    policy: ComparePolicy,
}

impl<C, F> LockstepRunner<C, F>
where
    C: Cpu + LockstepObservable,
    F: FnMut() -> SimResult<Machine<C>>,
{
    pub fn new(factory: F, steps: u64) -> Self {
        Self {
            factory,
            steps,
            policy: ComparePolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: ComparePolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Record both traces; return them along with any step-level error
    /// each side hit. Does **not** call [`compare_traces`] — split so
    /// callers can inspect the traces before the diff (e.g. dump them
    /// to disk for offline analysis on a known-bad firmware).
    pub fn record_both<I>(mut self, mut inject: I) -> SimResult<RecordedTraces>
    where
        I: FnMut(&mut Machine<C>) -> SimResult<()>,
    {
        let mut interp_machine = (self.factory)()?;
        let (interp_trace, interp_err) = record(
            &mut interp_machine,
            ExecMode::Interpreter,
            self.steps,
            |_, _| Ok(()),
        );
        drop(interp_machine);

        let mut jit_machine = (self.factory)()?;
        inject(&mut jit_machine)?;
        let (jit_trace, jit_err) =
            record(&mut jit_machine, ExecMode::Jit, self.steps, |_, _| Ok(()));

        Ok((interp_trace, jit_trace, interp_err, jit_err))
    }

    /// Record both traces and diff them. The common case — what Phase
    /// 3.6 commits will call as the post-emit correctness gate.
    ///
    /// `Result::Err(DivergenceReport)` signals a state mismatch. A
    /// factory-side error (e.g. ELF parse failure) propagates through
    /// the report's `field == "factory_error"`. Callers that need to
    /// distinguish "harness setup failed" from "JIT diverged" should
    /// use [`Self::record_both`] directly.
    pub fn run_and_compare(self) -> Result<LockstepReport, Box<DivergenceReport>> {
        let policy = self.policy;
        let recorded = self.record_both(|_| Ok(()));
        let (interp, jit, interp_err, jit_err) = match recorded {
            Ok(t) => t,
            Err(err) => {
                return Err(Box::new(DivergenceReport {
                    step: 0,
                    field: "factory_error",
                    interp: empty_state(),
                    jit: empty_state(),
                    pc_trail: vec![err.to_string().len() as u32],
                }));
            }
        };

        // Mismatched step errors are themselves a divergence — the
        // interpreter and JIT disagreed on whether the firmware faults
        // at this point.
        match (&interp_err, &jit_err) {
            (Some(a), Some(b)) if a.to_string() == b.to_string() => {}
            (None, None) => {}
            _ => {
                // Both sides ran out (possibly only one faulted). Treat
                // as length-mismatch divergence so the report carries
                // the lead-up trail.
                return Err(Box::new(DivergenceReport {
                    step: interp.len().min(jit.len()) as u64,
                    field: "step_error_disagrees",
                    interp: interp.last().copied().unwrap_or_else(empty_state),
                    jit: jit.last().copied().unwrap_or_else(empty_state),
                    pc_trail: pc_trail(&interp, interp.len()),
                }));
            }
        }

        compare_traces(&interp, &jit, policy)?;
        Ok(LockstepReport {
            steps_compared: interp.len() as u64,
            interp_final: interp.last().copied(),
            jit_final: jit.last().copied(),
        })
    }
}

fn empty_state() -> CpuState {
    CpuState {
        step: 0,
        pc: 0,
        ps: 0,
        sar: 0,
        lbeg: 0,
        lend: 0,
        lcount: 0,
        ccount: 0,
        ar: [0; 16],
    }
}

/// Summary returned on a successful `run_and_compare`.
#[derive(Debug)]
pub struct LockstepReport {
    pub steps_compared: u64,
    pub interp_final: Option<CpuState>,
    pub jit_final: Option<CpuState>,
}

/// Output of [`LockstepRunner::record_both`] — both traces plus any
/// per-side step errors. Boxed into a `type` alias to keep clippy's
/// `type_complexity` lint happy without forcing callers through a
/// constructor.
pub type RecordedTraces = (
    Vec<CpuState>,
    Vec<CpuState>,
    Option<SimulationError>,
    Option<SimulationError>,
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::XtensaLx7;
    use crate::{Bus, Machine};

    /// Builds a tiny machine whose RAM holds two NOP.N instructions
    /// followed by an illegal instruction. PC starts at the first NOP.
    /// Used by the unit-level harness self-tests so they don't depend
    /// on the 12 MB labwired-ereader ELF.
    fn build_nop_machine() -> SimResult<Machine<XtensaLx7>> {
        let mut bus = SystemBus::new();
        let mut cpu = XtensaLx7::new();
        cpu.reset(&mut bus)?;
        const PC: u32 = 0x2000_0000;
        cpu.set_pc(PC);
        // 10× NOP.N at PC, PC+2, …
        for i in 0..10 {
            bus.write_u8(PC as u64 + (i * 2) as u64, 0x3d).unwrap();
            bus.write_u8(PC as u64 + (i * 2) as u64 + 1, 0xf0).unwrap();
        }
        Ok(Machine::new(cpu, bus))
    }

    #[test]
    fn no_op_jit_matches_interpreter_on_nop_stream() {
        // Today JIT == interpreter (no emit code). Harness must report
        // zero divergence — proves the harness itself doesn't introduce
        // false positives.
        let runner = LockstepRunner::new(build_nop_machine, 8);
        let report = runner
            .run_and_compare()
            .expect("no-op JIT must not diverge from interpreter");
        assert_eq!(report.steps_compared, 8);
    }

    #[test]
    fn deliberate_register_corruption_is_detected() {
        // Confirm the diff logic is real: inject a corrupted a4 into the
        // JIT machine before stepping. Harness MUST flag it.
        let runner = LockstepRunner::new(build_nop_machine, 4);
        let (interp, jit, _, _) = runner
            .record_both(|m| {
                m.cpu.set_register(4, 0xDEAD_BEEF);
                Ok(())
            })
            .expect("record_both must not fail");
        let err = compare_traces(&interp, &jit, ComparePolicy::default())
            .expect_err("must detect corrupted a4");
        assert_eq!(err.field, "a4");
        assert_eq!(err.step, 0);
        assert_eq!(err.jit.ar[4], 0xDEAD_BEEF);
        assert_ne!(err.interp.ar[4], 0xDEAD_BEEF);
    }

    #[test]
    fn ccount_tolerance_swallows_one_cycle_drift() {
        let mut a = empty_state();
        let mut b = empty_state();
        a.ccount = 100;
        b.ccount = 101;
        assert!(compare_traces(&[a], &[b], ComparePolicy::default()).is_ok());
        // ±2 must trip.
        b.ccount = 102;
        let err = compare_traces(&[a], &[b], ComparePolicy::default()).unwrap_err();
        assert_eq!(err.field, "ccount");
    }

    #[test]
    fn pc_divergence_is_detected_first() {
        let mut a = empty_state();
        let mut b = empty_state();
        a.pc = 0x1000;
        b.pc = 0x2000;
        a.ar[5] = 0x11;
        b.ar[5] = 0x22;
        let err = compare_traces(&[a], &[b], ComparePolicy::default()).unwrap_err();
        // PC checked before AR.
        assert_eq!(err.field, "pc");
    }
}
