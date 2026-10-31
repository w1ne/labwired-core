// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Fixed-point poll-loop fast-forward for Cortex-M.
//!
//! The commonest Cortex-M wait is a busy loop on a tick counter that only an
//! interrupt changes. Arduino `delay()` on STM32duino, `HAL_Delay`, and most
//! RTOS-less sketches look like this:
//!
//! ```text
//! 80074fc: bl   yield            ; bx lr
//! 8007500: bl   getCurrentMillis ; ldr r3,=uwTick; ldr r0,[r3]; bx lr
//! 8007504: subs r3, r0, r5
//! 8007506: cmp  r3, r4
//! 8007508: bcc  80074fc
//! ```
//!
//! Until the SysTick handler increments `uwTick`, every iteration starts and
//! ends in the same state. The interpreter still ran each one: at 250 MHz,
//! 250 M instructions per simulated second of `delay()`.
//!
//! **The proof.** At a loop head (a backward branch target the core has just
//! jumped to again), one iteration is run from the current state on a
//! read-only view of plain memory ([`ProbeBus`]). It is refused when:
//! - an exception is pending or active state changes, or the core is asleep,
//!   in an IT block, or has a fault or lockup latched;
//! - any access is not a read of plain RAM/flash (stores and MMIO are
//!   refused: a store would change memory, and a device read can change or
//!   have side effects);
//! - it does not come back to the head within [`MAX_PERIOD`] instructions;
//! - the architectural state it comes back with differs from the start.
//!
//! Then the iteration is a fixed point: nothing it reads changes until
//! something outside the core writes memory or raises an interrupt. The
//! machine's skip is bounded by the next CPU-interrupt, scheduler, motor and
//! resident-edge deadline, and refused while a legacy per-cycle peripheral
//! walk runs, so nothing does within the skip. The machine skips a whole
//! number of iterations; the core resumes at the head in the same state, and
//! the interrupt lands on the same instruction as it would without the skip.

use super::{AccessWidth, CortexM};
use crate::{Bus, DmaRequest, SimResult, SimulationConfig, SimulationError};

/// Longest iteration, in instructions, the probe follows.
const MAX_PERIOD: u64 = 256;
/// Backward jumps longer than this are not considered loop back-edges.
const MAX_BACKEDGE_BYTES: u32 = 0x400;
/// Head arrivals to sit out after the first refused probe at that head;
/// doubles on each further refusal there, up to [`MAX_BACKOFF`]. Small,
/// because the commonest refusal is the first iteration after an interrupt
/// (it reloads the tick the handler just changed) and the next one proves.
const REFUSE_BACKOFF: u32 = 2;
const MAX_BACKOFF: u32 = 1 << 16;
/// Fewest cycles worth a probe, and the arrivals to wait when the window to
/// the next deadline is shorter.
const MIN_SKIP: u64 = 256;
const SHORT_BACKOFF: u32 = 16;

/// What the core tracks between instructions to find a loop head.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PollDetector {
    /// Recent backward-jump targets, most recent last.
    recent: [u32; 4],
    /// Set while the current PC is a candidate head.
    anchor: Option<u32>,
    /// Recently refused heads: `(head, arrivals left to ignore it, backoff
    /// it got)`. A loop often has two heads (its own back-edge and a call's
    /// return), so one slot per head.
    refused: [(u32, u32, u32); 4],
    /// The last proven loop, as `(head, period)`.
    plan: Option<(u32, u64)>,
}

/// Everything an iteration could change in the core, compared before and
/// after the probe.
#[derive(Clone, PartialEq, Debug)]
struct ArchState {
    regs: [u32; 17],
    msp: u32,
    psp: u32,
    control: u32,
    xpsr: u32,
    pending: [u64; 4],
    primask: bool,
    faultmask: bool,
    basepri: u8,
    it_state: u8,
    active_exception: u32,
    fpu_s: [u32; 32],
    fpscr: u32,
    sleeping: bool,
    waiting_for_event: bool,
    event_register: bool,
    exclusive: Option<(u32, AccessWidth, u32)>,
    firmware_exit: Option<u32>,
    it_state_restored: bool,
}

impl ArchState {
    fn capture(c: &CortexM) -> Self {
        Self {
            regs: [
                c.r0, c.r1, c.r2, c.r3, c.r4, c.r5, c.r6, c.r7, c.r8, c.r9, c.r10, c.r11, c.r12,
                c.sp, c.lr, c.pc, 0,
            ],
            msp: c.msp,
            psp: c.psp,
            control: c.control,
            xpsr: c.xpsr,
            pending: c.pending_exceptions,
            primask: c.primask,
            faultmask: c.faultmask,
            basepri: c.basepri,
            it_state: c.it_state,
            active_exception: c.active_exception,
            fpu_s: c.fpu_s,
            fpscr: c.fpscr,
            sleeping: c.sleeping,
            waiting_for_event: c.waiting_for_event,
            event_register: c.event_register,
            exclusive: c.exclusive_subword,
            firmware_exit: c.firmware_exit,
            it_state_restored: c.it_state_restored,
        }
    }

    fn restore(&self, c: &mut CortexM) {
        [
            c.r0, c.r1, c.r2, c.r3, c.r4, c.r5, c.r6, c.r7, c.r8, c.r9, c.r10, c.r11, c.r12, c.sp,
            c.lr, c.pc,
        ] = [
            self.regs[0],
            self.regs[1],
            self.regs[2],
            self.regs[3],
            self.regs[4],
            self.regs[5],
            self.regs[6],
            self.regs[7],
            self.regs[8],
            self.regs[9],
            self.regs[10],
            self.regs[11],
            self.regs[12],
            self.regs[13],
            self.regs[14],
            self.regs[15],
        ];
        c.msp = self.msp;
        c.psp = self.psp;
        c.control = self.control;
        c.xpsr = self.xpsr;
        c.pending_exceptions = self.pending;
        c.primask = self.primask;
        c.faultmask = self.faultmask;
        c.basepri = self.basepri;
        c.it_state = self.it_state;
        c.active_exception = self.active_exception;
        c.fpu_s = self.fpu_s;
        c.fpscr = self.fpscr;
        c.sleeping = self.sleeping;
        c.waiting_for_event = self.waiting_for_event;
        c.event_register = self.event_register;
        c.exclusive_subword = self.exclusive;
        c.firmware_exit = self.firmware_exit;
        c.it_state_restored = self.it_state_restored;
    }
}

/// A read-only view of plain memory: reads are served from the slices
/// [`Bus::plain_memory_slice`] exposes (RAM and flash); every other access,
/// and every write, is refused.
struct ProbeBus<'a> {
    inner: &'a dyn Bus,
}

impl ProbeBus<'_> {
    fn plain_read(&self, addr: u64, size: usize) -> SimResult<u32> {
        let (start, end, slice) = self
            .inner
            .plain_memory_slice(addr)
            .ok_or(SimulationError::MemoryViolation(addr))?;
        if addr < start || addr + size as u64 > end {
            return Err(SimulationError::MemoryViolation(addr));
        }
        let off = (addr - start) as usize;
        let bytes = slice
            .get(off..off + size)
            .ok_or(SimulationError::MemoryViolation(addr))?;
        Ok(bytes
            .iter()
            .enumerate()
            .fold(0u32, |v, (i, b)| v | (u32::from(*b) << (8 * i))))
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
    fn fetch_slice(&self, pc: u64) -> Option<(u64, u64, &[u8])> {
        self.inner.fetch_slice(pc)
    }
    fn plain_memory_slice(&self, addr: u64) -> Option<(u64, u64, &[u8])> {
        self.inner.plain_memory_slice(addr)
    }
}

impl CortexM {
    /// Called after every retired instruction of a batch with the PC it
    /// started at. Marks the current PC as a candidate loop head when the
    /// core has just jumped back to a recent backward-jump target.
    #[inline]
    pub(super) fn note_poll_step(&mut self, pc_before: u32) {
        let pc = self.pc;
        // `pc == pc_before` is a branch to itself (`b .`), the tightest loop.
        if pc > pc_before || pc_before - pc > MAX_BACKEDGE_BYTES {
            return;
        }
        self.poll_loop_note_backedge(pc);
    }

    #[cold]
    fn poll_loop_note_backedge(&mut self, pc: u32) {
        let d = &mut self.poll;
        if let Some(slot) = d.refused.iter_mut().find(|(h, _, _)| *h == pc) {
            if slot.1 > 0 {
                slot.1 -= 1;
                return;
            }
        }
        if d.recent.contains(&pc) {
            d.anchor = Some(pc);
        } else {
            d.recent.rotate_left(1);
            d.recent[3] = pc;
        }
    }

    /// Called when a batch starts. An anchor still set here was not taken:
    /// the machine could not skip on this bus right now (a legacy
    /// per-cycle walk, a cycle-accurate device, ...). Ending every batch at
    /// that head would cost far more than it can save, so back off from it.
    #[inline]
    pub(super) fn poll_batch_start(&mut self) {
        if let Some(head) = self.poll.anchor.take() {
            self.refuse(head, true);
        }
    }

    pub(super) fn poll_anchor_ready(&self) -> bool {
        self.poll.anchor == Some(self.pc)
    }

    /// See [`crate::Cpu::poll_loop_skip_budget`].
    pub(super) fn poll_skip_budget(&mut self, bus: &dyn Bus, limit: u64) -> Option<(u64, u64)> {
        let head = self.poll.anchor.take()?;
        if head != self.pc {
            return None;
        }
        // Too short a window for a skip to pay for the probe: wait a few
        // arrivals before asking again.
        if limit < MIN_SKIP {
            self.refuse(head, false);
            return None;
        }
        let period = match self.probe_fixed_point(bus, head) {
            Some(p) => p,
            None => {
                self.refuse(head, true);
                return None;
            }
        };
        let cycles = (limit / period) * period;
        if cycles == 0 {
            self.refuse(head, false);
            return None;
        }
        // Proven: forget any backoff this head had.
        if let Some(slot) = self.poll.refused.iter_mut().find(|(h, _, _)| *h == head) {
            *slot = (0, 0, 0);
        }
        self.poll.plan = Some((head, period));
        Some((period, cycles))
    }

    /// Ignore `head` for a while. A failed proof backs off exponentially (a
    /// head that is not a fixed point is probed ever more rarely); a window
    /// too short to skip waits [`SHORT_BACKOFF`] arrivals.
    fn refuse(&mut self, head: u32, proof_failed: bool) {
        let slots = &mut self.poll.refused;
        let i = slots
            .iter()
            .position(|(h, _, _)| *h == head)
            .unwrap_or_else(|| {
                // Evict the slot closest to expiring (an exhausted one first).
                (0..slots.len())
                    .min_by_key(|&i| (slots[i].1, slots[i].2))
                    .unwrap()
            });
        let previous = if slots[i].0 == head { slots[i].2 } else { 0 };
        let backoff = if proof_failed {
            if previous >= REFUSE_BACKOFF {
                (previous * 2).min(MAX_BACKOFF)
            } else {
                REFUSE_BACKOFF
            }
        } else {
            previous.max(SHORT_BACKOFF)
        };
        slots[i] = (
            head,
            if proof_failed { backoff } else { SHORT_BACKOFF },
            backoff,
        );
    }

    /// See [`crate::Cpu::fast_forward_poll_loop`]. The loop is a fixed point,
    /// so whole iterations leave the core exactly as it is.
    pub(super) fn poll_fast_forward(&mut self, cycles: u64) -> bool {
        matches!(
            self.poll.plan.take(),
            Some((head, period)) if head == self.pc && cycles % period == 0
        )
    }

    /// Run one iteration from `head` on a read-only view of plain memory and
    /// return its length when it comes back to `head` in the state it left.
    fn probe_fixed_point(&mut self, bus: &dyn Bus, head: u32) -> Option<u64> {
        if self.any_exception_pending()
            || self.sleeping
            || self.waiting_for_event
            || self.it_state != 0
            || self.fault_entry.is_some()
            || self.lockup.is_some()
            || self.pending_data_fault.is_some()
            || self.pending_mem_manage.is_some()
            || self.pending_undef_instruction
            || self.pending_div0
            || self.debug_batch_break()
        {
            return None;
        }
        let before = ArchState::capture(self);
        let mut probe = ProbeBus { inner: bus };
        let config = bus.config().clone();
        let mut period = None;
        for n in 1..=MAX_PERIOD {
            // `step_execute`, not `step_internal`: a refused access must end
            // the probe, never escalate into a fault (which would write the
            // SCB fault registers the bus shares).
            if self.step_execute(&mut probe, &[], &config).is_err()
                || self.any_exception_pending()
                || self.active_exception != before.active_exception
                || self.fault_entry.is_some()
                || self.lockup.is_some()
                || self.pending_data_fault.is_some()
                || self.pending_mem_manage.is_some()
                || self.pending_undef_instruction
                || self.pending_div0
            {
                break;
            }
            if self.pc == head {
                period = Some(n);
                break;
            }
        }
        let same = ArchState::capture(self) == before;
        before.restore(self);
        // A refused probe may have latched a fault or a pending flag on the
        // way out; none of that happened, so clear it.
        self.fault_entry = None;
        self.lockup = None;
        self.pending_data_fault = None;
        self.pending_mem_manage = None;
        self.pending_undef_instruction = false;
        self.pending_div0 = false;
        period.filter(|_| same)
    }
}
