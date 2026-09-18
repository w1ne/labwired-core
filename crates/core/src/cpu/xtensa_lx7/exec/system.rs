// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the system instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};

use crate::cpu::xtensa_regs::Ps;
use crate::cpu::xtensa_sr::INTERRUPT;
use crate::decoder::xtensa;
use crate::SimulationError;
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_break(
        &mut self,
        bus: &mut dyn Bus,
        _len: u32,
        imm_s: u8,
        imm_t: u8,
    ) -> SimResult<()> {
        use crate::peripherals::esp_xtensa_common::rom_thunks::{ROM_THUNK_IMM_S, ROM_THUNK_IMM_T};
        if imm_s == ROM_THUNK_IMM_S && imm_t == ROM_THUNK_IMM_T {
            let pc = self.pc;
            if let Some(thunk) = bus.get_rom_thunk(pc) {
                return thunk(self, bus);
            }
            return Err(SimulationError::NotImplemented(format!(
                "ROM thunk at 0x{pc:08x} not registered (BREAK 1,14 with no thunk)"
            )));
        }
        Err(SimulationError::BreakpointHit(self.pc))
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_waiti(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        level: u8,
    ) -> SimResult<()> {
        // Xtensa ISA RM, WAITI: PS.INTLEVEL ← level, then the core
        // suspends. **WAITI retires before it waits** — the wait state
        // sits between WAITI and its successor, so the interrupt that
        // ends it is taken with EPC[level] = the address of the
        // instruction AFTER the WAITI, and RFI/RFE resumes there.
        //
        // Advancing the PC here is load-bearing, not cosmetic. Parking
        // *on* the WAITI made every wake re-enter it: dispatch_irq
        // latched EPC1 = the WAITI's own address, the handler ran, and
        // RFE dropped the core straight back into the wait. Code that
        // must make forward progress after a wake therefore never did.
        // ESP-IDF's SMP bring-up is exactly that shape — core 1's idle
        // task calls esp_cpu_wait_for_intr() from
        // esp_vApplicationIdleHook() and only reaches the registered
        // idle hooks (which set `s_other_cpu_startup_done`) on the NEXT
        // loop iteration, i.e. after the call returns. With the PC
        // pinned, core 1 took its systimer tick hundreds of times and
        // still never returned from the call, so core 0 spun forever in
        // main_task's `while (!s_other_cpu_startup_done)`.
        //
        // `waiti_parked` is what models the wait itself: later steps
        // skip fetch/decode (and let the idle fast-forward run) until a
        // wake-capable IRQ arrives, at which point the pre-fetch
        // interrupt check clears the park and dispatches with the PC
        // already pointing past the WAITI.
        self.ps.set_intlevel(level);
        self.waiti_parked = true;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_loop(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ins: xtensa::Instruction,
        as_: u8,
        offset: i32,
    ) -> SimResult<()> {
        use crate::cpu::xtensa_sr::{LBEG, LCOUNT, LEND};
        use crate::decoder::xtensa::Instruction::{Loop, Loopgtz, Loopnez};
        let count = self.regs.read_logical(as_);
        // ISA RM §7.4: LOOPNEZ/LOOPGTZ skip body when count is
        // 0/non-positive. LOOP always enters the body. The post-LEND
        // check decrements LCOUNT and branches back while LCOUNT > 0.
        let take = match ins {
            Loop { .. } => true,
            Loopnez { .. } => count != 0,
            Loopgtz { .. } => (count as i32) > 0,
            _ => unreachable!(),
        };
        let after = self.pc.wrapping_add(len);
        // ISA RM §7.4.1: LEND = LOOP_PC + 4 + imm8. Decoder produces
        // `offset = imm8 + 4`, so LEND = PC + offset (not PC+len+offset).
        let lend = (self.pc as i32).wrapping_add(offset) as u32;
        if take {
            self.sr.write(LBEG, after);
            self.sr.write(LEND, lend);
            // LCOUNT = count - 1 (wrapping). With post-LEND check
            // `if LCOUNT > 0 { LCOUNT--; PC = LBEG; }`, body runs
            // exactly count times for count > 0. For LOOP-with-
            // count=0, LCOUNT wraps to 0xFFFFFFFF so the body
            // iterates ~unbounded — terminated only by the body's
            // own internal branches (this is how strlen sweeps for
            // a null byte without a fixed upper bound).
            self.sr.write(LCOUNT, count.wrapping_sub(1));
            self.pc = after; // fall through to loop body
        } else {
            // LOOPNEZ/LOOPGTZ with non-positive count: skip body.
            self.sr.write(LCOUNT, 0);
            self.pc = lend;
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rer(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
    ) -> SimResult<()> {
        let _addr = self.regs.read_logical(as_);
        self.regs.write_logical(at, 0);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_syscall(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        self.vector_exception(1)
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rsr(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        at: u8,
        sr: u16,
    ) -> SimResult<()> {
        let mut v = self.read_sr(sr);
        // INTERRUPT (SR 226) is a hardware-aggregated view of pending
        // interrupts. In our model, peripheral source IDs route
        // through the bus's `pending_cpu_irqs` aggregator and never
        // touch the SR-file `INTERRUPT` slot directly. esp-hal's
        // `__level_1_interrupt` reads INTERRUPT to find which
        // peripheral source fired, so we must OR the bus-side bits
        // in here, otherwise the firmware sees INTERRUPT=0 and
        // never dispatches to the user ISR (Plan 3 Task 10 case
        // study).
        if sr == INTERRUPT {
            // Per-core IRQ routing handled by the bus aggregator:
            // PRO_CPU (core 0) gets peripheral source IRQs; both cores
            // get their own cross-core FROM_CPU IPIs. APP_CPU never
            // sees PRO_CPU's peripheral interrupts (which would unbalance
            // its critical nesting → vPortExitCritical "nesting > 0").
            v |= bus.pending_cpu_irqs(self.core_id());
        }
        self.regs.write_logical(at, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_wsr(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        sr: u16,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(at);
        self.write_sr(sr, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_xsr(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        sr: u16,
    ) -> SimResult<()> {
        let new_v = self.regs.read_logical(at);
        let old_v = self.read_sr(sr);
        self.write_sr(sr, new_v);
        self.regs.write_logical(at, old_v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rur(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        ar: u8,
        ur: u16,
    ) -> SimResult<()> {
        let v = self.ur[(ur as usize) & 0xFF];
        self.regs.write_logical(ar, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_wur(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        ur: u16,
    ) -> SimResult<()> {
        let v = self.regs.read_logical(at);
        self.ur[(ur as usize) & 0xFF] = v;
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rsil(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        level: u8,
    ) -> SimResult<()> {
        let prev_ps = self.ps.as_raw();
        self.regs.write_logical(at, prev_ps);
        let new_ps = (prev_ps & !0xF) | (level as u32 & 0xF);
        self.ps = Ps::from_raw(new_ps);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ill(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        self.raise_general_exception(0)
    }
}
