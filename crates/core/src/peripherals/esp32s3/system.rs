// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 `SYSTEM` clock/reset control block (`DR_REG_SYSTEM_BASE`,
//! `0x600C_0000`).
//!
//! This is the faithful register model for the SYSTEM peripheral's
//! clock-gating / reset / PVT register file. It replaces the generic
//! round-tripping `SystemStub` for the 42 architected SYSTEM registers with a
//! fixed register file: each register is seeded to its silicon reset value and
//! a write applies the register's writable-bit mask
//! (`stored = (stored & !wmask) | (value & wmask)`). Read-only registers ignore
//! writes; unmapped offsets in the block read as zero and ignore writes (NOT
//! round-trip — so the behavioral coverage probe's catch-all baseline does not
//! flag this peripheral as generic storage).
//!
//! ## Layered MMIO — boot-critical offsets are served elsewhere
//!
//! The `0x600C_0000` region is layered. Three offset ranges are owned by
//! dedicated boot-critical peripherals that register and route ahead of this
//! model, and this model must NEVER shadow them at runtime:
//!
//!   * `0x000` / `0x004` — `CORE_1_CONTROL_0/1` → [`core1_control`]: the
//!     `RESETING` 1→0 edge boots the APP_CPU.
//!   * `0x030`..`0x03C` — `CPU_INTR_FROM_CPU_0..3` → [`crosscore_ipi`]: the
//!     SMP cross-core doorbell (asserts level sources 79/80).
//!
//! This model still *holds* faithful reset values for those six offsets so that
//! the behavioral probe (which resolves the SYSTEM window to this peripheral)
//! reads sensible values, but the bus router is wired so those six offsets are
//! served by `core1_control` / `crosscore_ipi` at runtime. See the routing
//! tests in `system::xtensa`.
//!
//! [`core1_control`]: crate::peripherals::esp32s3::core1_control
//! [`crosscore_ipi`]: crate::peripherals::esp32s3::crosscore_ipi
//!
//! ## Register table (ESP32-S3 TRM §15 System Registers; reset values marked ✓
//! are HW-validated against a physical board read at reset-halt)
//!
//! | Offset | Name                                    | Reset        | Write mask   |
//! |-------:|-----------------------------------------|--------------|--------------|
//! | 0x000  | CORE_1_CONTROL_0                        | 0x0000_0004  | 0x0000_0007  |
//! | 0x004  | CORE_1_CONTROL_1                        | 0x0000_0000  | 0xFFFF_FFFF  |
//! | 0x008  | CPU_PERI_CLK_EN                         | 0x0000_0000  | 0x0000_00C0  |
//! | 0x00C  | CPU_PERI_RST_EN                         | 0x0000_00C0  | 0x0000_00C0  |
//! | 0x010  | CPU_PER_CONF                          ✓ | 0x0000_000C  | 0x0000_00FF  |
//! | 0x014  | MEM_PD_MASK                             | 0x0000_0001  | 0x0000_0001  |
//! | 0x018  | PERIP_CLK_EN0                         ✓ | 0xF9C1_E06F  | 0xFFFF_FFFF  |
//! | 0x01C  | PERIP_CLK_EN1                         ✓ | 0x0000_0600  | 0x0000_07FF  |
//! | 0x020  | PERIP_RST_EN0                           | 0x0000_0000  | 0xFFFF_FFFF  |
//! | 0x024  | PERIP_RST_EN1                         ✓ | 0x0000_01FE  | 0x0000_07FF  |
//! | 0x028  | BT_LPCK_DIV_INT                         | 0x0000_00FF  | 0x0000_0FFF  |
//! | 0x02C  | BT_LPCK_DIV_FRAC                        | 0x0200_1001  | 0x1FFF_FFFF  |
//! | 0x030  | CPU_INTR_FROM_CPU_0                     | 0x0000_0000  | 0x0000_0001  |
//! | 0x034  | CPU_INTR_FROM_CPU_1                     | 0x0000_0000  | 0x0000_0001  |
//! | 0x038  | CPU_INTR_FROM_CPU_2                     | 0x0000_0000  | 0x0000_0001  |
//! | 0x03C  | CPU_INTR_FROM_CPU_3                     | 0x0000_0000  | 0x0000_0001  |
//! | 0x040  | RSA_PD_CTRL                             | 0x0000_0001  | 0x0000_0007  |
//! | 0x044  | EDMA_CTRL                               | 0x0000_0001  | 0x0000_0003  |
//! | 0x048  | CACHE_CONTROL                           | 0x0000_0005  | 0x0000_000F  |
//! | 0x04C  | EXTERNAL_DEVICE_ENCRYPT_DECRYPT_CONTROL | 0x0000_0000  | 0x0000_000F  |
//! | 0x050  | RTC_FASTMEM_CONFIG                    ✓ | 0x7FF0_0000  | 0x7FFF_FF00  |
//! | 0x054  | RTC_FASTMEM_CRC                  (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x058  | REDUNDANT_ECO_CTRL                      | 0x0000_0000  | 0x0000_0001  |
//! | 0x05C  | CLOCK_GATE                              | 0x0000_0001  | 0x0000_0001  |
//! | 0x060  | SYSCLK_CONF                             | 0x0000_0001  | 0x0000_0FFF  |
//! | 0x064  | MEM_PVT                                 | 0x0000_0003  | 0x00C0_003F  |
//! | 0x068  | COMB_PVT_LVT_CONF                       | 0x0000_0003  | 0x0000_007F  |
//! | 0x06C  | COMB_PVT_NVT_CONF                       | 0x0000_0003  | 0x0000_007F  |
//! | 0x070  | COMB_PVT_HVT_CONF                       | 0x0000_0003  | 0x0000_007F  |
//! | 0x074  | COMB_PVT_ERR_LVT_SITE0           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x078  | COMB_PVT_ERR_NVT_SITE0           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x07C  | COMB_PVT_ERR_HVT_SITE0           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x080  | COMB_PVT_ERR_LVT_SITE1           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x084  | COMB_PVT_ERR_NVT_SITE1           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x088  | COMB_PVT_ERR_HVT_SITE1           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x08C  | COMB_PVT_ERR_LVT_SITE2           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x090  | COMB_PVT_ERR_NVT_SITE2           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x094  | COMB_PVT_ERR_HVT_SITE2           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x098  | COMB_PVT_ERR_LVT_SITE3           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x09C  | COMB_PVT_ERR_NVT_SITE3           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0x0A0  | COMB_PVT_ERR_HVT_SITE3           (RO)    | 0x0000_0000  | 0x0000_0000  |
//! | 0xFFC  | DATE                                 ✓ | 0x0210_1220  | 0x0FFF_FFFF  |

use crate::peripherals::esp_xtensa_common::rom_thunks::APPCPU_RESET_RELEASED;
use crate::{Peripheral, SimResult};

/// The window this model covers: `[0x600C_0000, 0x600C_1000)`. DATE at 0xFFC is
/// the last architected register.
pub const SIZE: u64 = 0x1000;

/// `CORE_1_CONTROL_0.RESETING` (bit 2): the 1→0 edge releases the APP_CPU.
const RESETING: u32 = 1 << 2;

/// One architected SYSTEM register: its offset, silicon reset value, and the
/// mask of writable bits (0 ⇒ read-only).
struct RegDef {
    offset: u64,
    reset: u32,
    wmask: u32,
}

/// The 42 architected SYSTEM registers, in offset order.
const REGS: &[RegDef] = &[
    RegDef {
        offset: 0x000,
        reset: 0x0000_0004,
        wmask: 0x0000_0007,
    }, // CORE_1_CONTROL_0
    RegDef {
        offset: 0x004,
        reset: 0x0000_0000,
        wmask: 0xFFFF_FFFF,
    }, // CORE_1_CONTROL_1
    RegDef {
        offset: 0x008,
        reset: 0x0000_0000,
        wmask: 0x0000_00C0,
    }, // CPU_PERI_CLK_EN
    RegDef {
        offset: 0x00C,
        reset: 0x0000_00C0,
        wmask: 0x0000_00C0,
    }, // CPU_PERI_RST_EN
    RegDef {
        offset: 0x010,
        reset: 0x0000_000C,
        wmask: 0x0000_00FF,
    }, // CPU_PER_CONF ✓
    RegDef {
        offset: 0x014,
        reset: 0x0000_0001,
        wmask: 0x0000_0001,
    }, // MEM_PD_MASK
    RegDef {
        offset: 0x018,
        reset: 0xF9C1_E06F,
        wmask: 0xFFFF_FFFF,
    }, // PERIP_CLK_EN0 ✓
    RegDef {
        offset: 0x01C,
        reset: 0x0000_0600,
        wmask: 0x0000_07FF,
    }, // PERIP_CLK_EN1 ✓
    RegDef {
        offset: 0x020,
        reset: 0x0000_0000,
        wmask: 0xFFFF_FFFF,
    }, // PERIP_RST_EN0
    RegDef {
        offset: 0x024,
        reset: 0x0000_01FE,
        wmask: 0x0000_07FF,
    }, // PERIP_RST_EN1 ✓
    RegDef {
        offset: 0x028,
        reset: 0x0000_00FF,
        wmask: 0x0000_0FFF,
    }, // BT_LPCK_DIV_INT
    RegDef {
        offset: 0x02C,
        reset: 0x0200_1001,
        wmask: 0x1FFF_FFFF,
    }, // BT_LPCK_DIV_FRAC
    RegDef {
        offset: 0x030,
        reset: 0x0000_0000,
        wmask: 0x0000_0001,
    }, // CPU_INTR_FROM_CPU_0
    RegDef {
        offset: 0x034,
        reset: 0x0000_0000,
        wmask: 0x0000_0001,
    }, // CPU_INTR_FROM_CPU_1
    RegDef {
        offset: 0x038,
        reset: 0x0000_0000,
        wmask: 0x0000_0001,
    }, // CPU_INTR_FROM_CPU_2
    RegDef {
        offset: 0x03C,
        reset: 0x0000_0000,
        wmask: 0x0000_0001,
    }, // CPU_INTR_FROM_CPU_3
    RegDef {
        offset: 0x040,
        reset: 0x0000_0001,
        wmask: 0x0000_0007,
    }, // RSA_PD_CTRL
    RegDef {
        offset: 0x044,
        reset: 0x0000_0001,
        wmask: 0x0000_0003,
    }, // EDMA_CTRL
    RegDef {
        offset: 0x048,
        reset: 0x0000_0005,
        wmask: 0x0000_000F,
    }, // CACHE_CONTROL
    RegDef {
        offset: 0x04C,
        reset: 0x0000_0000,
        wmask: 0x0000_000F,
    }, // EXT_DEV_ENC_DEC_CONTROL
    RegDef {
        offset: 0x050,
        reset: 0x7FF0_0000,
        wmask: 0x7FFF_FF00,
    }, // RTC_FASTMEM_CONFIG ✓
    RegDef {
        offset: 0x054,
        reset: 0x0000_0000,
        wmask: 0x0000_0000,
    }, // RTC_FASTMEM_CRC (RO)
    RegDef {
        offset: 0x058,
        reset: 0x0000_0000,
        wmask: 0x0000_0001,
    }, // REDUNDANT_ECO_CTRL
    RegDef {
        offset: 0x05C,
        reset: 0x0000_0001,
        wmask: 0x0000_0001,
    }, // CLOCK_GATE
    RegDef {
        offset: 0x060,
        reset: 0x0000_0001,
        wmask: 0x0000_0FFF,
    }, // SYSCLK_CONF
    RegDef {
        offset: 0x064,
        reset: 0x0000_0003,
        wmask: 0x00C0_003F,
    }, // MEM_PVT
    RegDef {
        offset: 0x068,
        reset: 0x0000_0003,
        wmask: 0x0000_007F,
    }, // COMB_PVT_LVT_CONF
    RegDef {
        offset: 0x06C,
        reset: 0x0000_0003,
        wmask: 0x0000_007F,
    }, // COMB_PVT_NVT_CONF
    RegDef {
        offset: 0x070,
        reset: 0x0000_0003,
        wmask: 0x0000_007F,
    }, // COMB_PVT_HVT_CONF
    RegDef {
        offset: 0x074,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_LVT_SITE0 (RO)
    RegDef {
        offset: 0x078,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_NVT_SITE0 (RO)
    RegDef {
        offset: 0x07C,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_HVT_SITE0 (RO)
    RegDef {
        offset: 0x080,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_LVT_SITE1 (RO)
    RegDef {
        offset: 0x084,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_NVT_SITE1 (RO)
    RegDef {
        offset: 0x088,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_HVT_SITE1 (RO)
    RegDef {
        offset: 0x08C,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_LVT_SITE2 (RO)
    RegDef {
        offset: 0x090,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_NVT_SITE2 (RO)
    RegDef {
        offset: 0x094,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_HVT_SITE2 (RO)
    RegDef {
        offset: 0x098,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_LVT_SITE3 (RO)
    RegDef {
        offset: 0x09C,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_NVT_SITE3 (RO)
    RegDef {
        offset: 0x0A0,
        reset: 0,
        wmask: 0,
    }, // COMB_PVT_ERR_HVT_SITE3 (RO)
    RegDef {
        offset: 0xFFC,
        reset: 0x0210_1220,
        wmask: 0x0FFF_FFFF,
    }, // DATE ✓
];

/// Faithful SYSTEM clock/reset register model.
///
/// Because the 0x600C_0030..0x600C_0040 crosscore-IPI doorbell must keep being
/// served by [`crosscore_ipi`](crate::peripherals::esp32s3::crosscore_ipi) — a
/// stateful boot-critical peripheral whose `tick()` re-asserts level sources —
/// this model is registered as TWO non-overlapping windows that straddle that
/// hole, so it can never shadow the doorbell (the bus's resolution hint makes
/// any overlapping window order-dependent, which would risk swallowing a
/// doorbell write at runtime). Each registration carries the absolute base
/// address of its window via [`Self::with_window_base`] so the bus's
/// base-relative offsets are translated back to the architected (absolute)
/// register offsets used by [`REGS`].
#[derive(Debug)]
pub struct Esp32s3System {
    /// Stored value per architected register, index-parallel to [`REGS`].
    stored: Vec<u32>,
    /// Absolute offset (from the SYSTEM block base 0x600C_0000) of this
    /// window's start. Added to incoming base-relative offsets so register
    /// lookup is always in architected-offset space.
    window_base: u64,
}

impl Default for Esp32s3System {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s3System {
    /// A model whose window starts at the SYSTEM block base (offset 0).
    pub fn new() -> Self {
        Self::with_window_base(0)
    }

    /// A model whose window starts at `window_base` offset from 0x600C_0000.
    /// Used for the second (0x040..0x1000) registration so its base-relative
    /// offsets map back onto the absolute architected register offsets.
    pub fn with_window_base(window_base: u64) -> Self {
        Self {
            stored: REGS.iter().map(|r| r.reset).collect(),
            window_base,
        }
    }

    /// Index into [`REGS`] / [`Self::stored`] for a word offset, if architected.
    #[inline]
    fn index(offset: u64) -> Option<usize> {
        let word = offset & !3;
        REGS.iter().position(|r| r.offset == word)
    }

    /// The full 32-bit value at `word` offset (reset for unmapped → 0).
    #[inline]
    fn word_value(&self, offset: u64) -> u32 {
        Self::index(offset).map(|i| self.stored[i]).unwrap_or(0)
    }
}

impl Peripheral for Esp32s3System {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let abs = offset + self.window_base;
        let word = self.word_value(abs);
        Ok(((word >> ((abs & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        // Compose the byte into the current word, then re-run the masked
        // 32-bit write so the write mask is honored on byte-granular writes.
        let abs = offset + self.window_base;
        let word_off = abs & !3;
        let byte_off = (abs & 3) * 8;
        let cur = self.word_value(word_off);
        let new = (cur & !(0xFFu32 << byte_off)) | ((value as u32) << byte_off);
        // write_u32 re-adds window_base, so pass the base-relative word offset.
        self.write_u32(word_off - self.window_base, new)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.word_value(offset + self.window_base))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = (offset + self.window_base) & !3;
        if let Some(i) = Self::index(word_off) {
            let wmask = REGS[i].wmask;
            let prev = self.stored[i];
            let new = (prev & !wmask) | (value & wmask);
            self.stored[i] = new;
            // Mirror core1_control's faithful side effect for CORE_1_CONTROL_0:
            // the RESETING (bit 2) 1→0 edge releases the APP_CPU. This is only
            // observed when this model serves 0x000 directly (it normally does
            // not — core1_control wins that offset at runtime — but keeping the
            // edge here means the model stays faithful if routing ever changes).
            if word_off == 0x000 && (prev & RESETING) != 0 && (new & RESETING) == 0 {
                APPCPU_RESET_RELEASED.with(|s| s.set(true));
            }
        }
        // Unmapped offsets: read-as-zero / ignore-write (no round-trip).
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Look up a register's reset/wmask by offset for assertions.
    fn def(offset: u64) -> &'static RegDef {
        REGS.iter().find(|r| r.offset == offset).expect("offset")
    }

    #[test]
    fn exactly_42_registers() {
        assert_eq!(
            REGS.len(),
            42,
            "the SYSTEM block has 42 architected registers"
        );
    }

    #[test]
    fn hw_validated_reset_values() {
        let s = Esp32s3System::new();
        // The ✓ HW-validated reset values read at reset-halt from the board.
        assert_eq!(s.read_u32(0x010).unwrap(), 0x0000_000C, "CPU_PER_CONF");
        assert_eq!(s.read_u32(0x018).unwrap(), 0xF9C1_E06F, "PERIP_CLK_EN0");
        assert_eq!(s.read_u32(0x01C).unwrap(), 0x0000_0600, "PERIP_CLK_EN1");
        assert_eq!(s.read_u32(0x024).unwrap(), 0x0000_01FE, "PERIP_RST_EN1");
        assert_eq!(
            s.read_u32(0x050).unwrap(),
            0x7FF0_0000,
            "RTC_FASTMEM_CONFIG"
        );
        assert_eq!(s.read_u32(0xFFC).unwrap(), 0x0210_1220, "DATE");
    }

    #[test]
    fn date_is_constant_read_only_low_bits() {
        // DATE has wmask 0x0FFFFFFF; the top nibble is read-only. A full-ones
        // write changes only the writable low 28 bits.
        let mut s = Esp32s3System::new();
        s.write_u32(0xFFC, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0xFFC).unwrap(), 0x0FFF_FFFF);
    }

    #[test]
    fn write_mask_round_trips_writable_bits_only() {
        let mut s = Esp32s3System::new();
        // CPU_PERI_CLK_EN: only bits 6,7 (0xC0) writable, reset 0.
        s.write_u32(0x008, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x008).unwrap(), 0x0000_00C0);
        s.write_u32(0x008, 0x0000_0000).unwrap();
        assert_eq!(s.read_u32(0x008).unwrap(), 0x0000_0000);

        // PERIP_CLK_EN0: fully writable.
        s.write_u32(0x018, 0x1234_5678).unwrap();
        assert_eq!(s.read_u32(0x018).unwrap(), 0x1234_5678);

        // MEM_PVT: split mask 0x00C0_003F.
        s.write_u32(0x064, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(0x064).unwrap(), 0x00C0_003F);
    }

    #[test]
    fn read_only_registers_ignore_writes() {
        let mut s = Esp32s3System::new();
        // RTC_FASTMEM_CRC (0x054) and a COMB_PVT_ERR site (0x074) are RO/zero.
        s.write_u32(0x054, 0xDEAD_BEEF).unwrap();
        assert_eq!(s.read_u32(0x054).unwrap(), 0, "RTC_FASTMEM_CRC stays 0");
        s.write_u32(0x074, 0xDEAD_BEEF).unwrap();
        assert_eq!(
            s.read_u32(0x074).unwrap(),
            0,
            "COMB_PVT_ERR_LVT_SITE0 stays 0"
        );
        assert_eq!(def(0x054).wmask, 0);
        assert_eq!(def(0x074).wmask, 0);
    }

    #[test]
    fn unmapped_offset_reads_zero_and_ignores_writes() {
        let mut s = Esp32s3System::new();
        // 0x100 is the first offset past the PVT block and below DATE — unmapped.
        assert_eq!(s.read_u32(0x100).unwrap(), 0);
        s.write_u32(0x100, 0xFFFF_FFFF).unwrap();
        assert_eq!(
            s.read_u32(0x100).unwrap(),
            0,
            "unmapped does NOT round-trip"
        );
        // 0x500 likewise.
        s.write_u32(0x500, 0xA5A5_A5A5).unwrap();
        assert_eq!(s.read_u32(0x500).unwrap(), 0);
    }

    #[test]
    fn byte_write_honors_mask() {
        let mut s = Esp32s3System::new();
        // SYSCLK_CONF wmask 0x0FFF: write 0xFF to byte 1 (bits 8..15) → only
        // bits 8..11 stick (mask 0x0F00).
        s.write(0x061, 0xFF).unwrap();
        assert_eq!(s.read_u32(0x060).unwrap(), 0x0000_0F01);
    }

    #[test]
    fn reseting_falling_edge_releases_appcpu() {
        APPCPU_RESET_RELEASED.with(|s| s.set(false));
        let mut s = Esp32s3System::new();
        // CORE_1_CONTROL_0 resets to 0x4 (RESETING set). Clear it → release.
        assert_eq!(s.read_u32(0x000).unwrap() & RESETING, RESETING);
        s.write_u32(0x000, 0x0000_0002).unwrap(); // clear RESETING
        assert!(APPCPU_RESET_RELEASED.with(|s| s.get()), "released on 1->0");
    }
}
