// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Split out of `step_inner` in `cpu/avr.rs` (pure move, no behaviour
// change). See that file's `step_inner` and `exec/mod.rs` for the shared
// fetch/next-pc plumbing and the fixed original check order this respects.

use crate::cpu::avr::Avr;
use crate::{Bus, SimResult};

impl Avr {
    /// SBI, CBI.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_bitops_a(
        &mut self,
        bus: &mut dyn Bus,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // SBI
        if (op & 0xFF00) == 0x9A00 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? | (1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }

        // CBI
        if (op & 0xFF00) == 0x9800 {
            let a = ((op >> 3) & 0x1F) as u8;
            let b = (op & 0x07) as u8;
            let data_addr = 0x20u16 + a as u16;
            let v = self.data_read(data_addr, bus)? & !(1 << b);
            self.data_write(data_addr, v, bus)?;
            self.pc = next;
            self.cycles += 2;
            return Ok(Some(()));
        }
        Ok(None)
    }

    /// SWAP, ASR, LSR, ROR.
    #[inline(always)]
    pub(in crate::cpu::avr) fn exec_bitops_b(
        &mut self,
        op: u16,
        next: u32,
    ) -> SimResult<Option<()>> {
        // SWAP Rd: 1001 010d dddd 0010
        if (op & 0xFE0F) == 0x9402 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let v = self.r[rd];
            self.r[rd] = v.rotate_left(4);
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ASR Rd: 1001 010d dddd 0101
        if (op & 0xFE0F) == 0x9405 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let res = ((a as i8) >> 1) as u8;
            self.r[rd] = res;
            self.set_c(a & 1 != 0);
            self.set_z(res);
            self.set_n(res);
            self.set_v(((res >> 7) ^ (a & 1)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // LSR Rd: 1001 010d dddd 0110
        if (op & 0xFE0F) == 0x9406 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c = a & 1 != 0;
            let res = a >> 1;
            self.r[rd] = res;
            self.set_c(c);
            self.set_z(res);
            self.sreg &= !0x04; // N = 0
            self.set_v(c); // V = N⊕C = C
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }

        // ROR Rd: 1001 010d dddd 0111
        if (op & 0xFE0F) == 0x9407 {
            let rd = ((op >> 4) & 0x1F) as usize;
            let a = self.r[rd];
            let c_in = self.sreg & 1;
            let c_out = a & 1 != 0;
            let res = (a >> 1) | (c_in << 7);
            self.r[rd] = res;
            self.set_c(c_out);
            self.set_z(res);
            self.set_n(res);
            let n = (res >> 7) & 1;
            self.set_v((n ^ (c_out as u8)) != 0);
            self.update_s_from_nv();
            self.pc = next;
            self.cycles += 1;
            return Ok(Some(()));
        }
        Ok(None)
    }
}
