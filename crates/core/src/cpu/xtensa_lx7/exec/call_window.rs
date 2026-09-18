// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Arm bodies of `XtensaLx7::execute` for the call_window instruction class,
//! moved here verbatim. `execute` keeps the single `match ins`; each arm
//! calls one of these `#[inline(always)]` methods.

use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::{Bus, SimResult};

use crate::cpu::xtensa_regs::Ps;
use crate::cpu::xtensa_sr::EPC1;
use crate::cpu::xtensa_sr::EPC2;
use crate::cpu::xtensa_sr::EPC3;
use crate::cpu::xtensa_sr::EPC4;
use crate::cpu::xtensa_sr::EPC5;
use crate::cpu::xtensa_sr::EPC6;
use crate::cpu::xtensa_sr::EPC7;
use crate::cpu::xtensa_sr::EPS2;
use crate::cpu::xtensa_sr::EPS3;
use crate::cpu::xtensa_sr::EPS4;
use crate::cpu::xtensa_sr::EPS5;
use crate::cpu::xtensa_sr::EPS6;
use crate::cpu::xtensa_sr::EPS7;
use crate::cpu::xtensa_sr::VECBASE;
impl XtensaLx7 {
    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_call0(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        offset: i32,
    ) -> SimResult<()> {
        let ret_pc = self.pc.wrapping_add(3);
        let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
        self.regs.write_logical(0, ret_pc);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_callx0(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let ret_pc = self.pc.wrapping_add(3);
        let target = self.regs.read_logical(as_);
        self.regs.write_logical(0, ret_pc);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_call4(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        offset: i32,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
        let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
        self.spill_shadow_on_call(1);
        self.regs.write_logical(4, ret_pc);
        self.ps.set_callinc(1);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_call8(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        offset: i32,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
        let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
        self.spill_shadow_on_call(2);
        self.regs.write_logical(8, ret_pc);
        self.ps.set_callinc(2);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_call12(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        offset: i32,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
        let target = (self.pc.wrapping_add(4) & !3u32).wrapping_add(offset as u32);
        self.spill_shadow_on_call(3);
        self.regs.write_logical(12, ret_pc);
        self.ps.set_callinc(3);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_callx4(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (1 << 30);
        let target = self.regs.read_logical(as_);
        self.spill_shadow_on_call(1);
        self.regs.write_logical(4, ret_pc);
        self.ps.set_callinc(1);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_callx8(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (2 << 30);
        let target = self.regs.read_logical(as_);
        self.spill_shadow_on_call(2);
        self.regs.write_logical(8, ret_pc);
        self.ps.set_callinc(2);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_callx12(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
        as_: u8,
    ) -> SimResult<()> {
        let raw_ret = self.pc.wrapping_add(3);
        let ret_pc = (raw_ret & 0x3FFF_FFFF) | (3 << 30);
        let target = self.regs.read_logical(as_);
        self.spill_shadow_on_call(3);
        self.regs.write_logical(12, ret_pc);
        self.ps.set_callinc(3);
        self.pc = target;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_ret(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        self.pc = self.regs.read_logical(0);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_entry(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        as_: u8,
        imm: u32,
    ) -> SimResult<()> {
        let callinc = self.ps.callinc();
        let wb_old = self.regs.windowbase();
        let wb_new = wb_old.wrapping_add(callinc) & 0x0F;

        // F3: Window overflow detection. Per Xtensa ISA RM §4.7.1.6,
        // real silicon vectors to OF4/OF8/OF12 handlers that spill the
        // displaced frame to its stack save area, relying on a chain
        // of prior spills to know where the parent frame's SP is
        // (`l32e a0, a1, -12` in OF8/OF12). On a cold call chain that
        // wraps for the first time, no prior spill has primed that
        // chain, so the canonical handler reads garbage.
        //
        // We sidestep this with sim-level transparent spilling: on
        // CALL{n}, if the slot we'd land in is already live, save the
        // displaced frame's a0..a3 to a per-WB shadow stack BEFORE the
        // CALL clobbers them. On the corresponding RETW, restore.
        //
        // The displaced-frame save happens in the CALL{n} exec arms,
        // not here — by the time we reach ENTRY, the corruption has
        // already happened. See `spill_to_shadow_on_call` in this file.

        // Per Xtensa ISA RM §8.1.5 ENTRY:
        //   AR[WB_new*4 + as] = AR[WB_old*4 + as] - imm*8
        // i.e. read the SP from the CALLER's frame, subtract the
        // requested frame size, and write it into the CALLEE's frame
        // — a single value flowing across the window boundary. We
        // were reading post-rotation, which gave the callee an
        // uninitialized AR slot (typically 0) instead of caller's SP,
        // so chained CALL4 calls underflowed SP into 0xffffffXX and
        // every subsequent stack write trapped MemoryViolation.
        let caller_sp = self.regs.read_logical(as_);
        self.regs.set_windowbase(wb_new);
        self.regs.set_windowstart_bit(wb_new, true);
        self.ps.set_callinc(0);
        self.regs
            .write_logical(as_, caller_sp.wrapping_sub(imm * 8));
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_retw(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        let a0 = self.regs.read_logical(0);
        let n = (a0 >> 30) as u8; // bits[31:30] = callinc used by the call
        let wb_cur = self.regs.windowbase();
        let wb_dest = wb_cur.wrapping_sub(n) & 0x0F;

        // Shadow mode owns window save/restore only while it still
        // HOLDS the frames — i.e. while `call_preserve_stack` is
        // non-empty. Once `spill_call_preserve_to_stack` has run, the
        // frames live in the on-stack OF save areas and WINDOWSTART has
        // collapsed to `1<<WB`; from then on the firmware's underflow
        // handler is the correct reader and this shortcut must not fire.
        //
        // Dropping the `is_empty` condition (an earlier attempt at the
        // `graphicstest` fault below) is measurably wrong: with
        // LABWIRED_DIAG_RETW instrumentation, all 14 force-live events
        // in that run had `preserve_depth=0` and a prior spill, so the
        // shortcut skipped the only path that could still reload the
        // frame. `testFillScreen`'s RETW then returned a1=0x20 to
        // `setup()` and `Print::printNumber` faulted on the garbage SP.
        //
        // The remaining defect is NOT here. Adafruit's stock
        // `graphicstest` still faults (real silicon, an ESP32-D0WDQ6,
        // completes all twelve benchmarks) because the underflow
        // handler reads a save area the spill never populated:
        // `spill_call_preserve_to_stack` skips frames whose a1 fails
        // `valid_sp`/`stackish`, leaving holes. Fixing the holes is the
        // open work — see the graphicstest task.
        if !self.faithful_windows
            && !self.regs.windowstart_bit(wb_dest)
            && n > 0
            && !self.call_preserve_stack.is_empty()
        {
            self.regs.set_windowstart_bit(wb_dest, true);
            for k in 1..n {
                let s = wb_dest.wrapping_add(k) & 0x0F;
                self.regs.set_windowstart_bit(s, true);
            }
        }

        // F4: Window underflow check — destination frame must be live.
        if !self.regs.windowstart_bit(wb_dest) {
            // Window underflow path — symmetric to ENTRY's overflow:
            // rotate WB *backwards* by N (the call type encoded in
            // a0[31:30]) so the handler runs in the window the
            // caller-of-caller occupies. Save WB → PS.OWB so RFWU
            // can restore it. Set EXCM, EPC1, jump to UF vector.
            //
            // Window underflow vector offsets (Xtensa LX ISA RM §5.6):
            const UF4_VECOFS: u32 = 0x040;
            const UF8_VECOFS: u32 = 0x0C0;
            const UF12_VECOFS: u32 = 0x140;
            let vec_ofs = match n {
                1 => UF4_VECOFS,
                2 => UF8_VECOFS,
                _ => UF12_VECOFS, // N=3 → UF12; N=0 also lands here (see note above)
            };
            let vecbase = self.sr.read(VECBASE);
            self.sr.write(EPC1, self.pc);
            self.ps.set_owb(wb_cur);
            self.regs.set_windowbase(wb_dest);
            self.ps.set_excm(true);
            self.pc = vecbase.wrapping_add(vec_ofs);
            // Still consumed a RETW attempt — clear thunk IRQ deferral.
            self.defer_irq_until_retw = false;
            return Ok(());
        }

        // Normal RETW path (destination frame is live).
        let target_pc = (a0 & 0x3FFF_FFFF) | (self.pc & 0xC000_0000);
        self.regs.set_windowstart_bit(wb_cur, false);
        self.regs.set_windowbase(wb_dest);
        self.pc = target_pc;
        // The callee just placed its return value in its a2 =
        // AR[wb_cur*4 + 2] = caller's a{n*4 + 2} after rotation.
        // Save it before the pops below — displace pop would restore
        // stale data into that physical and clobber the return value.
        let return_value = if n > 0 {
            Some(self.regs.read_logical(n * 4 + 2))
        } else {
            None
        };
        // Hybrid restore:
        //  1. Classic LIFO for displace (callee window) + WS re-set —
        //     same as early-boot path; LIFO holds ONLY displaces now.
        //  2. Authoritative preserve from call_preserve_stack so outer
        //     a4..a7 cannot be stolen by a wrap-around displace sweep.
        for k in 0..4u8 {
            let slot = wb_cur.wrapping_add(k) & 0x0F;
            if self.regs.pop_shadow(slot) {
                self.regs.set_windowstart_bit(slot, true);
            }
        }
        self.restore_call_preserve();
        if let Some(rv) = return_value {
            self.regs.write_logical(n * 4 + 2, rv);
        }
        // Close the windowed-thunk IRQ deferral window (if any).
        self.defer_irq_until_retw = false;
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_movsp(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        at: u8,
        as_: u8,
    ) -> SimResult<()> {
        let wb = self.regs.windowbase();
        let next_idx = wb.wrapping_add(1) & 0x0F;

        if self.regs.windowstart_bit(next_idx) {
            // Adjacent frame is live — silicon raises AllocaCause and the
            // firmware handler spills one window to the stack save area.
            if self.faithful_windows {
                return self.vector_exception(5);
            }
            // Shadow / fast-boot mode: just perform the register move.
            // Live frames are already on the shadow stacks for RETW.
            // Raising AllocaCause hard-faults heap_caps_init's VLA path.
        }

        // Safe path: simple register move (stack-pointer adjust).
        let v = self.regs.read_logical(as_);
        self.regs.write_logical(at, v);
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rotw(
        &mut self,
        _bus: &mut dyn Bus,
        len: u32,
        n: i8,
    ) -> SimResult<()> {
        let wb = self.regs.windowbase();
        // n is i8 (range -8..=+7); wrapping add modulo 16.
        let wb_new = (wb as i32).wrapping_add(n as i32).rem_euclid(16) as u8;
        self.regs.set_windowbase(wb_new);
        // WindowStart is NOT modified (ISA RM §8 ROTW).
        self.pc = self.pc.wrapping_add(len);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfe(
        &mut self,
        bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        self.ps.set_excm(false);
        self.pc = self.sr.read(EPC1);
        if !self.faithful_windows {
            self.pop_irq_window_frame(bus);
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfde(
        &mut self,
        bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        self.ps.set_excm(false);
        self.pc = self.sr.read(EPC1);
        if !self.faithful_windows {
            self.pop_irq_window_frame(bus);
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfi(
        &mut self,
        bus: &mut dyn Bus,
        len: u32,
        level: u8,
    ) -> SimResult<()> {
        let (eps_id, epc_id) = match level {
            2 => (EPS2, EPC2),
            3 => (EPS3, EPC3),
            4 => (EPS4, EPC4),
            5 => (EPS5, EPC5),
            6 => (EPS6, EPC6),
            7 => (EPS7, EPC7),
            _ => {
                // Invalid level — skip silently.
                self.pc = self.pc.wrapping_add(len);
                return Ok(());
            }
        };
        let new_ps = self.sr.read(eps_id);
        let new_pc = self.sr.read(epc_id);
        self.ps = Ps::from_raw(new_ps);
        self.pc = new_pc;
        if !self.faithful_windows {
            self.pop_irq_window_frame(bus);
        }
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfwo(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        let wb_handler = self.regs.windowbase();
        let wb_old = self.ps.owb();
        self.regs.set_windowstart_bit(wb_handler, false);
        self.regs.set_windowbase(wb_old);
        self.ps.set_excm(false);
        self.pc = self.sr.read(EPC1);
        Ok(())
    }

    #[inline(always)]
    pub(in crate::cpu::xtensa_lx7) fn exec_rfwu(
        &mut self,
        _bus: &mut dyn Bus,
        _len: u32,
    ) -> SimResult<()> {
        let wb_handler = self.regs.windowbase();
        let wb_old = self.ps.owb();
        self.regs.set_windowstart_bit(wb_handler, true);
        self.regs.set_windowbase(wb_old);
        self.ps.set_excm(false);
        self.pc = self.sr.read(EPC1);
        Ok(())
    }
}
