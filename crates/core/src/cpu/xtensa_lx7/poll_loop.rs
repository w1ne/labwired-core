// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CCOUNT time-poll loop fast-forward.
//!
//! A busy-wait on the cycle counter is the commonest way an Arduino sketch
//! times a pulse or a frame:
//!
//! ```text
//! 400d12c1: rsr.ccount a3          ; anchor
//! 400d12c4: sub        a3, a3, a2  ; dt = ccount - t0
//! 400d12c7: bgeu       a4, a3, 400d12c1
//! ```
//!
//! Such a loop is not idle (no WAITI), so the idle skip never saw it and the
//! interpreter ran every iteration: 240 M instructions per simulated second
//! of ESP32 time. This module proves, for one iteration, that the loop is a
//! pure function of the CCOUNT value read at its anchor, and computes the
//! exact iteration at which a branch outcome first changes. The machine then
//! skips the steady iterations as it skips a WAITI window, and the CPU
//! resumes at the anchor of the first iteration that behaves differently. The
//! result is the same cycle-for-cycle as running every iteration.
//!
//! **The proof.** One iteration is run from the current state (just after the
//! anchor `RSR CCOUNT`) on a read-only view of memory, with a taint per
//! logical register: the anchor's destination has slope +1 in the CCOUNT
//! value `v`, and every value computed from it by add/sub/neg/mov keeps a
//! slope of +1 or -1. The iteration is refused when:
//! - an instruction outside a pure whitelist runs (stores, calls, window
//!   ops, SR writes, exceptions, FP, ...);
//! - a load reads anything but plain RAM/flash (MMIO reads can have side
//!   effects or change between iterations);
//! - a tainted value reaches anything but add/sub/neg/mov and an ordered or
//!   equality compare-branch against an untainted value;
//! - an interrupt could be taken or CCOMPARE0 crossed;
//! - it does not return to the anchor in exactly the observed period, or
//!   the state it returns with differs from the start state in anything but
//!   the tainted registers and CCOUNT.
//!
//! Then iteration `j` sees every tainted value shifted by `s * j * period`
//! and every untainted value unchanged, so its path is the same as iteration
//! 0's until one of the recorded compares changes outcome. For a compare of an
//! affine value against a constant that happens only where the value reaches
//! one of a handful of boundary values (the constant, its neighbours, and the
//! unsigned and signed wrap points), so the first change is found exactly by
//! evaluating the compare there. No monotonicity is assumed.

use super::{xtensa, xtensa_length, xtensa_narrow, XtensaLx7, DECODE_CACHE_MASK};
use crate::cpu::xtensa_sr::{CCOMPARE0, CCOUNT, EPC1, INTERRUPT, LBEG, LCOUNT, LEND, SAR};

/// The only SRs a probed instruction can write: the zero-overhead loop
/// registers (`LOOP*`), SAR (`SSL`/`SSR`/`SSA*`), CCOUNT (the probe's own
/// clock), and EPC1 (a faithful-window overflow, which also changes PS and is
/// refused). The probe saves, compares and restores exactly these.
const PROBE_SRS: [u16; 6] = [LBEG, LEND, LCOUNT, SAR, EPC1, CCOUNT];
use crate::{Bus, DmaRequest, SimResult, SimulationConfig, SimulationError};

/// Longest loop iteration, in instructions, the detector considers.
const MAX_PERIOD: u32 = 512;
/// Anchor arrivals to sit out after a refused probe at that anchor.
const REFUSE_BACKOFF: u32 = 64;
/// Most tainted compares one iteration may execute.
const MAX_COMPARES: usize = 32;

/// What the core remembers about its last `RSR CCOUNT`. Cheap enough to
/// update on every such read.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PollTracker {
    have_last: bool,
    last_pc: u32,
    /// CCOUNT value the last `RSR CCOUNT` read. CCOUNT still equals it while
    /// no instruction has retired since.
    last_value: u32,
    /// Distance in CCOUNT between the last two reads at `last_pc`, when they
    /// were consecutive reads at the same PC and at most `MAX_PERIOD` apart.
    period: u32,
    refused_pc: u32,
    refused_left: u32,
}

/// A proven skip, computed by [`XtensaLx7::poll_loop_budget`] and applied by
/// [`XtensaLx7::apply_poll_loop_skip`].
#[derive(Clone, Debug)]
pub(crate) struct PollPlan {
    period: u32,
    /// Physical AR file after one iteration.
    end_phys: [u32; 64],
    window_base: u8,
    /// Slope of each logical register after one iteration (0 = unchanged).
    slopes: [i8; 16],
    /// Iterations that are proven steady (`u64::MAX`: no compare ever flips).
    steady_iters: u64,
    /// Every byte range the iteration read (its code and its loads), with
    /// the bytes the proof saw. A deferred loop is valid while they hold.
    reads: Vec<(u32, Vec<u8>)>,
}

/// Most distinct byte ranges one proven iteration may read.
const MAX_READ_RANGES: usize = 8;

#[derive(Clone, Copy, Debug)]
enum CmpKind {
    Eq,
    Ne,
    Lt,
    Ge,
    Ltu,
    Geu,
}

#[derive(Clone, Copy, Debug)]
struct Operand {
    value: u32,
    slope: i8,
}

#[derive(Clone, Copy, Debug)]
struct Compare {
    kind: CmpKind,
    a: Operand,
    b: Operand,
}

impl CmpKind {
    fn eval(self, a: u32, b: u32) -> bool {
        match self {
            CmpKind::Eq => a == b,
            CmpKind::Ne => a != b,
            CmpKind::Lt => (a as i32) < (b as i32),
            CmpKind::Ge => (a as i32) >= (b as i32),
            CmpKind::Ltu => a < b,
            CmpKind::Geu => a >= b,
        }
    }
}

impl Compare {
    /// Smallest `k >= 1` at which the outcome differs from `k = 0`, where the
    /// tainted operand is `value + slope * k`. `None` when it never does.
    fn first_flip(&self) -> Option<u64> {
        let (t0, slope, c, tainted_is_a) = if self.a.slope != 0 {
            (self.a.value, self.a.slope, self.b.value, true)
        } else {
            (self.b.value, self.b.slope, self.a.value, false)
        };
        let at = |k: u32| -> bool {
            let v = if slope > 0 {
                t0.wrapping_add(k)
            } else {
                t0.wrapping_sub(k)
            };
            if tainted_is_a {
                self.kind.eval(v, c)
            } else {
                self.kind.eval(c, v)
            }
        };
        let p0 = at(0);
        // The outcome of `x OP c` can change only where x arrives at one of
        // these values (c's neighbourhood, the unsigned wrap, the signed wrap).
        let boundaries = [
            c.wrapping_sub(1),
            c,
            c.wrapping_add(1),
            0,
            u32::MAX,
            0x7FFF_FFFF,
            0x8000_0000,
        ];
        let mut best: Option<u64> = None;
        let mut consider = |k: u32| {
            if k != 0 && at(k) != p0 {
                let k = u64::from(k);
                best = Some(best.map_or(k, |b| b.min(k)));
            }
        };
        consider(1);
        for v in boundaries {
            let k = if slope > 0 {
                v.wrapping_sub(t0)
            } else {
                t0.wrapping_sub(v)
            };
            consider(k);
        }
        best
    }
}

/// Read-only view of the bus for the probe: loads are served from plain
/// memory only, and every write is refused, so the probe cannot change
/// anything outside the CPU.
struct ProbeBus<'a> {
    inner: &'a dyn Bus,
    /// Every read, as `(addr, size, value)`, when recording.
    log: Option<std::cell::RefCell<Vec<(u32, u8, u32)>>>,
    /// Reads served from these byte ranges instead of `inner` (a deferred
    /// loop catching up sees the memory its proof saw).
    snapshot: &'a [(u32, Vec<u8>)],
}

impl<'a> ProbeBus<'a> {
    fn new(inner: &'a dyn Bus) -> Self {
        Self {
            inner,
            log: None,
            snapshot: &[],
        }
    }

    fn plain_read(&self, addr: u64, size: usize) -> SimResult<u32> {
        for (base, bytes) in self.snapshot {
            let base = u64::from(*base);
            if addr >= base && addr + size as u64 <= base + bytes.len() as u64 {
                let off = (addr - base) as usize;
                let mut v = 0u32;
                for (i, b) in bytes[off..off + size].iter().enumerate() {
                    v |= u32::from(*b) << (8 * i);
                }
                return Ok(v);
            }
        }
        let v = self.plain_read_live(addr, size)?;
        if let Some(log) = &self.log {
            log.borrow_mut().push((addr as u32, size as u8, v));
        }
        Ok(v)
    }

    fn plain_read_live(&self, addr: u64, size: usize) -> SimResult<u32> {
        let (start, end, slice) = self
            .inner
            .fetch_slice(addr)
            .ok_or(SimulationError::MemoryViolation(addr))?;
        if addr < start || addr + size as u64 > end {
            return Err(SimulationError::MemoryViolation(addr));
        }
        let off = (addr - start) as usize;
        let bytes = slice
            .get(off..off + size)
            .ok_or(SimulationError::MemoryViolation(addr))?;
        let mut v = 0u32;
        for (i, b) in bytes.iter().enumerate() {
            v |= u32::from(*b) << (8 * i);
        }
        Ok(v)
    }
}

impl Bus for ProbeBus<'_> {
    fn read_u8(&self, addr: u64) -> SimResult<u8> {
        self.plain_read(addr, 1).map(|v| v as u8)
    }
    fn read_u16(&self, addr: u64) -> SimResult<u16> {
        self.plain_read(addr, 2).map(|v| v as u16)
    }
    fn read_u32(&self, addr: u64) -> SimResult<u32> {
        self.plain_read(addr, 4)
    }
    fn write_u8(&mut self, addr: u64, _value: u8) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(addr))
    }
    fn write_u16(&mut self, addr: u64, _value: u16) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(addr))
    }
    fn write_u32(&mut self, addr: u64, _value: u32) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(addr))
    }
    fn write_u64(&mut self, addr: u64, _value: u64) -> SimResult<()> {
        Err(SimulationError::MemoryViolation(addr))
    }
    fn tick_peripherals(&mut self) -> Vec<u32> {
        Vec::new()
    }
    fn execute_dma(&mut self, _requests: &[DmaRequest]) -> SimResult<()> {
        Err(SimulationError::Other("probe bus: no DMA".into()))
    }
    fn config(&self) -> &SimulationConfig {
        self.inner.config()
    }
    fn pending_cpu_irqs(&self, core_id: u8) -> u32 {
        self.inner.pending_cpu_irqs(core_id)
    }
    fn fetch_slice(&self, pc: u64) -> Option<(u64, u64, &[u8])> {
        self.inner.fetch_slice(pc)
    }
}

/// Per-iteration register bookkeeping for the probe.
struct Taint {
    slopes: [i8; 16],
    written: u16,
    read_before_write: u16,
    compares: [Option<Compare>; MAX_COMPARES],
    n_compares: usize,
}

impl Taint {
    fn slope(&mut self, r: u8) -> i8 {
        let bit = 1u16 << (r & 15);
        if self.written & bit == 0 {
            self.read_before_write |= bit;
        }
        self.slopes[(r & 15) as usize]
    }
    fn set(&mut self, r: u8, slope: i8) {
        self.written |= 1u16 << (r & 15);
        self.slopes[(r & 15) as usize] = slope;
    }
    /// Sources that must be untainted, then an untainted destination.
    fn pure(&mut self, srcs: &[u8], dst: Option<u8>) -> Result<(), ()> {
        for &r in srcs {
            if self.slope(r) != 0 {
                return Err(());
            }
        }
        if let Some(d) = dst {
            self.set(d, 0);
        }
        Ok(())
    }
    fn affine(&mut self, dst: u8, slope: i32) -> Result<(), ()> {
        if !(-1..=1).contains(&slope) {
            return Err(());
        }
        self.set(dst, slope as i8);
        Ok(())
    }
    fn compare(&mut self, kind: CmpKind, a: Operand, b: Operand) -> Result<(), ()> {
        if a.slope == 0 && b.slope == 0 {
            return Ok(());
        }
        if a.slope != 0 && b.slope != 0 {
            return Err(());
        }
        if self.n_compares == MAX_COMPARES {
            return Err(());
        }
        self.compares[self.n_compares] = Some(Compare { kind, a, b });
        self.n_compares += 1;
        Ok(())
    }
}

/// The probe's reads as merged byte ranges. `None` when there are too many
/// to check cheaply, or two reads of one byte disagree.
fn merge_reads(mut log: Vec<(u32, u8, u32)>) -> Option<Vec<(u32, Vec<u8>)>> {
    log.sort_unstable_by_key(|r| r.0);
    let mut out: Vec<(u32, Vec<u8>)> = Vec::new();
    for (addr, size, value) in log {
        let bytes: Vec<u8> = (0..size).map(|i| (value >> (8 * i)) as u8).collect();
        if let Some((base, have)) = out.last_mut() {
            let end = base.wrapping_add(have.len() as u32);
            if addr <= end {
                for (i, b) in bytes.iter().enumerate() {
                    let at = (addr - *base) as usize + i;
                    match have.get(at) {
                        Some(old) if old != b => return None,
                        Some(_) => {}
                        None => have.push(*b),
                    }
                }
                continue;
            }
        }
        out.push((addr, bytes));
    }
    (out.len() <= MAX_READ_RANGES).then_some(out)
}

impl XtensaLx7 {
    /// Called by `RSR CCOUNT` with its PC and the value it read.
    #[inline]
    pub(super) fn note_ccount_read(&mut self, pc: u32, value: u32) {
        let t = &mut self.poll;
        if t.have_last && t.last_pc == pc {
            let d = value.wrapping_sub(t.last_value);
            t.period = if d > 0 && d <= MAX_PERIOD { d } else { 0 };
            if t.refused_left > 0 && t.refused_pc == pc {
                t.refused_left -= 1;
            }
        } else {
            t.period = 0;
        }
        t.have_last = true;
        t.last_pc = pc;
        t.last_value = value;
    }

    /// True right after the anchor `RSR CCOUNT` of a loop that has come round
    /// to the same PC at least twice, `period` cycles apart.
    #[inline]
    pub(super) fn poll_anchor_ready(&self) -> bool {
        let t = &self.poll;
        t.period > 0
            && !self.halted
            && !self.waiti_parked
            && self.sr.read(CCOUNT) == t.last_value
            && !(t.refused_left > 0 && t.refused_pc == t.last_pc)
    }

    /// Prove the loop at the current anchor and return `(period, cycles)`:
    /// the iteration length and the most cycles (a multiple of it, at most
    /// `limit`) that may be skipped. `None` refuses; nothing has changed.
    pub(super) fn poll_loop_budget(&mut self, bus: &dyn Bus, limit: u64) -> Option<(u64, u64)> {
        self.poll_plan = None;
        if !self.poll_anchor_ready() {
            return None;
        }
        let period = self.poll.period;
        if limit < u64::from(period) {
            return None;
        }
        if !self.ps.excm() {
            if let Some(level) = self.pending_irq_level(bus) {
                if level > self.ps.intlevel() {
                    return None;
                }
            }
        }
        let Some(plan) = self.probe_poll_iteration(bus, period) else {
            self.poll.refused_pc = self.poll.last_pc;
            self.poll.refused_left = REFUSE_BACKOFF;
            return None;
        };
        let iters = plan.steady_iters.min(limit / u64::from(period));
        if iters == 0 {
            return None;
        }
        self.poll_plan = Some(plan);
        Some((u64::from(period), iters * u64::from(period)))
    }

    /// Skip `cycles` (a multiple of the period of the plan the last
    /// [`Self::poll_loop_budget`] returned, and at most its cycles): leave
    /// the core exactly where running those iterations would leave it.
    pub(super) fn apply_poll_loop_skip(&mut self, cycles: u64) -> bool {
        let Some(plan) = self.poll_plan.take() else {
            return false;
        };
        let period = u64::from(plan.period);
        if cycles == 0 || cycles % period != 0 || cycles / period > plan.steady_iters {
            return false;
        }
        let iters = cycles / period;
        // State after one iteration, with every tainted register moved on by
        // the remaining `iters - 1` iterations.
        let shift = ((iters - 1) * period) as u32;
        let mut phys = plan.end_phys;
        for (logical, &slope) in plan.slopes.iter().enumerate() {
            if slope == 0 {
                continue;
            }
            let p = (usize::from(plan.window_base) * 4 + logical) % 64;
            phys[p] = if slope > 0 {
                phys[p].wrapping_add(shift)
            } else {
                phys[p].wrapping_sub(shift)
            };
        }
        self.regs.set_phys(phys);
        // CCOUNT (and a CCOMPARE0 crossing) moves the whole way, as for a
        // WAITI skip; the anchor read the value CCOUNT now holds.
        use crate::Cpu;
        self.fast_forward_idle_cycles(cycles);
        self.poll.last_value = self.sr.read(CCOUNT);
        self.bus_irq_memo = None;
        true
    }

    fn probe_decode(&self, bus: &ProbeBus<'_>, pc: u32) -> SimResult<(u32, xtensa::Instruction)> {
        let entry = &self.decode_cache[(pc as usize >> 1) & DECODE_CACHE_MASK];
        if entry.generation == self.cur_decode_gen && entry.tag == pc {
            // A recording probe logs the code bytes too, when they are plain
            // memory (a store can reach them); flash-mapped code is read-only.
            if bus.log.is_some() {
                let _ = bus.plain_read(u64::from(pc), entry.len as usize);
            }
            return Ok((entry.len, entry.ins));
        }
        let b0 = bus.read_u8(u64::from(pc))?;
        let len = xtensa_length::instruction_length(b0);
        let ins = if len == 2 {
            xtensa_narrow::decode_narrow(bus.read_u16(u64::from(pc))?)
        } else {
            xtensa::decode(bus.read_u32(u64::from(pc))?)
        };
        Ok((len, ins))
    }

    /// Run one iteration from the current state, then put the state back.
    fn probe_poll_iteration(&mut self, bus: &dyn Bus, period: u32) -> Option<PollPlan> {
        let save_phys = *self.regs.phys_slice();
        let save_wb = self.regs.windowbase();
        let save_ws = self.regs.windowstart();
        let save_sr = PROBE_SRS.map(|sr| self.sr.read(sr));
        let save_ps = self.ps;
        let save_pc = self.pc;
        let save_br = self.br;
        let save_branched = self.branched;
        let save_tracker = self.poll;
        let save_memo = self.bus_irq_memo;

        let result = self.probe_run(bus, period, &save_phys, save_wb, save_ws, &save_sr);

        self.regs.set_phys(save_phys);
        self.regs.set_windowbase(save_wb);
        self.regs.set_windowstart(save_ws);
        for (sr, v) in PROBE_SRS.iter().zip(save_sr) {
            self.sr.set_raw(*sr, v);
        }
        self.ps = save_ps;
        self.pc = save_pc;
        self.br = save_br;
        self.branched = save_branched;
        self.poll = save_tracker;
        self.bus_irq_memo = save_memo;
        result
    }

    fn probe_run(
        &mut self,
        bus: &dyn Bus,
        period: u32,
        start_phys: &[u32; 64],
        start_wb: u8,
        start_ws: u16,
        start_sr: &[u32; PROBE_SRS.len()],
    ) -> Option<PollPlan> {
        use xtensa::Instruction::*;
        let anchor_pc = self.poll.last_pc;
        let start_pc = self.pc;
        let start_ps = self.ps.as_raw();
        let start_br = self.br;
        let mut probe_bus = ProbeBus::new(bus);
        probe_bus.log = Some(std::cell::RefCell::new(Vec::new()));
        // A fetch the probe cannot serve from plain memory refuses the skip;
        // the real step then runs that instruction and reports its own error.
        let anchor_dst = match self.probe_decode(&probe_bus, anchor_pc) {
            Ok((_, Rsr { at, sr })) if sr == CCOUNT => at,
            _ => return None,
        };
        let mut taint = Taint {
            slopes: [0; 16],
            written: 0,
            read_before_write: 0,
            compares: [None; MAX_COMPARES],
            n_compares: 0,
        };
        taint.slopes[usize::from(anchor_dst & 15)] = 1;
        let initial_taint = 1u16 << (anchor_dst & 15);

        let mut closed = false;
        for n in 1..=period {
            // CCOUNT and CCOMPARE0 exactly as `step_body`; a crossing ends the
            // probe (the machine clamps the skip to the edge anyway).
            let before = self.sr.read(CCOUNT);
            let after = before.wrapping_add(1);
            self.sr.write(CCOUNT, after);
            if self.sr.ccompare0_armed() {
                let cc0 = self.sr.read(CCOMPARE0);
                if before < cc0 && after >= cc0 {
                    return None;
                }
            }
            if !self.ps.excm() {
                if let Some(level) = self.pending_irq_level(bus) {
                    if level > self.ps.intlevel() {
                        return None;
                    }
                }
            }
            let pc = self.pc;
            let Ok((len, ins)) = self.probe_decode(&probe_bus, pc) else {
                return None;
            };
            self.probe_taint(&ins, &mut taint).ok()?;
            self.branched = false;
            let fall_through = pc.wrapping_add(len);
            self.execute_hot(ins, &mut probe_bus, len).ok()?;
            let lcount = self.sr.read(LCOUNT);
            let lend = self.sr.read(LEND);
            if lcount > 0 && self.pc == lend && fall_through == lend && !self.branched {
                self.sr.write(LCOUNT, lcount - 1);
                self.pc = self.sr.read(LBEG);
            }
            if pc == anchor_pc {
                if n != period {
                    return None;
                }
                closed = true;
                break;
            }
        }
        if !closed {
            return None;
        }

        // Back at the anchor with the same control state.
        if self.pc != start_pc
            || self.ps.as_raw() != start_ps
            || self.br != start_br
            || self.regs.windowbase() != start_wb
            || self.regs.windowstart() != start_ws
        {
            return None;
        }
        for (sr, start) in PROBE_SRS.iter().zip(start_sr) {
            if *sr != CCOUNT && self.sr.read(*sr) != *start {
                return None;
            }
        }
        // A register still carrying a value of the previous iteration into
        // this one would accumulate, not shift: refuse unless it started
        // tainted (only the anchor's own destination does).
        let tainted_mask: u16 = taint
            .slopes
            .iter()
            .enumerate()
            .filter(|(_, s)| **s != 0)
            .fold(0, |m, (i, _)| m | (1 << i));
        if tainted_mask & taint.read_before_write & !initial_taint != 0 {
            return None;
        }
        if taint.slopes[usize::from(anchor_dst & 15)] != 1 {
            return None;
        }
        let end_phys = *self.regs.phys_slice();
        for p in 0..64 {
            let logical = (p + 64 - usize::from(start_wb) * 4) % 64;
            let tainted = logical < 16 && taint.slopes[logical] != 0;
            if !tainted && end_phys[p] != start_phys[p] {
                return None;
            }
        }

        let mut steady_iters = u64::MAX;
        for cmp in taint.compares[..taint.n_compares].iter().flatten() {
            if let Some(k) = cmp.first_flip() {
                // Iteration j compares at `k = j * period`: steady while
                // j * period < k.
                steady_iters = steady_iters.min(k.div_ceil(u64::from(period)));
            }
        }
        let reads = merge_reads(probe_bus.log.take()?.into_inner())?;
        Some(PollPlan {
            period,
            end_phys,
            window_base: start_wb,
            slopes: taint.slopes,
            steady_iters,
            reads,
        })
    }

    /// See [`crate::Cpu::poll_loop_watch`].
    pub(super) fn poll_watch(&self, out: &mut Vec<(u64, u64)>) {
        if let Some(plan) = self.poll_plan.as_ref() {
            out.extend(plan.reads.iter().map(|(base, bytes)| {
                let lo = u64::from(*base);
                (lo, lo + bytes.len() as u64)
            }));
        }
    }

    /// See [`crate::Cpu::poll_loop_defer_changed`].
    pub(super) fn poll_defer_changed(&self, bus: &dyn Bus) -> bool {
        let Some(plan) = self.poll_plan.as_ref() else {
            return true;
        };
        for (base, bytes) in &plan.reads {
            let Some((start, end, slice)) = bus.fetch_slice(u64::from(*base)) else {
                return true;
            };
            let lo = u64::from(*base);
            let hi = lo + bytes.len() as u64;
            if lo < start || hi > end {
                return true;
            }
            let off = (lo - start) as usize;
            if slice.get(off..off + bytes.len()) != Some(bytes.as_slice()) {
                return true;
            }
        }
        if !self.ps.excm() {
            if let Some(level) = self.pending_irq_level(bus) {
                if level > self.ps.intlevel() {
                    return true;
                }
            }
        }
        false
    }

    /// See [`crate::Cpu::poll_loop_catch_up`]: whole iterations as
    /// [`Self::apply_poll_loop_skip`], then the rest of the next one
    /// instruction by instruction, as the probe runs it, on the memory the
    /// proof read. That partial iteration is a prefix of a steady one (the
    /// deferral never outlives the plan's budget), so it takes the proven
    /// path; the machine ends a deferral before the CCOMPARE0 edge, so no
    /// timer interrupt is due inside it, and before any other interrupt
    /// becomes deliverable.
    pub(super) fn poll_catch_up(&mut self, bus: &dyn Bus, cycles: u64) -> bool {
        let Some(plan) = self.poll_plan.as_ref() else {
            return false;
        };
        let period = u64::from(plan.period);
        if cycles / period > plan.steady_iters {
            return false;
        }
        let reads = plan.reads.clone();
        let whole = cycles - cycles % period;
        if whole > 0 {
            if !self.apply_poll_loop_skip(whole) {
                return false;
            }
        } else {
            self.poll_plan = None;
        }
        let mut probe_bus = ProbeBus::new(bus);
        probe_bus.snapshot = &reads;
        for _ in 0..cycles % period {
            let before = self.sr.read(CCOUNT);
            let after = before.wrapping_add(1);
            self.sr.write(CCOUNT, after);
            debug_assert!(
                !(self.sr.ccompare0_armed() && {
                    let cc0 = self.sr.read(CCOMPARE0);
                    before < cc0 && after >= cc0
                }),
                "a deferred poll loop caught up across its CCOMPARE0 edge"
            );
            let pc = self.pc;
            // Code the decode cache no longer holds and plain memory cannot
            // serve (flash-mapped) is read through the bus, as a step reads it.
            let decoded = self.probe_decode(&probe_bus, pc).or_else(|_| {
                let b0 = bus.read_u8(u64::from(pc))?;
                let len = xtensa_length::instruction_length(b0);
                let ins = if len == 2 {
                    xtensa_narrow::decode_narrow(bus.read_u16(u64::from(pc))?)
                } else {
                    xtensa::decode(bus.read_u32(u64::from(pc))?)
                };
                Ok::<_, SimulationError>((len, ins))
            });
            let Ok((len, ins)) = decoded else {
                debug_assert!(false, "deferred poll loop: fetch failed at {pc:#x}");
                return false;
            };
            self.branched = false;
            let fall_through = pc.wrapping_add(len);
            if self.execute_hot(ins, &mut probe_bus, len).is_err() {
                debug_assert!(false, "deferred poll loop: step failed at {pc:#x}");
                return false;
            }
            let lcount = self.sr.read(LCOUNT);
            let lend = self.sr.read(LEND);
            if lcount > 0 && self.pc == lend && fall_through == lend && !self.branched {
                self.sr.write(LCOUNT, lcount - 1);
                self.pc = self.sr.read(LBEG);
            }
        }
        self.bus_irq_memo = None;
        true
    }

    /// Taint transfer for one instruction, before it runs. `Err` refuses.
    fn probe_taint(&self, ins: &xtensa::Instruction, t: &mut Taint) -> Result<(), ()> {
        use xtensa::Instruction::*;
        let reg = |r: u8| self.regs.read_logical(r);
        let op = |t: &mut Taint, r: u8| Operand {
            value: reg(r),
            slope: t.slope(r),
        };
        let imm = |v: u32| Operand { value: v, slope: 0 };
        match *ins {
            Add { ar, as_, at } => {
                let s = i32::from(t.slope(as_)) + i32::from(t.slope(at));
                t.affine(ar, s)
            }
            Sub { ar, as_, at } => {
                let s = i32::from(t.slope(as_)) - i32::from(t.slope(at));
                t.affine(ar, s)
            }
            Addi { at, as_, .. } | Addmi { at, as_, .. } => {
                let s = t.slope(as_);
                t.affine(at, i32::from(s))
            }
            Neg { ar, at } => {
                let s = t.slope(at);
                t.affine(ar, -i32::from(s))
            }
            Or { ar, as_, at } if as_ == at => {
                let s = t.slope(as_);
                t.affine(ar, i32::from(s))
            }
            Addx2 { ar, as_, at }
            | Addx4 { ar, as_, at }
            | Addx8 { ar, as_, at }
            | Subx2 { ar, as_, at }
            | Subx4 { ar, as_, at }
            | Subx8 { ar, as_, at } => {
                if t.slope(as_) != 0 {
                    return Err(());
                }
                let s = t.slope(at);
                let s = if matches!(*ins, Subx2 { .. } | Subx4 { .. } | Subx8 { .. }) {
                    -i32::from(s)
                } else {
                    i32::from(s)
                };
                t.affine(ar, s)
            }
            Movi { at, .. } => t.pure(&[], Some(at)),
            And { ar, as_, at }
            | Or { ar, as_, at }
            | Xor { ar, as_, at }
            | Src { ar, as_, at }
            | Salt { ar, as_, at }
            | Saltu { ar, as_, at }
            | Mull { ar, as_, at }
            | Muluh { ar, as_, at }
            | Mulsh { ar, as_, at }
            | Mul16s { ar, as_, at }
            | Mul16u { ar, as_, at }
            | Min { ar, as_, at }
            | Max { ar, as_, at }
            | Minu { ar, as_, at }
            | Maxu { ar, as_, at } => t.pure(&[as_, at], Some(ar)),
            // Conditional moves keep the old `ar` when they do not move.
            Moveqz { ar, as_, at }
            | Movnez { ar, as_, at }
            | Movltz { ar, as_, at }
            | Movgez { ar, as_, at } => t.pure(&[ar, as_, at], Some(ar)),
            Abs { ar, at } | Srl { ar, at } | Sra { ar, at } => t.pure(&[at], Some(ar)),
            Srli { ar, at, .. } | Srai { ar, at, .. } | Extui { ar, at, .. } => {
                t.pure(&[at], Some(ar))
            }
            Sll { ar, as_ } | Slli { ar, as_, .. } | Nsa { ar, as_ } | Nsau { ar, as_ } => {
                t.pure(&[as_], Some(ar))
            }
            Sext { ar, as_, .. } | Clamps { ar, as_, .. } => t.pure(&[as_], Some(ar)),
            Ssl { as_ } | Ssr { as_ } | Ssa8l { as_ } | Ssa8b { as_ } => t.pure(&[as_], None),
            Ssai { .. } | Nop | Memw | Extw | Isync | Rsync | Esync | Dsync | J { .. } => Ok(()),
            Jx { as_ } => t.pure(&[as_], None),
            L8ui { at, as_, .. } | L16ui { at, as_, .. } | L16si { at, as_, .. } => {
                t.pure(&[as_], Some(at))
            }
            L32i { at, as_, .. } => t.pure(&[as_], Some(at)),
            L32r { at, .. } => t.pure(&[], Some(at)),
            Rsr { at, sr } if sr == CCOUNT => {
                t.set(at, 1);
                Ok(())
            }
            // Reading INTERRUPT folds in live bus state; other SRs are
            // plain storage.
            Rsr { at, sr } if sr != INTERRUPT => t.pure(&[], Some(at)),
            Loop { as_, .. } | Loopnez { as_, .. } | Loopgtz { as_, .. } => t.pure(&[as_], None),
            Beq { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Eq, a, b)
            }
            Bne { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Ne, a, b)
            }
            Blt { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Lt, a, b)
            }
            Bge { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Ge, a, b)
            }
            Bltu { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Ltu, a, b)
            }
            Bgeu { as_, at, .. } => {
                let (a, b) = (op(t, as_), op(t, at));
                t.compare(CmpKind::Geu, a, b)
            }
            Beqz { as_, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Eq, a, imm(0))
            }
            Bnez { as_, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Ne, a, imm(0))
            }
            Bltz { as_, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Lt, a, imm(0))
            }
            Bgez { as_, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Ge, a, imm(0))
            }
            Beqi { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Eq, a, imm(i as u32))
            }
            Bnei { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Ne, a, imm(i as u32))
            }
            Blti { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Lt, a, imm(i as u32))
            }
            Bgei { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Ge, a, imm(i as u32))
            }
            Bltui { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Ltu, a, imm(i))
            }
            Bgeui { as_, imm: i, .. } => {
                let a = op(t, as_);
                t.compare(CmpKind::Geu, a, imm(i))
            }
            Bany { as_, at, .. }
            | Ball { as_, at, .. }
            | Bnone { as_, at, .. }
            | Bnall { as_, at, .. }
            | Bbc { as_, at, .. }
            | Bbs { as_, at, .. } => t.pure(&[as_, at], None),
            Bbci { as_, .. } | Bbsi { as_, .. } => t.pure(&[as_], None),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flip(kind: CmpKind, a: (u32, i8), b: (u32, i8)) -> Option<u64> {
        Compare {
            kind,
            a: Operand {
                value: a.0,
                slope: a.1,
            },
            b: Operand {
                value: b.0,
                slope: b.1,
            },
        }
        .first_flip()
    }

    #[test]
    fn first_flip_is_exact_for_every_compare_shape() {
        // Brute force over a small window agrees with the boundary method.
        let kinds = [
            CmpKind::Eq,
            CmpKind::Ne,
            CmpKind::Lt,
            CmpKind::Ge,
            CmpKind::Ltu,
            CmpKind::Geu,
        ];
        let values = [
            0u32,
            1,
            5,
            100,
            0x7FFF_FFF0,
            0x7FFF_FFFF,
            0x8000_0000,
            0x8000_0005,
            u32::MAX - 3,
            u32::MAX,
        ];
        for kind in kinds {
            for &t0 in &values {
                for &c in &values {
                    for slope in [1i8, -1] {
                        for tainted_a in [true, false] {
                            let (a, b) = if tainted_a {
                                ((t0, slope), (c, 0))
                            } else {
                                ((c, 0), (t0, slope))
                            };
                            let got = flip(kind, a, b);
                            let at = |k: u32| {
                                let v = if slope > 0 {
                                    t0.wrapping_add(k)
                                } else {
                                    t0.wrapping_sub(k)
                                };
                                if tainted_a {
                                    kind.eval(v, c)
                                } else {
                                    kind.eval(c, v)
                                }
                            };
                            // Every k before the answer keeps the k=0 outcome.
                            let limit = got.unwrap_or(1 << 12).min(1 << 12) as u32;
                            for k in 0..limit {
                                assert_eq!(
                                    at(k),
                                    at(0),
                                    "{kind:?} t0={t0:#x} c={c:#x} s={slope} a={tainted_a} k={k}"
                                );
                            }
                            if let Some(k) = got {
                                assert_ne!(at(k as u32), at(0));
                            }
                        }
                    }
                }
            }
        }
    }
}
