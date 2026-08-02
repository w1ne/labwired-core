// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! EFUSE peripheral for ESP32-classic.
//!
//! Per ESP32 TRM v5.0 §20. The EFUSE controller sits at base `0x3FF5_A000`
//! and exposes four 256-bit blocks (BLK0..BLK3) plus a small control
//! window (CONF/CMD/INT_*/DAC_CONF/DEC_STATUS). BLK0 carries the
//! per-die system-wide configuration (MAC address, chip revision and
//! package type, flash-encryption / secure-boot enable flags, etc.).
//!
//! ## Why this peripheral exists
//!
//! ESP32 BROM reads BLK0 during reset-handler init to decide which code
//! path to take (e.g. rev3 enables certain workaround branches). When
//! the eFuse window read back as garbage/zero our simulator hit an
//! ILL.N at PC 0x4000fdd3 on cycle 32 of cold boot because the reset
//! handler couldn't reconcile the chip-revision value with the BROM
//! revision check, so it fell through to an unreachable path and the
//! `panic_helper` trampoline tripped ILL.N. Returning a coherent
//! rev3 + non-zero MAC value lets the reset handler progress.
//!
//! ## Default values
//!
//! For a stock ESP32-WROOM-32 rev3 part (per ESP32 TRM v5.0 §20.3.1.3):
//!   * `BLK0_RDATA3` bits[11:8] = `0x3` (chip_revision = rev3).
//!     Bits[15:12] (chip_ver_pkg) = 0 (WROOM-32 default).
//!   * `BLK0_RDATA1` + `BLK0_RDATA2` = MAC `24:6F:28:00:00:01`
//!     (placeholder; BROM only checks non-zero during late init).
//!   * All other RDATA / WDATA / control regs default to 0
//!     (no flash encryption, no secure boot programmed).
//!
//! Sparse storage (HashMap) keeps the model compact — almost the whole
//! 256-byte window is unused on a first-boot probe.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::cell::Cell;
use std::collections::HashMap;

// ── Register offsets (per ESP32 TRM v5.0 §20.4 and ESP-IDF
// `soc/esp32/include/soc/efuse_reg.h`) ────────────────────────────────────

/// BLK0_RDATA0 — write-disable / read-disable / flash-crypt-cnt flags.
pub const EFUSE_BLK0_RDATA0_OFFSET: u64 = 0x000;
/// BLK0_RDATA1 — low 32 bits of the factory MAC address.
pub const EFUSE_BLK0_RDATA1_OFFSET: u64 = 0x004;
/// BLK0_RDATA2 — high 16 bits of the factory MAC (bits[15:0]) + CRC etc.
pub const EFUSE_BLK0_RDATA2_OFFSET: u64 = 0x008;
/// BLK0_RDATA3 — bits[15:12]=chip_ver_pkg, bits[11:8]=chip_revision.
pub const EFUSE_BLK0_RDATA3_OFFSET: u64 = 0x00C;
/// BLK0_RDATA4..7 — additional system config (ADC calibration, etc.).
pub const EFUSE_BLK0_RDATA4_OFFSET: u64 = 0x010;
pub const EFUSE_BLK0_RDATA5_OFFSET: u64 = 0x014;
pub const EFUSE_BLK0_RDATA6_OFFSET: u64 = 0x018;
pub const EFUSE_BLK0_RDATA7_OFFSET: u64 = 0x01C;

/// BLK0_WDATA0..7 — write buffer for BLK0.
pub const EFUSE_BLK0_WDATA0_OFFSET: u64 = 0x01C;

/// BLK1_RDATA0..7 — flash encryption key.
pub const EFUSE_BLK1_RDATA0_OFFSET: u64 = 0x038;
/// BLK2_RDATA0..7 — secure boot key.
pub const EFUSE_BLK2_RDATA0_OFFSET: u64 = 0x058;
/// BLK3_RDATA0..7 — user-programmable.
pub const EFUSE_BLK3_RDATA0_OFFSET: u64 = 0x078;

/// Clock control (CLK_REG). Read-as-zero on stock parts.
pub const EFUSE_CLK_OFFSET: u64 = 0x0F8;
/// Magic-write enable (CONF_REG, accepts 0x5AA5 as the write-enable code).
pub const EFUSE_CONF_OFFSET: u64 = 0x0FC;
/// Read-only status (STATUS_REG).
pub const EFUSE_STATUS_OFFSET: u64 = 0x100;
/// Operation trigger (CMD_REG). Write 1 to bit 0 to trigger a read
/// operation; HW clears the bit when the operation completes. Our model
/// treats EFUSE operations as instantaneous and auto-clears the bit on
/// the *first* read after the write (see `Efuse::cmd_pending`).
pub const EFUSE_CMD_OFFSET: u64 = 0x104;
pub const EFUSE_INT_RAW_OFFSET: u64 = 0x108;
pub const EFUSE_INT_ST_OFFSET: u64 = 0x10C;
pub const EFUSE_INT_ENA_OFFSET: u64 = 0x110;
pub const EFUSE_INT_CLR_OFFSET: u64 = 0x114;
pub const EFUSE_DAC_CONF_OFFSET: u64 = 0x118;
pub const EFUSE_DEC_STATUS_OFFSET: u64 = 0x11C;

// ── Chip revision encoding (per task spec + ESP32 TRM v5.0 §20.3.1.3) ────

/// Bit position of `chip_revision` field within BLK0_RDATA3.
const CHIP_REVISION_SHIFT: u32 = 8;
/// Field width: 4 bits.
const CHIP_REVISION_MASK: u32 = 0xF;

/// Stock ESP32-WROOM-32 rev3 chip-revision value.
pub const CHIP_REVISION_REV3: u32 = 3;

// ── Placeholder MAC (Espressif OUI 24:6F:28, host-bytes 00:00:01) ────────
// BROM only checks for non-zero during late init; this value is good
// enough to keep the reset-handler path satisfied.
const PLACEHOLDER_MAC: [u8; 6] = [0x24, 0x6F, 0x28, 0x00, 0x00, 0x01];

/// EFUSE peripheral.
///
/// Word-granular sparse storage (HashMap) mirrors the RTC_CNTL peripheral
/// pattern. Reads of unset offsets return 0 (an unblown fuse). Writes
/// land in the same map: real silicon requires a separate CMD trigger
/// to actually burn a fuse, but no firmware in our smoke-test scope
/// does that.
#[derive(Debug)]
pub struct Efuse {
    /// Base MMIO address (for debugging / logs only — not used in
    /// offset math since the bus already dispatches by offset).
    base: u32,
    /// Backing word store. Indexed by 4-byte-aligned offset.
    regs: HashMap<u32, u32>,
    /// One-shot pending CMD value. Set by `write_word_32` of EFUSE_CMD;
    /// returned by the next `read_u32` and then cleared. Mirrors the
    /// hardware behaviour where CMD bit clears after the eFuse FSM
    /// finishes its read/program cycle (effectively immediately in sim).
    cmd_pending: Cell<u32>,
}

impl Default for Efuse {
    fn default() -> Self {
        Self::new()
    }
}

impl Efuse {
    /// Canonical MMIO base address on ESP32-classic.
    pub const BASE: u32 = 0x3FF5_A000;

    /// Construct a freshly-fused EFUSE block for a stock WROOM-32 rev3.
    ///
    /// Seeds:
    ///   * `BLK0_RDATA3` with `chip_revision = 3` in bits[11:8].
    ///   * `BLK0_RDATA1` / `BLK0_RDATA2` with the placeholder MAC
    ///     `24:6F:28:00:00:01`.
    ///   * Everything else defaults to 0 (no flash encryption, no
    ///     secure boot, no read/write disable bits set).
    pub fn new() -> Self {
        let mut regs = HashMap::new();

        // chip_revision = 3 in BLK0_RDATA3 bits[11:8].
        let blk0_rdata3 = (CHIP_REVISION_REV3 & CHIP_REVISION_MASK) << CHIP_REVISION_SHIFT;
        regs.insert(EFUSE_BLK0_RDATA3_OFFSET as u32, blk0_rdata3);

        // MAC low 32 bits in BLK0_RDATA1. Per ESP-IDF
        // `esp_efuse_mac_get_default()`, the MAC is laid out big-endian:
        // BLK0_RDATA1 holds MAC[5..2] (low 4 bytes of the MAC), and
        // BLK0_RDATA2 bits[15:0] hold MAC[1..0] (high 2 bytes). Encode
        // as little-endian words for the byte-addressed read path.
        let mac = PLACEHOLDER_MAC;
        // BLK0_RDATA1: MAC[5], MAC[4], MAC[3], MAC[2] (LE: byte0..3).
        let rdata1 = u32::from(mac[5])
            | (u32::from(mac[4]) << 8)
            | (u32::from(mac[3]) << 16)
            | (u32::from(mac[2]) << 24);
        // BLK0_RDATA2 low 16 bits: MAC[1], MAC[0].
        let rdata2 = u32::from(mac[1]) | (u32::from(mac[0]) << 8);
        regs.insert(EFUSE_BLK0_RDATA1_OFFSET as u32, rdata1);
        regs.insert(EFUSE_BLK0_RDATA2_OFFSET as u32, rdata2);

        Self {
            base: Self::BASE,
            regs,
            cmd_pending: Cell::new(0),
        }
    }

    /// Base MMIO address (informational).
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Read the encoded chip revision (BLK0_RDATA3 bits[11:8]).
    pub fn chip_revision(&self) -> u32 {
        let w = self
            .regs
            .get(&(EFUSE_BLK0_RDATA3_OFFSET as u32))
            .copied()
            .unwrap_or(0);
        (w >> CHIP_REVISION_SHIFT) & CHIP_REVISION_MASK
    }

    /// Read the 48-bit factory MAC as a 6-byte big-endian array.
    pub fn mac(&self) -> [u8; 6] {
        let rdata1 = self
            .regs
            .get(&(EFUSE_BLK0_RDATA1_OFFSET as u32))
            .copied()
            .unwrap_or(0);
        let rdata2 = self
            .regs
            .get(&(EFUSE_BLK0_RDATA2_OFFSET as u32))
            .copied()
            .unwrap_or(0);
        [
            ((rdata2 >> 8) & 0xFF) as u8,  // MAC[0]
            (rdata2 & 0xFF) as u8,         // MAC[1]
            ((rdata1 >> 24) & 0xFF) as u8, // MAC[2]
            ((rdata1 >> 16) & 0xFF) as u8, // MAC[3]
            ((rdata1 >> 8) & 0xFF) as u8,  // MAC[4]
            (rdata1 & 0xFF) as u8,         // MAC[5]
        ]
    }
}

impl Peripheral for Efuse {
    // Inert walk: eFuse register bank; the CMD handshake settles across the write/next-read pair, and tick() is an explicit no-op ("no time-varying state").
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        // EFUSE_CMD: one-shot pending value is returned on the next read
        // (mirrors HW where the trigger bit clears after the eFuse FSM
        // finishes — instantaneous in sim). Cleared by `read_u32` on the
        // last byte so the four byte-reads of one u32 see a coherent value.
        let word = if word_off as u64 == EFUSE_CMD_OFFSET {
            let pending = self.cmd_pending.get();
            if pending != 0 {
                pending
            } else {
                self.regs.get(&word_off).copied().unwrap_or(0)
            }
        } else {
            self.regs.get(&word_off).copied().unwrap_or(0)
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let word_off = (offset & !3) as u32;
        let word = if word_off as u64 == EFUSE_CMD_OFFSET {
            let pending = self.cmd_pending.get();
            // Clear the one-shot now so the next read returns 0 — the
            // signal HW uses to tell the BROM the eFuse op finished.
            self.cmd_pending.set(0);
            if pending != 0 {
                pending
            } else {
                self.regs.get(&word_off).copied().unwrap_or(0)
            }
        } else {
            self.regs.get(&word_off).copied().unwrap_or(0)
        };
        // The bus calls into here directly for aligned u32 loads; we
        // already short-circuited the pending one-shot above, so just
        // return the word verbatim. Going back through `read()` four
        // times would drain the Cell prematurely.
        Ok(word)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        let byte_off = (offset & 3) * 8;
        // Read-modify-write on the existing word (or 0 if unwritten).
        let mut word = self.regs.get(&word_off).copied().unwrap_or(0);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.regs.insert(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let word_off = (offset & !3) as u32;
        // EFUSE_CMD trigger: latch the value into the one-shot Cell so
        // the *next* read returns it (the BROM polls expecting non-zero,
        // then waits for the bit to clear).
        if word_off as u64 == EFUSE_CMD_OFFSET {
            self.cmd_pending.set(value);
            return Ok(());
        }
        self.regs.insert(word_off, value);
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        // EFUSE has no time-varying state in our model.
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
        }
        let snap = Snap {
            regs: self.regs.iter().map(|(k, v)| (*k, *v)).collect(),
        };
        bincode::serialize(&snap).expect("bincode serialize Efuse")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            regs: Vec<(u32, u32)>,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Efuse snapshot decode: {e}"))
        })?;
        self.regs = snap.regs.into_iter().collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_u32_at(p: &Efuse, offset: u64) -> u32 {
        let mut v = 0u32;
        for i in 0..4u64 {
            v |= (p.read(offset + i).unwrap() as u32) << (i * 8);
        }
        v
    }

    fn write_u32_at(p: &mut Efuse, offset: u64, value: u32) {
        for i in 0..4u64 {
            p.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)
                .unwrap();
        }
    }

    #[test]
    fn fresh_efuse_encodes_rev3_in_blk0_rdata3() {
        let p = Efuse::new();
        let w = read_u32_at(&p, EFUSE_BLK0_RDATA3_OFFSET);
        let rev = (w >> CHIP_REVISION_SHIFT) & CHIP_REVISION_MASK;
        assert_eq!(
            rev, CHIP_REVISION_REV3,
            "BLK0_RDATA3 bits[11:8] must encode chip_revision=3 \
             for a stock ESP32-WROOM-32 rev3 part"
        );
        assert_eq!(p.chip_revision(), CHIP_REVISION_REV3);
    }

    #[test]
    fn fresh_efuse_returns_non_zero_mac_from_blk0_rdata1_2() {
        let p = Efuse::new();
        let rdata1 = read_u32_at(&p, EFUSE_BLK0_RDATA1_OFFSET);
        let rdata2 = read_u32_at(&p, EFUSE_BLK0_RDATA2_OFFSET);
        assert_ne!(rdata1, 0, "BLK0_RDATA1 (MAC low) must be non-zero");
        assert_ne!(rdata2, 0, "BLK0_RDATA2 (MAC high) must be non-zero");

        // The placeholder MAC round-trips through the byte accessor.
        let mac = p.mac();
        assert_eq!(mac, [0x24, 0x6F, 0x28, 0x00, 0x00, 0x01]);
        // Sanity: MAC is not all-zero (the only thing BROM late-init checks).
        assert!(mac.iter().any(|&b| b != 0), "MAC must be non-zero");
    }

    #[test]
    fn other_blk0_rdata_registers_default_to_zero() {
        let p = Efuse::new();
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA0_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA4_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA5_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA6_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA7_OFFSET), 0);
    }

    #[test]
    fn blk1_blk2_blk3_default_to_zero() {
        // No flash encryption / secure boot programmed on a stock part.
        let p = Efuse::new();
        for off in (EFUSE_BLK1_RDATA0_OFFSET..EFUSE_BLK1_RDATA0_OFFSET + 32).step_by(4) {
            assert_eq!(read_u32_at(&p, off), 0, "BLK1 word @ {:#x} must be 0", off);
        }
        for off in (EFUSE_BLK2_RDATA0_OFFSET..EFUSE_BLK2_RDATA0_OFFSET + 32).step_by(4) {
            assert_eq!(read_u32_at(&p, off), 0, "BLK2 word @ {:#x} must be 0", off);
        }
        for off in (EFUSE_BLK3_RDATA0_OFFSET..EFUSE_BLK3_RDATA0_OFFSET + 32).step_by(4) {
            assert_eq!(read_u32_at(&p, off), 0, "BLK3 word @ {:#x} must be 0", off);
        }
    }

    #[test]
    fn control_registers_default_to_zero() {
        let p = Efuse::new();
        assert_eq!(read_u32_at(&p, EFUSE_CONF_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_CMD_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_INT_RAW_OFFSET), 0);
        assert_eq!(read_u32_at(&p, EFUSE_INT_ST_OFFSET), 0);
    }

    #[test]
    fn write_read_round_trip_through_any_rdata_offset() {
        let mut p = Efuse::new();
        // BLK0_RDATA0 was 0 — write some pattern and read it back.
        write_u32_at(&mut p, EFUSE_BLK0_RDATA0_OFFSET, 0xDEAD_BEEF);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_RDATA0_OFFSET), 0xDEAD_BEEF);

        // BLK1/BLK2/BLK3 all round-trip independently.
        write_u32_at(&mut p, EFUSE_BLK1_RDATA0_OFFSET, 0x1111_1111);
        write_u32_at(&mut p, EFUSE_BLK2_RDATA0_OFFSET, 0x2222_2222);
        write_u32_at(&mut p, EFUSE_BLK3_RDATA0_OFFSET, 0x3333_3333);
        assert_eq!(read_u32_at(&p, EFUSE_BLK1_RDATA0_OFFSET), 0x1111_1111);
        assert_eq!(read_u32_at(&p, EFUSE_BLK2_RDATA0_OFFSET), 0x2222_2222);
        assert_eq!(read_u32_at(&p, EFUSE_BLK3_RDATA0_OFFSET), 0x3333_3333);

        // Writes don't bleed into adjacent words.
        assert_eq!(read_u32_at(&p, EFUSE_BLK1_RDATA0_OFFSET + 4), 0);
        assert_eq!(read_u32_at(&p, EFUSE_BLK2_RDATA0_OFFSET + 4), 0);
    }

    #[test]
    fn write_preserves_unseeded_blk0_rdata3_revision_unless_overwritten() {
        // Writing one byte of BLK0_RDATA3 read-modify-writes the word.
        // The rev3 value must still be readable through the byte path
        // until firmware deliberately clobbers the whole word.
        let p = Efuse::new();
        // Byte 1 of BLK0_RDATA3 holds bits[15:8] — chip_revision (4 bits)
        // sits in the low nibble of that byte.
        assert_eq!(
            p.read(EFUSE_BLK0_RDATA3_OFFSET + 1).unwrap() & 0x0F,
            CHIP_REVISION_REV3 as u8
        );
    }

    #[test]
    fn write_buffer_offsets_round_trip() {
        // BLK0_WDATA0 is the first write-buffer slot (overlaps with
        // BLK0_RDATA7 + 0x4). Real silicon needs a CMD trigger to copy
        // it into BLK0_RDATAn, but our model just stores it verbatim.
        let mut p = Efuse::new();
        write_u32_at(&mut p, EFUSE_BLK0_WDATA0_OFFSET, 0xCAFE_F00D);
        assert_eq!(read_u32_at(&p, EFUSE_BLK0_WDATA0_OFFSET), 0xCAFE_F00D);
    }

    #[test]
    fn runtime_snapshot_round_trip_preserves_state() {
        let mut p = Efuse::new();
        write_u32_at(&mut p, EFUSE_BLK3_RDATA0_OFFSET, 0xC0FF_EE00);
        let snap = p.runtime_snapshot();

        let mut restored = Efuse::new();
        restored.restore_runtime_snapshot(&snap).unwrap();
        assert_eq!(
            read_u32_at(&restored, EFUSE_BLK3_RDATA0_OFFSET),
            0xC0FF_EE00
        );
        // Seeded values survive too.
        assert_eq!(restored.chip_revision(), CHIP_REVISION_REV3);
        assert_eq!(restored.mac(), [0x24, 0x6F, 0x28, 0x00, 0x00, 0x01]);
    }

    #[test]
    fn base_is_esp32_classic_canonical_address() {
        let p = Efuse::new();
        assert_eq!(p.base(), 0x3FF5_A000);
    }

    #[test]
    fn cmd_register_one_shot_clears_on_first_read() {
        // ESP32 BROM `_reload_efuses_and_check` writes 1 to CMD then
        // polls until the bit clears. Real silicon clears the trigger
        // bit after the eFuse FSM finishes; our model auto-clears on
        // the first read. Without this the BROM either spins forever or
        // (if CMD read-as-zero) branches to `_rtc_trigger_sw_system_reset`.
        let mut p = Efuse::new();
        // Trigger.
        p.write_u32(EFUSE_CMD_OFFSET, 1).unwrap();
        // First read must observe the trigger.
        assert_eq!(
            p.read_u32(EFUSE_CMD_OFFSET).unwrap(),
            1,
            "first CMD read after write must return the trigger value"
        );
        // Second read must return 0 — the signal the FSM is done.
        assert_eq!(
            p.read_u32(EFUSE_CMD_OFFSET).unwrap(),
            0,
            "second CMD read must return 0 (HW clears bit after op)"
        );
        // Idle reads stay at 0.
        assert_eq!(p.read_u32(EFUSE_CMD_OFFSET).unwrap(), 0);
    }
}
