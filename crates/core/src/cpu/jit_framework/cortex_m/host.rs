// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! [`JitHost`] adapter and register marshalling for `Machine<CortexM>`.
//!
//! ## Flash invalidation
//!
//! * **There is no flash-dirty flag on the bus.** Nothing tracks flash
//!   writes today, so [`take_flash_dirty`](JitHost::take_flash_dirty) is
//!   conservatively `false`. That is correct for any run that does not
//!   self-modify flash (the overwhelming common case, and every current
//!   test); when flash self-write support lands it wires in here.

use crate::cpu::CortexM;
use crate::Machine;

use super::super::fallback::{HostStep, JitHost, SafetyGate};
use super::super::{Pc, StateVec};

/// Words in a Cortex-M [`StateVec`].
pub const STATE_VEC_LEN: usize = 30 + 32 + 1;

/// Flatten a [`CortexM`] into the differential-harness [`StateVec`].
///
/// | index | contents |
/// | ----- | -------- |
/// | `0..13` | `r0..r12` |
/// | `13` | `sp` |
/// | `14` | `lr` |
/// | `15` | `pc` |
/// | `16` | `xpsr` |
/// | `17` | `primask` as 0/1 |
/// | `18` | `faultmask` as 0/1 |
/// | `19` | `basepri` |
/// | `20` | `it_state` |
/// | `21` | `active_exception` |
/// | `22..30` | `pending_exceptions[0..4]` as lo/hi `u32` pairs |
/// | `30..62` | `fpu_s[0..32]` (raw IEEE-754 bits) |
/// | `62` | `fpscr` |
///
/// `fpu_s` is in the vector on purpose: a VFP block writes S registers
/// through the same host pointer the wasm side reads, so a divergence there
/// is architectural state the harness must see at EVERY unit. Without it a
/// lockstep test could only assert the S register it happened to name, and
/// VSUB/VMUL/VDIV/VMOV coverage was one hand-written `assert_eq!` deep.
/// `fpscr` rides along so a test can prove the compiled lane read the same
/// FZ/DN modes the interpreter did.
pub fn snapshot_state(cpu: &CortexM) -> StateVec {
    let mut v = Vec::with_capacity(STATE_VEC_LEN);
    v.extend_from_slice(&[
        cpu.r0, cpu.r1, cpu.r2, cpu.r3, cpu.r4, cpu.r5, cpu.r6, cpu.r7, cpu.r8, cpu.r9, cpu.r10,
        cpu.r11, cpu.r12, cpu.sp, cpu.lr, cpu.pc, cpu.xpsr,
    ]);
    v.push(u32::from(cpu.primask));
    v.push(u32::from(cpu.faultmask));
    v.push(u32::from(cpu.basepri));
    v.push(u32::from(cpu.it_state));
    v.push(cpu.active_exception);
    for w in cpu.pending_exceptions {
        v.push(w as u32);
        v.push((w >> 32) as u32);
    }
    v.extend_from_slice(&cpu.fpu_s);
    v.push(cpu.fpscr);
    debug_assert_eq!(v.len(), STATE_VEC_LEN);
    v
}

/// Copy `r0..r12, sp, lr, xpsr` into the 16-word JIT register file.
pub fn pack_regs(cpu: &CortexM, out: &mut [u32; 16]) {
    out[0] = cpu.r0;
    out[1] = cpu.r1;
    out[2] = cpu.r2;
    out[3] = cpu.r3;
    out[4] = cpu.r4;
    out[5] = cpu.r5;
    out[6] = cpu.r6;
    out[7] = cpu.r7;
    out[8] = cpu.r8;
    out[9] = cpu.r9;
    out[10] = cpu.r10;
    out[11] = cpu.r11;
    out[12] = cpu.r12;
    out[13] = cpu.sp;
    out[14] = cpu.lr;
    out[15] = cpu.xpsr;
}

/// Write the 16-word JIT register file back into the core. Does not touch PC.
pub fn unpack_regs(cpu: &mut CortexM, src: &[u32; 16]) {
    cpu.r0 = src[0];
    cpu.r1 = src[1];
    cpu.r2 = src[2];
    cpu.r3 = src[3];
    cpu.r4 = src[4];
    cpu.r5 = src[5];
    cpu.r6 = src[6];
    cpu.r7 = src[7];
    cpu.r8 = src[8];
    cpu.r9 = src[9];
    cpu.r10 = src[10];
    cpu.r11 = src[11];
    cpu.r12 = src[12];
    cpu.sp = src[13];
    cpu.lr = src[14];
    cpu.xpsr = src[15];
}

/// A [`JitHost`] view over a Cortex-M machine.
pub struct CortexMJitHost<'m> {
    machine: &'m mut Machine<CortexM>,
}

impl<'m> CortexMJitHost<'m> {
    /// Wrap a machine for JIT dispatch.
    pub fn new(machine: &'m mut Machine<CortexM>) -> Self {
        Self { machine }
    }
}

impl JitHost for CortexMJitHost<'_> {
    fn pc(&self) -> Pc {
        self.machine.cpu.pc as Pc
    }

    fn interpret_one(&mut self) -> HostStep {
        match self.machine.step() {
            Ok(()) => HostStep::Advanced,
            Err(_) => HostStep::Halted,
        }
    }

    fn resume_at(&mut self, pc: Pc) {
        self.machine.cpu.pc = pc as u32;
    }

    fn code_bytes(&self, pc: Pc) -> Option<Vec<u8>> {
        let bytes = self
            .machine
            .bus
            .read_code_slice(pc, super::MAX_BLOCK_INSTRS as usize * 4);
        (bytes.len() >= 2).then_some(bytes)
    }

    fn safety(&self) -> SafetyGate {
        SafetyGate {
            observers_active: !self.machine.observers.is_empty(),
            breakpoints_active: !self.machine.breakpoints.is_empty(),
            probes_active: self.machine.logic_probes_active(),
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
