// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_execute` in `cpu/cortex_m.rs` (pure move, no behaviour
// change). See that file's `step_execute` for the shared exception/IT-state
// plumbing that wraps these per-class dispatches.

use super::super::CortexM;
use crate::decoder::arm::Instruction;
use crate::{Bus, Cpu, SimResult};

impl CortexM {
    #[allow(clippy::too_many_lines)]
    pub(in crate::cpu::cortex_m) fn exec_system<B: Bus + ?Sized>(
        &mut self,
        _bus: &mut B,
        instruction: Instruction,
        pc_increment: &mut u32,
        _it_block_instruction: &mut bool,
    ) -> SimResult<()> {
        match instruction {
            Instruction::Nop => { /* Do nothing */ }
            Instruction::Wfi => {
                // ARMv7-M WFI: complete as a NOP if a wake-up event is
                // already pending, otherwise suspend the core until one
                // arrives. Wake-up ignores PRIMASK (see `wfi_wake_pending`);
                // `Machine::run` fast-forwards the idle window while
                // `self.sleeping` holds. PC has already advanced past the
                // WFI like any other 16-bit hint.
                if !self.wfi_wake_pending() {
                    self.sleeping = true;
                }
            }
            Instruction::Cpsie { primask, faultmask } => {
                if primask {
                    self.primask = false;
                }
                if faultmask {
                    self.faultmask = false;
                }
            }
            Instruction::Cpsid { primask, faultmask } => {
                if primask {
                    self.primask = true;
                }
                if faultmask {
                    self.faultmask = true;
                }
            }

            // Shifts
            Instruction::Bkpt { imm8 } => {
                // ARM semihosting uses `bkpt #0xAB` as the trap into
                // the debugger. On real silicon openocd intercepts
                // these and emulates the syscall (WRITEC, WRITE0,
                // SYS_EXIT, …). The simulator doesn't emulate the
                // syscalls itself — firmware that wants the same
                // bytes available on both sides should also emit
                // them via UART, which our sink already captures.
                // Treating semihosting BKPT as a no-op here lets
                // such dual-emit firmware run identically on sim
                // and silicon. Any other BKPT immediate (typical
                // for `panic!` traps or debugger breakpoints) is
                // still a halt.
                if imm8 != 0xAB {
                    return Err(crate::SimulationError::Halt);
                }
            }

            Instruction::Svc { imm8: _ } => {
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
            }

            // Stack Operations
            Instruction::Barrier => {
                // DMB / DSB / ISB — architectural no-ops on a single-threaded
                // simulator. They're modelled explicitly so they don't raise
                // DecodeError; startup code and HAL inline-asm emit them
                // routinely.
                *pc_increment = 4;
            }
            Instruction::Mrs { rd, sysm } => {
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
                *pc_increment = 4;
            }
            Instruction::Msr { sysm, rn } => {
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
                *pc_increment = 4;
            }
            _ => unreachable!("exec_system called with instruction from a different class"),
        }
        Ok(())
    }
}
