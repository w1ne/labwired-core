// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Nuvoton NPCX Core Domain Clock Generator: the HFCG and LFCG register
//! blocks that Renode maps as `Miscellaneous.NPCX_HFCG` at 0x400B5000 and
//! `Miscellaneous.NPCX_LFCG` at 0x400B5100 on the NPCX9 (0x100 bytes each).
//!
//! Sources: Renode `NPCX_HFCG.cs` / `NPCX_LFCG.cs` (register offsets and
//! field widths), Zephyr `soc/nuvoton/npcx/common/reg/reg_def.h` `cdcg_reg`
//! (the same offsets, plus `HFCBCD3` at +0x1D) and Zephyr
//! `clock_control_npcx.c` (the M/N load sequence).
//!
//! Renode defines every field as a tag: reads return 0 and writes are
//! dropped. This model keeps the written values instead, so firmware that
//! reads back its clock configuration (Zephyr skips the M/N reload when
//! `HFCGN` / `HFCGML` / `HFCGMH` already match) sees what it wrote.
//!
//! Modelled (HFCG):
//! - `HFCGCTRL` +0x00: `LOAD` bit 0 is a command and reads 0; `CLK_CHNG`
//!   bit 7 reads 0, so the "wait for stable" poll ends at once (as on
//!   Renode); `LOCK` bit 2 is set-only and then drops every HFCG write;
//! - `HFCGML` +0x02, `HFCGMH` +0x04, `HFCGN` +0x06 (bits 0..5 and 7),
//!   `HFCGP` +0x08 (bits 0..1, 4..7), `HFCBCD` +0x10 (bits 1, 4..5),
//!   `HFCBCD1` +0x12, `HFCBCD2` +0x14, `HFCGPIDL` +0x1C (bits 3..7) and
//!   `HFCBCD3` +0x1D read back with the reserved bits clear;
//! - a `LOAD` with the block unlocked latches M and N (`loads` / `loaded_m`
//!   / `loaded_n` in the snapshot).
//!
//! Modelled (LFCG, offsets from 0x400B5100):
//! - `LFCGCTL` +0x00: `LREFEN` bit 2 reads back; `UDCP` bit 4 is a command
//!   and reads 0 (assumption: Renode tags it, no source gives its read
//!   value); `LFLOC` bit 5 is set-only and then drops every LFCG write;
//!   `LFLER` bit 3 and `XTCLK_VAL` bit 7 read 0;
//! - `HFRDI` +0x02 (12 bits), `HFRDF` +0x04 (16 bits), `FRCDIV` +0x06,
//!   `DIVCOR1` +0x08, `DIVCOR2` +0x0A (8 bits each) and `LFCGCTL2` +0x14
//!   (bits 3..6) read back. Byte and 16-bit accesses both work.
//!
//! Reset values are 0, as Renode reads them; the datasheet values are not
//! available to this model. A system or node reset returns them to 0 and
//! clears both locks (assumption, not checked against silicon).
//!
//! NOT modelled: the clock tree. The core clock stays at the chip yaml's
//! `cpu_hz` whatever M, N, `FPRED` or the APB dividers say, and the
//! timers keep their configured rates. `CLK_CHNG` never shows a settling
//! delay. Undefined offsets read 0 and drop writes.

use crate::{Peripheral, SimResult};

/// Which of the two 0x100-byte blocks this instance is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CdcgBlock {
    Hfcg,
    Lfcg,
}

pub const HFCGCTRL: u64 = 0x00;
pub const HFCGCTRL_LOAD: u8 = 1 << 0;
pub const HFCGCTRL_LOCK: u8 = 1 << 2;
pub const HFCGCTRL_CLK_CHNG: u8 = 1 << 7;
pub const HFCGML: u64 = 0x02;
pub const HFCGMH: u64 = 0x04;
pub const HFCGN: u64 = 0x06;

pub const LFCGCTL: u64 = 0x00;
pub const LFCGCTL_UDCP: u8 = 1 << 4;
pub const LFCGCTL_LFLOC: u8 = 1 << 5;
pub const HFRDI: u64 = 0x02;
pub const LFCGCTL2: u64 = 0x14;

const SIZE: usize = 0x100;

/// Writable bits per byte offset (0 = reserved / undefined).
fn hfcg_mask(offset: u64) -> u8 {
    match offset {
        // LOAD and LOCK; both are handled in `write`.
        HFCGCTRL => HFCGCTRL_LOAD | HFCGCTRL_LOCK,
        HFCGML | HFCGMH => 0xFF,
        HFCGN => 0xBF,
        // HFCGP: AHB6DIV 0..1, FPRED 4..7.
        0x08 => 0xF3,
        // HFCBCD: AHB6CLK_BLK 1, FIUDIV 4..5.
        0x10 => 0x32,
        // HFCBCD1 (APB1DIV / APB2DIV), HFCBCD2 (APB3DIV / APB4DIV).
        0x12 | 0x14 => 0xFF,
        // HFCGPIDL: FPRED_IDL_EN 3, FPRED_IDL 4..7 (Renode +0x1C).
        0x1C => 0xF8,
        // HFCBCD3 (Zephyr reg_def.h +0x1D; not in Renode).
        0x1D => 0xFF,
        _ => 0,
    }
}

fn lfcg_mask(offset: u64) -> u8 {
    match offset {
        // LREFEN, UDCP, LFLOC; UDCP and LFLOC are handled in `write`.
        LFCGCTL => (1 << 2) | LFCGCTL_UDCP | LFCGCTL_LFLOC,
        // HFRDI 12 bits.
        0x02 => 0xFF,
        0x03 => 0x0F,
        // HFRDF 16 bits.
        0x04 | 0x05 => 0xFF,
        // FRCDIV, DIVCOR1, DIVCOR2: 8 bits in a 16-bit register.
        0x06 | 0x08 | 0x0A => 0xFF,
        // LFCGCTL2: AUDP_EN 3, STOPCAL 4, FCLKRUN 5, XT_OSC_SL_EN 6.
        LFCGCTL2 => 0x78,
        _ => 0,
    }
}

/// NPCX HFCG or LFCG (see the module docs).
#[derive(Debug, Clone)]
pub struct NpcxCdcg {
    block: CdcgBlock,
    regs: [u8; SIZE],
    /// HFCG: `LOAD` commands accepted.
    loads: u32,
    /// HFCG: M and N latched by the last `LOAD`.
    loaded_m: u16,
    loaded_n: u8,
}

impl NpcxCdcg {
    pub fn new(block: CdcgBlock) -> Self {
        Self {
            block,
            regs: [0; SIZE],
            loads: 0,
            loaded_m: 0,
            loaded_n: 0,
        }
    }

    fn locked(&self) -> bool {
        match self.block {
            CdcgBlock::Hfcg => self.regs[HFCGCTRL as usize] & HFCGCTRL_LOCK != 0,
            CdcgBlock::Lfcg => self.regs[LFCGCTL as usize] & LFCGCTL_LFLOC != 0,
        }
    }

    fn mask(&self, offset: u64) -> u8 {
        match self.block {
            CdcgBlock::Hfcg => hfcg_mask(offset),
            CdcgBlock::Lfcg => lfcg_mask(offset),
        }
    }

    fn name(&self) -> &'static str {
        match self.block {
            CdcgBlock::Hfcg => "npcx_hfcg",
            CdcgBlock::Lfcg => "npcx_lfcg",
        }
    }

    fn write_reg(&mut self, offset: u64, value: u8) {
        let mask = self.mask(offset);
        if mask == 0 {
            crate::census_reg!(self.name(), offset, "write");
            return;
        }
        if self.locked() {
            return;
        }
        let i = offset as usize;
        let mut v = value & mask;
        match (self.block, offset) {
            (CdcgBlock::Hfcg, HFCGCTRL) => {
                if v & HFCGCTRL_LOAD != 0 {
                    self.loads = self.loads.wrapping_add(1);
                    self.loaded_m = u16::from_le_bytes([
                        self.regs[HFCGML as usize],
                        self.regs[HFCGMH as usize],
                    ]);
                    self.loaded_n = self.regs[HFCGN as usize];
                }
                // LOAD reads 0, CLK_CHNG reads 0: the multiplier settles at
                // once. LOCK is set-only (and once set, this is unreachable).
                v &= HFCGCTRL_LOCK;
            }
            // UDCP reads 0; LFLOC is set-only (unreachable once set).
            (CdcgBlock::Lfcg, LFCGCTL) => v &= !LFCGCTL_UDCP,
            _ => {}
        }
        self.regs[i] = v;
    }

    fn read_reg(&self, offset: u64) -> u8 {
        if self.mask(offset) == 0 {
            crate::census_reg!(self.name(), offset, "read");
            return 0;
        }
        self.regs[offset as usize]
    }
}

impl Peripheral for NpcxCdcg {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_reg(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_reg(offset, value);
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        (self.mask(offset) != 0).then(|| self.regs[offset as usize])
    }

    /// Pure register file: no tick, no IRQ.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn on_system_reset(&mut self) {
        *self = Self::new(self.block);
    }

    fn on_node_reset(&mut self) {
        *self = Self::new(self.block);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "block": self.name(),
            "locked": self.locked(),
            "loads": self.loads,
            "loaded_m": self.loaded_m,
            "loaded_n": self.loaded_n,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wr(p: &mut NpcxCdcg, offset: u64, value: u8) {
        p.write(offset, value).unwrap();
    }

    fn rd(p: &NpcxCdcg, offset: u64) -> u8 {
        p.read(offset).unwrap()
    }

    #[test]
    fn hfcg_load_sequence_reads_back_and_settles_at_once() {
        let mut h = NpcxCdcg::new(CdcgBlock::Hfcg);
        // Zephyr clock_control_npcx.c order: N, ML, MH, then LOAD.
        wr(&mut h, HFCGN, 0x02);
        wr(&mut h, HFCGML, 0xBA);
        wr(&mut h, HFCGMH, 0x0A);
        wr(&mut h, HFCGCTRL, HFCGCTRL_LOAD);
        assert_eq!(rd(&h, HFCGCTRL) & (HFCGCTRL_LOAD | HFCGCTRL_CLK_CHNG), 0);
        assert_eq!(
            (rd(&h, HFCGN), rd(&h, HFCGML), rd(&h, HFCGMH)),
            (0x02, 0xBA, 0x0A)
        );
        assert_eq!((h.loads, h.loaded_m, h.loaded_n), (1, 0x0ABA, 0x02));
    }

    #[test]
    fn hfcg_reserved_bits_and_offsets_read_zero() {
        let mut h = NpcxCdcg::new(CdcgBlock::Hfcg);
        for (offset, mask) in [
            (HFCGN, 0xBF),
            (0x08, 0xF3),
            (0x10, 0x32),
            (0x12, 0xFF),
            (0x14, 0xFF),
            (0x1C, 0xF8),
            (0x1D, 0xFF),
            (0x01, 0x00),
            (0x0A, 0x00),
            (0xFF, 0x00),
        ] {
            wr(&mut h, offset, 0xFF);
            assert_eq!(rd(&h, offset), mask, "offset {offset:#x}");
        }
        // HFCGCTRL: only LOCK reads back.
        let mut h = NpcxCdcg::new(CdcgBlock::Hfcg);
        wr(&mut h, HFCGCTRL, 0xFF);
        assert_eq!(rd(&h, HFCGCTRL), HFCGCTRL_LOCK);
    }

    #[test]
    fn hfcg_lock_drops_every_write_until_reset() {
        let mut h = NpcxCdcg::new(CdcgBlock::Hfcg);
        wr(&mut h, HFCGML, 0x11);
        wr(&mut h, HFCGCTRL, HFCGCTRL_LOCK);
        wr(&mut h, HFCGML, 0x22);
        wr(&mut h, 0x12, 0x33);
        wr(&mut h, HFCGCTRL, HFCGCTRL_LOAD);
        wr(&mut h, HFCGCTRL, 0);
        assert_eq!(rd(&h, HFCGML), 0x11);
        assert_eq!(rd(&h, 0x12), 0);
        assert_eq!(rd(&h, HFCGCTRL), HFCGCTRL_LOCK);
        assert_eq!(h.loads, 0);
        h.on_system_reset();
        assert_eq!((rd(&h, HFCGCTRL), rd(&h, HFCGML)), (0, 0));
        wr(&mut h, HFCGML, 0x22);
        assert_eq!(rd(&h, HFCGML), 0x22);
    }

    #[test]
    fn lfcg_fields_read_back_with_widths_and_16_bit_access() {
        let mut l = NpcxCdcg::new(CdcgBlock::Lfcg);
        l.write_u16(HFRDI, 0xFFFF).unwrap();
        assert_eq!(l.read_u16(HFRDI).unwrap(), 0x0FFF);
        l.write_u16(0x04, 0xBEEF).unwrap();
        assert_eq!(l.read_u16(0x04).unwrap(), 0xBEEF);
        for offset in [0x06, 0x08, 0x0A] {
            l.write_u16(offset, 0xFFFF).unwrap();
            assert_eq!(l.read_u16(offset).unwrap(), 0x00FF, "offset {offset:#x}");
        }
        wr(&mut l, LFCGCTL2, 0xFF);
        assert_eq!(rd(&l, LFCGCTL2), 0x78);
        // LREFEN reads back; UDCP, LFLER, XTCLK_VAL read 0.
        wr(&mut l, LFCGCTL, 0xDF);
        assert_eq!(rd(&l, LFCGCTL), 1 << 2);
    }

    #[test]
    fn lfcg_lock_drops_every_write() {
        let mut l = NpcxCdcg::new(CdcgBlock::Lfcg);
        wr(&mut l, LFCGCTL2, 1 << 6);
        wr(&mut l, LFCGCTL, LFCGCTL_LFLOC);
        wr(&mut l, LFCGCTL2, 0);
        wr(&mut l, LFCGCTL, 0);
        l.write_u16(HFRDI, 0x123).unwrap();
        assert_eq!(rd(&l, LFCGCTL2), 1 << 6);
        assert_eq!(rd(&l, LFCGCTL), LFCGCTL_LFLOC);
        assert_eq!(l.read_u16(HFRDI).unwrap(), 0);
    }
}
