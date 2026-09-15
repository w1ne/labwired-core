// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! [`JitHost`] adapter for a `Machine<CortexM>`.
//!
//! The universal dispatch loop drives the guest **entirely** through the
//! [`JitHost`] trait, never touching a concrete `Cpu`/`Bus`. This module is
//! the Cortex-M binding: it wraps `&mut Machine<CortexM>` and implements
//! every hook — interpret one instruction, read the current PC, resume at a
//! PC, hand out a flash code view, report the safety gate, and flatten
//! architectural state into a [`StateVec`] for the differential harness.
//!
//! ## Two places the framework API did not match the Cortex-M machine
//!
//! * **`code_bytes` fetches through the bus, not raw `flash.data`.** The JIT
//!   must compile from the exact bytes the interpreter fetches. Cortex-M
//!   fetch is `bus.read_u16(pc)` (and a second halfword for Thumb-2);
//!   [`SystemBus::read_code_slice`](crate::bus::SystemBus::read_code_slice)
//!   is the side-effect-free equivalent used by the RISC-V host. Using it
//!   here keeps both ISAs on the same fetch path.
//! * **There is no flash-dirty flag on the bus.** Nothing tracks flash
//!   writes today, so [`take_flash_dirty`](JitHost::take_flash_dirty) is
//!   conservatively `false`. That is correct for any run that does not
//!   self-modify flash (the overwhelming common case, and every current
//!   test); when flash self-write support lands it wires in here.

use crate::cpu::CortexM;
use crate::Machine;

use super::super::fallback::{HostStep, JitHost, SafetyGate};
use super::super::{Pc, StateVec};

/// Number of `u32` words in a Cortex-M [`StateVec`]: `r0..r12` (13) +
/// `sp` + `lr` + `pc` + `xpsr`.
pub const STATE_VEC_LEN: usize = 13 + 4;

/// Flatten a [`CortexM`] into the differential-harness [`StateVec`].
///
/// Layout (the architectural GPR file plus the status word the ISA needs
/// for equivalence):
///
/// | index | contents |
/// | ----- | -------- |
/// | `0..13` | `r0..r12` |
/// | `13` | `sp` (r13) |
/// | `14` | `lr` (r14) |
/// | `15` | `pc` (r15, Thumb bit already stripped) |
/// | `16` | `xpsr` |
///
/// Banked `msp`/`psp`, `CONTROL`, `PRIMASK`, and the NVIC pending mask are
/// deliberately excluded: the all-bail lockstep loop never takes an
/// exception or switches stacks, and the RISC-V host similarly omits
/// volatile/implementation words. Later codegen chunks that retire
/// exception-entry blocks add those words here (or to
/// [`super::differential_cycle_ignore_indices`]).
pub fn snapshot_state(cpu: &CortexM) -> StateVec {
    let mut v = Vec::with_capacity(STATE_VEC_LEN);
    v.push(cpu.r0);
    v.push(cpu.r1);
    v.push(cpu.r2);
    v.push(cpu.r3);
    v.push(cpu.r4);
    v.push(cpu.r5);
    v.push(cpu.r6);
    v.push(cpu.r7);
    v.push(cpu.r8);
    v.push(cpu.r9);
    v.push(cpu.r10);
    v.push(cpu.r11);
    v.push(cpu.r12);
    v.push(cpu.sp);
    v.push(cpu.lr);
    v.push(cpu.pc);
    v.push(cpu.xpsr);
    debug_assert_eq!(v.len(), STATE_VEC_LEN);
    v
}

/// A [`JitHost`] view over a Cortex-M machine, borrowing it for the
/// lifetime of one dispatch run.
pub struct CortexMJitHost<'m> {
    machine: &'m mut Machine<CortexM>,
}

impl<'m> CortexMJitHost<'m> {
    /// Wrap a machine for JIT dispatch.
    pub fn new(machine: &'m mut Machine<CortexM>) -> Self {
        Self { machine }
    }

    /// Borrow the underlying machine (telemetry / tests).
    pub fn machine(&self) -> &Machine<CortexM> {
        self.machine
    }
}

impl JitHost for CortexMJitHost<'_> {
    fn pc(&self) -> Pc {
        self.machine.cpu.pc as Pc
    }

    fn interpret_one(&mut self) -> HostStep {
        match self.machine.step() {
            Ok(()) => HostStep::Advanced,
            // A stopping condition (trap the interpreter cannot service,
            // decode error, halt): the dispatch loop returns. The
            // interpreter remains the single source of truth for *why*.
            Err(_) => HostStep::Halted,
        }
    }

    fn resume_at(&mut self, pc: Pc) {
        // Cortex-M stores PC with the Thumb bit stripped (`set_pc` masks
        // bit 0); the dispatch loop hands a guest address, not an EXC_RETURN.
        self.machine.cpu.pc = (pc as u32) & !1;
    }

    fn code_bytes(&self, pc: Pc) -> Option<Vec<u8>> {
        // Materialise up to one max-length block through the SAME fetch path
        // the interpreter uses (`bus.read_code_slice`), so a Thumb-2 32-bit
        // instruction that straddles a view boundary is seen as the CPU
        // would see it. `None`/too-short when `pc` is not fetchable code —
        // the PC stays on the interpreter.
        let bytes = self
            .machine
            .bus
            .read_code_slice(pc, super::MAX_BLOCK_INSTRS as usize * 4);
        (bytes.len() >= 2).then_some(bytes)
    }

    fn safety(&self) -> SafetyGate {
        SafetyGate {
            // Any per-instruction observer forces interpretation.
            observers_active: !self.machine.observers.is_empty(),
            // A block could step over a breakpoint address without stopping.
            breakpoints_active: !self.machine.breakpoints.is_empty(),
            // A logic-analyzer / DAP-watch tap (poll or push mode) needs
            // per-cycle pad visibility a batched block elides.
            probes_active: self.machine.logic_probes_active(),
            // A cycle-accurate peripheral (HC-SR04, IO-Link, op-modeled FLASH)
            // needs per-instruction bus services the JIT does not run.
            cycle_accurate: self.machine.bus.requires_cycle_accurate(),
        }
    }

    fn snapshot_state(&self) -> StateVec {
        snapshot_state(&self.machine.cpu)
    }

    fn take_flash_dirty(&mut self) -> bool {
        // No flash-write tracking on the bus (see module docs). Correct for
        // any run that does not self-modify flash.
        false
    }
}
