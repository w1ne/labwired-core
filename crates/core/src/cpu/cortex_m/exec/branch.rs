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

use super::super::AccessWidth;
use super::super::CortexM;
use super::super::PcAdvance;
use crate::Bus;
use crate::SimResult;

impl CortexM {
    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_tbb<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let mut base = self.read_reg(rn);
        if rn == 15 {
            // ARMv7-M: the table base is the in-execution PC
            // (insn address + 4) — NOT word-aligned. The old
            // Align(PC,4) read the table 2 bytes early whenever
            // the TBB sat at a 2-mod-4 address; ST's
            // HAL_DMA_RegisterCallback dispatched every
            // callback ID into the same slot because of it.
            base = self.pc.wrapping_add(4);
        }
        let index = self.read_reg(rm);
        let addr = base.wrapping_add(index);
        let byte = self.load(bus, addr, AccessWidth::Byte)?;
        let offset = byte << 1;
        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_tbh<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rn: u8,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let mut base = self.read_reg(rn);
        if rn == 15 {
            // Same unaligned PC+4 base rule as TBB above.
            base = self.pc.wrapping_add(4);
        }
        let index = self.read_reg(rm);
        let addr = base.wrapping_add(index << 1);
        let halfword = self.load(bus, addr, AccessWidth::Half)?;
        let offset = halfword << 1;
        self.pc = self.pc.wrapping_add(4).wrapping_add(offset);
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cbz(&mut self, rn: u8, imm: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        if self.read_reg(rn) == 0 {
            self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
            __pc = PcAdvance::Zero;
        }
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_cbnz(&mut self, rn: u8, imm: u8) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        if self.read_reg(rn) != 0 {
            self.pc = self.pc.wrapping_add(4).wrapping_add(imm as u32);
            __pc = PcAdvance::Zero;
        }
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_branch(&mut self, offset: i32) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
        self.pc = target;
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_bl(&mut self, offset: i32) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        // BL: Branch with Link.
        // LR = Next Instruction Address | 1 (Thumb bit)
        let _next_pc = self.pc.wrapping_add(4); // 32-bit instruction size for BL?
                                                // Wait. BL is decoded as 32-bit.
                                                // If we assume decode_thumb_16 handled a 32-bit stream, then PC increment should be adjusted?
                                                // Or does `decode_thumb_16` return `BlPrefix` and then we handle it?
                                                // The current `decoder` returns `Bl` with full offset if it sees the pair??
                                                // NO. My decoder implementation for BL (in previous turn) was:
                                                // `Instruction::Bl { offset: offset << 1 }`
                                                // But `decode_thumb_16` ONLY sees 16 bits. It cannot see the second half!
                                                // Real decoding of BL requires fetching 32 bits.

        // CRITICAL CORRECTION: `decode_thumb_16` is 16-bit.
        // BL is 32-bit (encoded as two 16-bit halves).
        // Fetch loop fetches 16 bits.
        // 1. Fetch High Half (0xF0xx). Returns BlPrefix?
        // 2. Fetch Low Half (0xF8xx). Combine?

        // My logic in decoder needs revisit. I put `Bl { offset }` thinking T1/T2 but BL is always 32-bit in Thumb-2.
        // T1 encoding of BL doesn't exist as single 16-bit.

        // For now, let's just implement the execution stub assuming the decoder *somehow* gave us the full BL.
        // But since the decoder only sees 16 bits, we need to handle the prefix state in the CPU loop!

        self.lr = self.pc.wrapping_add(4) | 1;
        let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
        self.pc = target;
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_branch_cond(
        &mut self,
        cond: u8,
        offset: i32,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        if self.check_condition(cond) {
            let target = (self.pc as i32).wrapping_add(4).wrapping_add(offset) as u32;
            self.pc = target;
            __pc = PcAdvance::Zero;
        }
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_bx<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let target = self.read_reg(rm);
        self.branch_to(target, bus)?;
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }

    #[inline(always)]
    pub(in crate::cpu::cortex_m) fn exec_blx_reg<B: Bus + ?Sized>(
        &mut self,
        bus: &mut B,
        rm: u8,
    ) -> SimResult<PcAdvance> {
        let mut __pc = PcAdvance::Keep;
        let target = self.read_reg(rm);
        self.lr = (self.pc.wrapping_add(2)) | 1;
        self.branch_to(target, bus)?;
        __pc = PcAdvance::Zero;
        Ok(__pc)
    }
}
