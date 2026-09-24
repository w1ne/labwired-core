// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Move-only split of the arm bodies of `CortexM::step_execute` in
// `cpu/cortex_m.rs`. Each `exec_*` method below is the verbatim body of one
// `match instruction` arm; `step_execute` still holds the single dispatch
// match (same arms, same order) and calls one method per arm. PC advance is
// communicated by the `PcAdvance` return value, not a `&mut` out-parameter.

use super::super::CortexM;
use super::super::PcAdvance;
use crate::Cpu;
use crate::SimResult;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_wfi(&mut self) -> SimResult<PcAdvance> {
        // ARMv7-M WFI: complete as a NOP if a wake-up event is
        // already pending, otherwise suspend the core until one
        // arrives. Wake-up ignores PRIMASK (see `wfi_wake_pending`);
        // `Machine::run` fast-forwards the idle window while
        // `self.sleeping` holds. PC has already advanced past the
        // WFI like any other 16-bit hint.
        if !self.wfi_wake_pending() {
            self.sleeping = true;
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cpsie(
        &mut self,
        primask: bool,
        faultmask: bool,
    ) -> SimResult<PcAdvance> {
        if primask {
            self.primask = false;
        }
        if faultmask {
            self.faultmask = false;
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cpsid(
        &mut self,
        primask: bool,
        faultmask: bool,
    ) -> SimResult<PcAdvance> {
        if primask {
            self.primask = true;
        }
        if faultmask {
            self.faultmask = true;
        }
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_bkpt<B: crate::Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        imm8: u8,
    ) -> SimResult<PcAdvance> {
        // `0xAB` is the Thumb semihosting trap. The body stays out of line so
        // this `#[inline(always)]` arm remains a branch and `step_execute`
        // does not grow. Any other immediate is still a halt.
        if imm8 != 0xAB {
            return Err(crate::SimulationError::Halt);
        }
        crate::cpu::cortex_m::semihost::handle(self, bus)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_svc(&mut self) -> SimResult<PcAdvance> {
        // Supervisor call. Pend the SVCall exception (number 11);
        // the exception-entry path at the top of `step_internal`
        // stacks the frame and vectors to the handler on the next
        // step. Zephyr drives its fatal handler, irq_offload, and
        // userspace syscalls through SVC, so an unmodeled SVC left
        // the PC stuck on the instruction (ztest hung forever).
        // The immediate selects the call on the Zephyr side; the
        // handler recovers it from the stacked instruction, so we
        // don't branch on it here. pc_increment stays 2 so the
        // stacked return address points just past the SVC.
        self.set_exception_pending(11);
        Ok(PcAdvance::Keep)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_barrier(&mut self) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // DMB / DSB / ISB — architectural no-ops on a single-threaded
        // simulator. They're modelled explicitly so they don't raise
        // DecodeError; startup code and HAL inline-asm emit them
        // routinely.
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_mrs(&mut self, rd: u8, sysm: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // IPSR (the active exception number, xPSR[8:0]) is load-bearing
        // for Zephyr: _isr_wrapper reads it and computes `IRQ = IPSR-16`
        // to index the software ISR table. Returning 0 made the index
        // -16 → garbage handler. The xPSR/IPSR-bearing reads all expose
        // the current exception number; PRIMASK, BASEPRI, FAULTMASK,
        // the banked SPs and CONTROL are the other modelled special
        // registers. Anything else still reads as zero.
        let ipsr = self.active_exception & 0x1FF;
        let val: u32 = match sysm {
            0x00 => self.xpsr & 0xF800_0000,          // APSR (condition flags)
            0x03 => (self.xpsr & 0xF800_0000) | ipsr, // xPSR
            0x05 => ipsr,                             // IPSR
            0x08 => self.read_msp(),                  // MSP
            0x09 => self.read_psp(),                  // PSP
            0x10 => self.primask as u32,
            // BASEPRI (0x11) and BASEPRI_MAX (0x12) both read BASEPRI.
            0x11 | 0x12 => self.basepri as u32,
            0x13 => self.faultmask as u32, // FAULTMASK
            0x14 => self.control & 0x3,    // CONTROL
            _ => 0,
        };
        self.write_reg(rd, val);
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_msr(&mut self, sysm: u8, rn: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let val = self.read_reg(rn);
        match sysm {
            0x08 => {
                // MSP bank. If MSP is the live stack, update `sp` too.
                self.msp = val;
                if !self.use_psp() {
                    self.sp = val;
                }
            }
            0x09 => {
                // PSP bank. If PSP is the live stack, update `sp` too.
                self.psp = val;
                if self.use_psp() {
                    self.sp = val;
                }
            }
            0x10 => self.primask = (val & 1) != 0,
            // BASEPRI: plain write of the priority mask byte.
            0x11 => self.basepri = (val & 0xFF) as u8,
            // BASEPRI_MAX: writes BASEPRI only if it raises the
            // masking level (smaller non-zero value), or BASEPRI is 0.
            0x12 => {
                let new = (val & 0xFF) as u8;
                if new != 0 && (self.basepri == 0 || new < self.basepri) {
                    self.basepri = new;
                }
            }
            0x13 => self.faultmask = (val & 1) != 0, // FAULTMASK
            0x14 => {
                // CONTROL.SPSEL can switch the active thread stack.
                // Persist the live `sp` to its bank, change SPSEL/nPRIV,
                // then re-point `sp` at the newly-selected bank.
                self.sync_sp_to_bank();
                self.control = (self.control & !0x3) | (val & 0x3);
                self.sp = self.current_stack_value();
            }
            _ => {}
        }
        __pc = PcAdvance::Add4;
        Ok(__pc)
    }
}
