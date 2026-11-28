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
//! - any load is not a read of plain RAM/flash (a device read can change or
//!   have side effects);
//! - any store is not a bare write to plain RAM nothing watches
//!   ([`Bus::plain_ram_store_unobserved`]): MMIO, flash, a debug watch, a
//!   memory observer, a memory policy or an armed batch stop;
//! - it does not come back to the head within [`MAX_PERIOD`] instructions;
//! - the architectural state it comes back with differs from the start, or
//!   a byte it stored differs from what memory held before it.
//!
//! Stores go to an overlay the probe's own loads read through, never to
//! memory. RTOS idle loops (mbed's `rtos_idle_loop` -> `default_idle_hook`)
//! call functions and enter and leave a critical section on every iteration:
//! they push the same return addresses to the same stack slots and set a
//! nesting counter to 1 and back to 0. Each iteration rewrites exactly the
//! bytes memory already holds, so it is a fixed point of registers and
//! memory together. A loop whose stores leave memory different (an idle
//! counter, any affine or other progression) is refused.
//!
//! Then the iteration is a fixed point: nothing it reads changes until
//! something outside the core writes memory or raises an interrupt. Its
//! intermediate stores are seen by nobody: every other memory reader (a
//! peripheral event, DMA, the other core) runs only at the window's ends,
//! where memory holds what it held at the start; the machine refuses the
//! skip while another core runs beside a loop that stores. The
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
    /// The last proven loop, as `(head, period, stores)`; `stores` is true
    /// when an iteration writes memory (with the values it already holds).
    plan: Option<(u32, u64, bool)>,
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
    pending: [u64; 8],
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

/// A side-effect-free view of plain memory: reads are served from the
/// slices [`Bus::plain_memory_slice`] exposes (RAM and flash), through the
/// overlay of bytes stored so far; stores to unobserved plain RAM go to that
/// overlay when allowed. Every other access is refused.
struct ProbeBus<'a> {
    inner: &'a dyn Bus,
    /// When set, every served load address is appended (diagnosis only).
    loads: Option<&'a std::cell::RefCell<Vec<u64>>>,
    /// Bytes stored so far, in store order, as `(address, byte)`. `None`:
    /// every store is refused.
    stores: Option<Vec<(u64, u8)>>,
}

/// Most bytes one probed iteration may store.
const MAX_STORED_BYTES: usize = 256;

/// The diagnosis probe also serves loads below this address through the bus
/// (the boot alias of flash is not a plain slice on every chip): on Cortex-M
/// it is always the code region, never a device, and it is how a loop
/// reading through a NULL pointer is recognised.
const NULL_PAGE: u64 = 0x400;

impl ProbeBus<'_> {
    fn plain_read(&self, addr: u64, size: usize) -> SimResult<u32> {
        let Some((start, end, slice)) = self.inner.plain_memory_slice(addr) else {
            return match self.loads {
                Some(loads) if addr + size as u64 <= NULL_PAGE => {
                    let v = match size {
                        1 => u32::from(self.inner.read_u8(addr)?),
                        2 => u32::from(self.inner.read_u16(addr)?),
                        _ => self.inner.read_u32(addr)?,
                    };
                    loads.borrow_mut().push(addr);
                    Ok(v)
                }
                _ => Err(SimulationError::MemoryViolation(addr)),
            };
        };
        if addr < start || addr + size as u64 > end {
            return Err(SimulationError::MemoryViolation(addr));
        }
        let off = (addr - start) as usize;
        let bytes = slice
            .get(off..off + size)
            .ok_or(SimulationError::MemoryViolation(addr))?;
        if let Some(loads) = self.loads {
            loads.borrow_mut().push(addr);
        }
        let mut v = bytes
            .iter()
            .enumerate()
            .fold(0u32, |v, (i, b)| v | (u32::from(*b) << (8 * i)));
        // Later stores win: apply them in order.
        for &(a, b) in self.stores.iter().flatten() {
            if a >= addr && a < addr + size as u64 {
                let shift = 8 * (a - addr) as u32;
                v = (v & !(0xff << shift)) | (u32::from(b) << shift);
            }
        }
        Ok(v)
    }

    fn store(&mut self, addr: u64, size: usize, value: u32) -> SimResult<()> {
        let refuse = Err(SimulationError::MemoryViolation(addr));
        let Some(stores) = self.stores.as_ref() else {
            return refuse;
        };
        if stores.len() + size > MAX_STORED_BYTES
            || !self.inner.plain_ram_store_unobserved(addr, size as u64)
        {
            return refuse;
        }
        // The overlay is read back through the plain slice: it must cover
        // the stored bytes.
        match self.inner.plain_memory_slice(addr) {
            Some((start, end, _)) if addr >= start && addr + size as u64 <= end => {}
            _ => return refuse,
        }
        let stores = self.stores.as_mut().expect("checked above");
        for i in 0..size {
            stores.push((addr + i as u64, (value >> (8 * i)) as u8));
        }
        Ok(())
    }

    /// True when every byte stored ends holding what memory holds: the
    /// iteration left memory as it found it.
    fn stores_rewrite_memory(&self) -> bool {
        let Some(stores) = self.stores.as_ref() else {
            return true;
        };
        stores.iter().enumerate().all(|(i, &(addr, _))| {
            // Only the last store to a byte decides what it ends holding.
            if stores[i + 1..].iter().any(|&(a, _)| a == addr) {
                return true;
            }
            let last = stores[i].1;
            self.inner
                .plain_memory_slice(addr)
                .and_then(|(start, _, slice)| slice.get((addr - start) as usize).copied())
                == Some(last)
        })
    }

    fn stored_any(&self) -> bool {
        self.stores.as_ref().is_some_and(|s| !s.is_empty())
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
    fn write_u8(&mut self, addr: u64, value: u8) -> SimResult<()> {
        self.store(addr, 1, u32::from(value))
    }
    fn write_u16(&mut self, addr: u64, value: u16) -> SimResult<()> {
        self.store(addr, 2, u32::from(value))
    }
    fn write_u32(&mut self, addr: u64, value: u32) -> SimResult<()> {
        self.store(addr, 4, value)
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
        let (period, stores) = match self.probe_fixed_point(bus, head) {
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
        self.poll.plan = Some((head, period, stores));
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
    pub(super) fn poll_fast_forward(&mut self, cycles: u64) -> Option<u64> {
        matches!(
            self.poll.plan.take(),
            Some((head, period, _)) if head == self.pc && cycles % period == 0
        )
        .then_some(cycles)
    }

    /// See [`crate::Cpu::poll_loop_plan_stores`].
    pub(super) fn poll_plan_stores(&self) -> bool {
        self.poll.plan.is_some_and(|(_, _, stores)| stores)
    }

    /// The skip's proof: [`Self::probe_iteration`] with stores allowed,
    /// refused outright while an exception is pending. Returns the period and
    /// whether an iteration stores.
    fn probe_fixed_point(&mut self, bus: &dyn Bus, head: u32) -> Option<(u64, bool)> {
        if self.any_exception_pending() {
            return None;
        }
        self.probe_iteration(bus, head, None, true)
    }

    /// See [`crate::Cpu::stalled_loop_loads`]. The same proof as the skip,
    /// run from the current PC, except that a pending interrupt does not
    /// refuse it: the run is over, and the question is only what the loop
    /// the core is parked in reads.
    pub(super) fn stalled_loads(&mut self, bus: &dyn Bus) -> Option<Vec<u64>> {
        let loads = std::cell::RefCell::new(Vec::new());
        let head = self.pc;
        // Diagnosis keeps its read-only proof: a stall report names the
        // words a loop waits on, and a loop that stores is not waiting.
        let (_, stored) = self.probe_iteration(bus, head, Some(&loads), false)?;
        debug_assert!(!stored);
        Some(loads.into_inner())
    }

    /// Run one iteration from `head` on a side-effect-free view of plain
    /// memory and return its length, and whether it stored, when it comes
    /// back to `head` with the core and memory in the state it left them.
    /// Stores are allowed (into an overlay) unless `loads` is set
    /// (diagnosis). `strict` also refuses when an exception becomes pending
    /// on the way.
    fn probe_iteration(
        &mut self,
        bus: &dyn Bus,
        head: u32,
        loads: Option<&std::cell::RefCell<Vec<u64>>>,
        strict: bool,
    ) -> Option<(u64, bool)> {
        if self.sleeping
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
        let mut probe = ProbeBus {
            inner: bus,
            loads,
            stores: loads.is_none().then(Vec::new),
        };
        let config = bus.config();
        let mut period = None;
        for n in 1..=MAX_PERIOD {
            // `step_execute`, not `step_internal`: a refused access must end
            // the probe, never escalate into a fault (which would write the
            // SCB fault registers the bus shares).
            if self.step_execute(&mut probe, &[], config).is_err()
                || (strict && self.any_exception_pending())
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
        let same =
            period.is_some() && ArchState::capture(self) == before && probe.stores_rewrite_memory();
        let stored = probe.stored_any();
        before.restore(self);
        // A refused probe may have latched a fault or a pending flag on the
        // way out; none of that happened, so clear it.
        self.fault_entry = None;
        self.lockup = None;
        self.pending_data_fault = None;
        self.pending_mem_manage = None;
        self.pending_undef_instruction = false;
        self.pending_div0 = false;
        period.filter(|_| same).map(|p| (p, stored))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::SystemBus;
    use crate::cpu::cortex_m::DecodeCacheEntry;
    use crate::decoder::arm::decode_thumb_16;
    use crate::{Cpu, SimulationConfig};

    fn cpu_with(code: &[u16]) -> (CortexM, SystemBus) {
        let mut cpu = CortexM::new();
        let mut bus = SystemBus::new();
        cpu.pc = 0x100;
        cpu.sp = 0x2000_0400;
        cpu.r1 = 0x2000_0100;
        for (i, &op) in code.iter().enumerate() {
            let pc = 0x100 + i as u32 * 2;
            assert!(bus.flash.write_u16(u64::from(pc), op));
            cpu.insert_decoded_entry(DecodeCacheEntry {
                tag: pc,
                instruction: decode_thumb_16(op),
                opcode: u32::from(op),
                pc_increment: 2,
                cycles: 1,
            });
        }
        (cpu, bus)
    }

    /// Thumb `b` from the `k`-th halfword back to the head at 0x100.
    fn b_head(k: u16) -> u16 {
        0xe000 | ((0u16.wrapping_sub(k + 2)) & 0x7ff)
    }

    const RAM: u32 = 0x2000_0100;
    const MMIO: u32 = 0x4000_C000;

    fn snapshot(cpu: &CortexM) -> serde_json::Value {
        serde_json::to_value(cpu.snapshot()).unwrap()
    }

    fn ram_word(bus: &SystemBus, addr: u32) -> u32 {
        bus.ram.read_u32(u64::from(addr)).unwrap()
    }

    /// Probe in skip mode; asserts the probe changed neither core nor memory.
    fn probe(cpu: &mut CortexM, bus: &SystemBus) -> Option<(u64, bool)> {
        let core = snapshot(cpu);
        let mem = bus.ram.data.clone();
        let got = cpu.probe_iteration(bus, 0x100, None, true);
        assert_eq!(snapshot(cpu), core, "the probe must restore the core");
        assert!(bus.ram.data == mem, "the probe must never write memory");
        got
    }

    /// Run one real iteration (`n` instructions) on the bus.
    fn run(cpu: &mut CortexM, bus: &mut SystemBus, n: u64) {
        let config = SimulationConfig::default();
        for _ in 0..n {
            cpu.step(bus, &[], &config).unwrap();
        }
        assert_eq!(cpu.pc, 0x100);
    }

    #[test]
    fn accepts_a_read_only_poll_loop() {
        // ldr r0,[r1]; b head
        let (mut cpu, bus) = cpu_with(&[0x6808, b_head(1)]);
        assert_eq!(probe(&mut cpu, &bus), Some((2, false)));
    }

    #[test]
    fn accepts_a_loop_whose_stores_rewrite_memory() {
        // The mbed idle loop's shape: enter a critical section (nesting
        // counter 0 -> 1, PRIMASK set), leave it (1 -> 0, PRIMASK clear).
        // cpsid i; ldr r0,[r1]; adds r0,#1; str r0,[r1];
        // subs r0,#1; str r0,[r1]; cpsie i; b head
        let code = [
            0xb672,
            0x6808,
            0x3001,
            0x6008,
            0x3801,
            0x6008,
            0xb662,
            b_head(7),
        ];
        let (mut cpu, mut bus) = cpu_with(&code);
        // `subs` leaves Z and C set: the first iteration changes xPSR.
        assert_eq!(probe(&mut cpu, &bus), None);
        run(&mut cpu, &mut bus, 8);
        assert_eq!(probe(&mut cpu, &bus), Some((8, true)));
        // N real iterations leave the core where it started.
        let before = snapshot(&cpu);
        run(&mut cpu, &mut bus, 8 * 3);
        assert_eq!(snapshot(&cpu), before);
        assert_eq!(ram_word(&bus, RAM), 0);
    }

    #[test]
    fn accepts_a_call_loop_once_its_stack_slots_hold_what_it_pushes() {
        // push {r4, lr}; pop {r4, pc} with lr = head: the stack slots below
        // SP get r4 and lr. The first iteration changes them (refused); from
        // the second on, every push rewrites what is there.
        let (mut cpu, mut bus) = cpu_with(&[0xb510, 0xbd10]);
        cpu.lr = 0x101;
        cpu.r4 = 0x1234_5678;
        assert_eq!(probe(&mut cpu, &bus), None);
        run(&mut cpu, &mut bus, 2);
        assert_eq!(ram_word(&bus, 0x2000_03f8), 0x1234_5678);
        assert_eq!(probe(&mut cpu, &bus), Some((2, true)));
    }

    #[test]
    fn refuses_mmio_reads() {
        // ldr r0,[r2]; b head, r2 in a peripheral window
        let (mut cpu, bus) = cpu_with(&[0x6810, b_head(1)]);
        cpu.r2 = MMIO;
        assert_eq!(probe(&mut cpu, &bus), None);
    }

    #[test]
    fn refuses_observable_stores() {
        // str r0,[r2]; b head: to a peripheral, to flash, and to RAM that
        // memory already holds but a debug watch or an observer sees.
        for (addr, watch, observe) in [
            (MMIO, false, false),
            (0x200, false, false),
            (RAM, true, false),
            (RAM, false, true),
            (RAM, false, false),
        ] {
            let (mut cpu, mut bus) = cpu_with(&[0x6010, b_head(1)]);
            cpu.r2 = addr;
            if watch {
                assert!(bus.add_debug_watch(u64::from(RAM), 4, false, true));
            }
            if observe {
                bus.add_observer(std::sync::Arc::new(
                    crate::metrics::PerformanceMetrics::new(),
                ));
            }
            let plain = addr == RAM && !watch && !observe;
            let want = plain.then_some((2, true));
            assert_eq!(probe(&mut cpu, &bus), want, "{addr:#x} {watch} {observe}");
        }
    }

    #[test]
    fn refuses_a_store_the_batch_stop_watches() {
        let (mut cpu, bus) = cpu_with(&[0x6008, b_head(1)]);
        bus.batch_stop.arm(&[(u64::from(RAM), u64::from(RAM) + 4)]);
        assert_eq!(probe(&mut cpu, &bus), None);
    }

    #[test]
    fn refuses_counters() {
        // adds r0,#1; b head: a register counter.
        let (mut cpu, bus) = cpu_with(&[0x3001, b_head(1)]);
        assert_eq!(probe(&mut cpu, &bus), None);
        // ldr r0,[r1]; adds r0,#1; str r0,[r1]; movs r0,#0; b head: an idle
        // counter in memory, registers back where they were.
        let (mut cpu, bus) = cpu_with(&[0x6808, 0x3001, 0x6008, 0x2000, b_head(4)]);
        assert_eq!(probe(&mut cpu, &bus), None);
        // A store that toggles a flag: each iteration flips it.
        // ldr r0,[r1]; movs r2,#1; eors r0,r2; str r0,[r1]; movs r0,#0;
        // movs r2,#0; b head
        let (mut cpu, bus) = cpu_with(&[0x6808, 0x2201, 0x4050, 0x6008, 0x2000, 0x2200, b_head(6)]);
        assert_eq!(probe(&mut cpu, &bus), None);
    }

    #[test]
    fn diagnosis_still_refuses_stores() {
        let (mut cpu, bus) = cpu_with(&[0x6008, b_head(1)]);
        assert_eq!(probe(&mut cpu, &bus), Some((2, true)));
        assert_eq!(cpu.stalled_loads(&bus), None);
    }

    #[test]
    fn skip_plan_records_stores_and_fast_forward_applies_it() {
        let code = [
            0xb672,
            0x6808,
            0x3001,
            0x6008,
            0x3801,
            0x6008,
            0xb662,
            b_head(7),
        ];
        let (mut cpu, mut bus) = cpu_with(&code);
        run(&mut cpu, &mut bus, 8);
        cpu.poll.anchor = Some(0x100);
        assert_eq!(cpu.poll_skip_budget(&bus, 1000), Some((8, 1000 / 8 * 8)));
        assert!(cpu.poll_plan_stores());
        assert_eq!(cpu.poll_fast_forward(800), Some(800));
        assert!(!cpu.poll_plan_stores());
    }

    #[test]
    fn mbed_idle_loop_fast_forward_preserves_reported_instruction_count() {
        // cpsid i; ldr r0,[r1]; adds r0,#1; str r0,[r1];
        // subs r0,#1; str r0,[r1]; cpsie i; b head
        let code = [
            0xb672,
            0x6808,
            0x3001,
            0x6008,
            0x3801,
            0x6008,
            0xb662,
            b_head(7),
        ];
        let (mut fast_cpu, mut fast_bus) = cpu_with(&code);
        run(&mut fast_cpu, &mut fast_bus, 8);
        fast_cpu.poll.anchor = Some(0x100);
        fast_bus.peripherals.clear();
        fast_bus.rebuild_peripheral_ranges();
        let mut fast = crate::Machine::new(fast_cpu, fast_bus);
        fast.config.idle_fast_forward_enabled = true;

        let (mut reference_cpu, mut reference_bus) = cpu_with(&code);
        run(&mut reference_cpu, &mut reference_bus, 8);
        reference_cpu.poll.anchor = Some(0x100);
        reference_bus.peripherals.clear();
        reference_bus.rebuild_peripheral_ranges();
        let mut reference = crate::Machine::new(reference_cpu, reference_bus);
        reference.config.idle_fast_forward_enabled = false;

        let fast_report = fast.advance(crate::AdvanceRequest::run(Some(256))).unwrap();
        let reference_report = reference
            .advance(crate::AdvanceRequest::run(Some(256)))
            .unwrap();

        assert_eq!(fast_report.primary_steps, reference_report.primary_steps);
        assert_eq!(fast_report.fuel_consumed, reference_report.fuel_consumed);
        assert_eq!(fast_report.elapsed_cycles, reference_report.elapsed_cycles);
        assert_eq!(snapshot(&fast.cpu), snapshot(&reference.cpu));
        assert_eq!(fast.bus.ram.data, reference.bus.ram.data);
        assert_eq!(
            fast.step_profile().cpu_instructions,
            reference.step_profile().cpu_instructions
        );
        if crate::cycle_clock::event_scheduler_enabled() {
            assert!(
                fast_report.idle_cycles > 0,
                "the mbed loop should fast-forward"
            );
        } else {
            assert_eq!(fast_report.idle_cycles, 0);
        }
    }
}
