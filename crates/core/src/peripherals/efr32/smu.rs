// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The EFR32 Series-2 Security Management Unit — the FIRST peripheral any
//! vendor-built image touches.
//!
//! # Why this one, before EMU or the oscillators
//!
//! `SystemInit` in `system_efr32mg26.c` (simplicity_sdk `sisdk-2025.6`) has a
//! very short peripheral footprint, and it is not the clock tree:
//!
//! ```text
//! CMU->CLKEN1_SET  = CMU_CLKEN1_SMU;
//! SMU->PPUSATD0_CLR = _SMU_PPUSATD0_MASK;
//! SMU->PPUSATD1_CLR = _SMU_PPUSATD1_MASK & ~SMU_PPUSATD1_SMU;
//! SMU->PPUSATD2_CLR = _SMU_PPUSATD2_MASK & ~SMU_PPUSATD2_SEMAILBOX;
//! SMU->IF_CLR = SMU_IF_PPUSEC | SMU_IF_BMPUSEC;
//! SMU->IEN    = SMU_IEN_PPUSEC | SMU_IEN_BMPUSEC;
//! ```
//!
//! With the block unmapped that is a bus fault three instructions into the
//! startup path, before `main` — so a vendor image could not begin. Ranking
//! EMU and the oscillators first was a guess; this is what the vendor source
//! actually says.
//!
//! # ⚠️ What is modelled, and what is NOT
//!
//! The REGISTER FILE: documented reset values, `__IM` members read-only, and
//! the chip-wide SET/CLR/TGL aliases (which is how firmware writes it — every
//! line above is an alias write).
//!
//! **No protection is enforced.** On silicon these registers decide which
//! peripherals answer from the secure alias and which from the non-secure one,
//! and a violation raises `PPUFS` and an interrupt. Here they are stored and
//! read back, `PPUFS`/`BMPUFS` stay clear, and nothing is refused. That is a
//! deliberate limit, not an oversight: a twin that accepted the writes AND
//! pretended to enforce would report a security posture it does not have.
//!
//! ⚠️ It also means the peripheral ALIAS MOVE is not modelled. On silicon,
//! after `SystemInit` marks peripherals non-secure they answer at
//! `0x5000_0000` and no longer at `0x4000_0000`; this chip's descriptor maps
//! the secure alias only, and firmware built through the LabWired noradio lane
//! never runs `SystemInit`. A vendor image that does will read its peripherals
//! at the secure alias here and at the non-secure one on the die — the same
//! registers either way, so it runs, but that is worth knowing before trusting
//! an address in a trace.

use crate::SimResult;

/// One modelled register: offset, reset value, and whether firmware may write
/// it. Every row is a line of `SMU_TypeDef`; a register absent from this table
/// is a reserved word in that struct.
struct RegDef {
    offset: u64,
    reset: u32,
    /// `false` for the `__IM` (read-only) members.
    writable: bool,
}

const fn ro(offset: u64, reset: u32) -> RegDef {
    RegDef {
        offset,
        reset,
        writable: false,
    }
}

const fn rw(offset: u64, reset: u32) -> RegDef {
    RegDef {
        offset,
        reset,
        writable: true,
    }
}

/// The SMU register map, in `SMU_TypeDef` order.
///
/// Offsets and reset values were WALKED FROM THE HEADER by script rather than
/// read off by hand — `RESERVED3[53]` alone is 212 bytes, and an off-by-one
/// there puts `PPUFS` on top of a writable register. Two anchors a reader can
/// check by eye: `IPVERSION` is 7 like every other Series-2 block's, and
/// `PPUSATD0/1` reset to all-ones ("everything secure") while `PPUSATD2` is
/// 0x0F because only four peripherals live in that word.
const REGS: &[RegDef] = &[
    ro(0x000, 0x0000_0007), // IPVERSION
    ro(0x004, 0x0000_0000), // STATUS
    rw(0x008, 0x0000_0000), // LOCK
    rw(0x00C, 0x0000_0000), // IF
    rw(0x010, 0x0000_0000), // IEN
    rw(0x020, 0x0000_0000), // M33CTRL
    rw(0x040, 0xFFFF_FFFF), // PPUPATD0
    rw(0x044, 0xFFFF_FFFF), // PPUPATD1
    rw(0x048, 0x0000_000F), // PPUPATD2
    rw(0x060, 0xFFFF_FFFF), // PPUSATD0
    rw(0x064, 0xFFFF_FFFF), // PPUSATD1
    rw(0x068, 0x0000_000F), // PPUSATD2
    ro(0x140, 0x0000_0000), // PPUFS
    rw(0x150, 0x0000_01FF), // BMPUPATD0
    rw(0x170, 0x0000_01FF), // BMPUSATD0
    ro(0x250, 0x0000_0000), // BMPUFS
    ro(0x254, 0x0000_0000), // BMPUFSADDR
    rw(0x260, 0x0000_0000), // ESAURTYPES0
    rw(0x264, 0x0000_0000), // ESAURTYPES1
    rw(0x270, 0x0A00_0000), // ESAUMRB01
    rw(0x274, 0x0C00_0000), // ESAUMRB12
    rw(0x280, 0x0200_0000), // ESAUMRB45
    rw(0x284, 0x0400_0000), // ESAUMRB56
];

/// The Security Management Unit.
#[derive(Debug, serde::Serialize)]
pub struct Efr32s2Smu {
    /// Live value per row of [`REGS`], same order.
    values: Vec<u32>,
}

impl Default for Efr32s2Smu {
    fn default() -> Self {
        Self::new()
    }
}

impl Efr32s2Smu {
    pub fn new() -> Self {
        Self {
            values: REGS.iter().map(|r| r.reset).collect(),
        }
    }

    fn index(offset: u64) -> Option<usize> {
        REGS.iter().position(|r| r.offset == offset)
    }

    fn read_word(&self, offset: u64) -> u32 {
        Self::index(offset).map_or(0, |i| self.values[i])
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let Some(i) = Self::index(offset) else {
            crate::census_reg!("efr32:Efr32s2Smu", offset, "write");
            return;
        };
        if REGS[i].writable {
            self.values[i] = value;
        }
    }
}

impl crate::Peripheral for Efr32s2Smu {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset % 4) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg = offset & !3;
        let shift = (offset % 4) * 8;
        let merged = (self.read_word(reg) & !(0xFFu32 << shift)) | (u32::from(value) << shift);
        self.write_word(reg, merged);
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resets_to_the_header_values() {
        let smu = Efr32s2Smu::new();
        assert_eq!(smu.read_word(0x000), 7, "IPVERSION");
        assert_eq!(
            smu.read_word(0x060),
            0xFFFF_FFFF,
            "PPUSATD0 boots all-secure",
        );
        assert_eq!(
            smu.read_word(0x068),
            0x0000_000F,
            "PPUSATD2 has only four peripherals in its word",
        );
        assert_eq!(smu.read_word(0x270), 0x0A00_0000, "ESAUMRB01");
    }

    /// `__IM` members ignore writes, exactly as silicon does. A fault-status
    /// register firmware could set would let a test manufacture a violation
    /// this model does not detect.
    #[test]
    fn the_read_only_registers_ignore_writes() {
        let mut smu = Efr32s2Smu::new();
        for off in [0x000u64, 0x004, 0x140, 0x250, 0x254] {
            smu.write_word(off, 0xDEAD_BEEF);
            assert_ne!(smu.read_word(off), 0xDEAD_BEEF, "offset {off:#x}");
        }
    }

    /// The exact sequence `SystemInit` runs. Not a paraphrase — this is the
    /// reason the block is mapped at all, so it is asserted as written.
    #[test]
    fn system_inits_own_sequence_lands() {
        let mut smu = Efr32s2Smu::new();
        // The three `_CLR` writes are alias writes on silicon; the alias window
        // turns them into a masked clear before reaching this model, so what
        // arrives here is the resulting value.
        smu.write_word(0x060, 0); // PPUSATD0_CLR = MASK
        smu.write_word(0x064, 1 << 8); // PPUSATD1_CLR = MASK & ~SMU
        smu.write_word(0x068, 1 << 2); // PPUSATD2_CLR = MASK & ~SEMAILBOX
        smu.write_word(0x00C, 0); // IF_CLR
        smu.write_word(0x010, 0b11); // IEN = PPUSEC | BMPUSEC

        assert_eq!(smu.read_word(0x060), 0);
        assert_eq!(smu.read_word(0x064), 1 << 8);
        assert_eq!(smu.read_word(0x068), 1 << 2);
        assert_eq!(smu.read_word(0x010), 0b11);
    }

    /// ⚠️ States the LIMIT as a test, so nobody reads the model as enforcing.
    /// Marking a peripheral non-secure changes nothing about who may reach it
    /// here, and `PPUFS` never latches.
    #[test]
    fn attribution_is_stored_and_enforces_nothing() {
        let mut smu = Efr32s2Smu::new();
        smu.write_word(0x060, 0); // every peripheral non-secure
        assert_eq!(
            smu.read_word(0x140),
            0,
            "PPUFS stays clear: this model detects no violation, because it \
             refuses no access",
        );
    }
}
