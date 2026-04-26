// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Three small register stubs needed for esp-hal `init()` to complete:
//!
//! * `SystemStub`  — SYSTEM peripheral at 0x600C_0000.
//!                  SYSCLK_CONF.SOC_CLK_SEL is round-tripped (esp-hal reads it
//!                  back to know the active clock source).  Other registers
//!                  are write-accept / read-as-zero.
//! * `RtcCntlStub` — RTC_CNTL at 0x6000_8000.  Fully cosmetic for hello-world:
//!                  read-as-zero, write-accept.
//! * `EfuseStub`   — EFUSE at 0x6000_7000.  Returns canned MAC + chip-rev for
//!                  the few fields esp-hal reads at boot.

use crate::{Peripheral, SimResult};
use std::collections::HashMap;

/// SYSTEM peripheral stub.  Tracks every written word so reads return what
/// the firmware wrote (so its boot config-back-check passes).
#[derive(Debug, Default)]
pub struct SystemStub {
    words: HashMap<u64, u32>,
}

impl SystemStub {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for SystemStub {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// RTC / PMU / IO_MUX / RTC_IO / APB_CTRL peripheral stub. Round-trips
/// every written word so that read-modify-write sequences observe the
/// values esp-hal just wrote (e.g. clock-mux selectors, voltage rails,
/// GPIO mux). Unwritten offsets read as zero, except for status bits
/// that boot code busy-waits for (e.g. PLL_LOCK) which are seeded.
#[derive(Debug)]
pub struct RtcCntlStub {
    words: HashMap<u64, u32>,
}

impl RtcCntlStub {
    pub fn new() -> Self {
        let mut words = HashMap::new();
        // 0x6040 = SYSCON_DATE / RTC PLL status (within the 0x6000_8000-base
        // window, this is absolute 0x6000_e040). esp-hal's
        // request_pll_clk()/ensure_voltage_raised() polls bit 24 = PLL_LOCK
        // until it goes high; on real silicon the BBPLL asserts within a few
        // microseconds of being requested. We seed it as locked.
        words.insert(0x6040, 0x0100_0000);
        Self { words }
    }
}

impl Peripheral for RtcCntlStub {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.words.get(&word_off).copied().unwrap_or(0);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let entry = self.words.entry(word_off).or_insert(0);
        *entry &= !(0xFFu32 << byte_off);
        *entry |= (value as u32) << byte_off;
        Ok(())
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// EFUSE peripheral stub.  Returns canned MAC + chip-rev for the fields
/// esp-hal reads at boot.
///
/// Per ESP32-S3 TRM §6 (eFuse Controller), the relevant fields esp-hal touches:
///
/// | Offset | Field                        | Canned value |
/// |-------:|------------------------------|--------------|
/// |  0x044 | RD_MAC_SPI_SYS_0 (MAC[3:0])  | 0x00000002   |
/// |  0x048 | RD_MAC_SPI_SYS_1 (MAC[5:4])  | 0x00000000   |
/// |  0x05C | RD_SYS_PART1_DATA0 (chip_rev)| 0x00000000   |
///
/// The canned MAC is `02:00:00:00:00:01` (locally-administered).
#[derive(Debug, Default)]
pub struct EfuseStub;

impl EfuseStub {
    pub fn new() -> Self {
        Self
    }
}

impl Peripheral for EfuseStub {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word: u32 = match word_off {
            0x044 => 0x0000_0002, // MAC low word: 0x00 00 00 02
            0x048 => 0x0000_0000, // MAC high word: 0x00 00 00 00
            0x05C => 0x0000_0000, // chip_rev = 0
            _ => 0,
        };
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_stub_round_trips_words() {
        let mut s = SystemStub::new();
        s.write(0x10, 0xAB).unwrap();
        s.write(0x11, 0xCD).unwrap();
        s.write(0x12, 0xEF).unwrap();
        s.write(0x13, 0x12).unwrap();
        assert_eq!(s.read(0x10).unwrap(), 0xAB);
        assert_eq!(s.read(0x11).unwrap(), 0xCD);
        assert_eq!(s.read(0x13).unwrap(), 0x12);
    }

    #[test]
    fn rtc_cntl_stub_unwritten_reads_zero() {
        let s = RtcCntlStub::new();
        for off in 0..16u64 {
            assert_eq!(s.read(off).unwrap(), 0);
        }
    }

    #[test]
    fn rtc_cntl_stub_round_trips_writes() {
        let mut s = RtcCntlStub::new();
        s.write(0x40, 0x12).unwrap();
        s.write(0x41, 0x34).unwrap();
        assert_eq!(s.read(0x40).unwrap(), 0x12);
        assert_eq!(s.read(0x41).unwrap(), 0x34);
    }

    #[test]
    fn efuse_returns_canned_mac() {
        let s = EfuseStub::new();
        // MAC low byte at 0x044 = 0x02.
        assert_eq!(s.read(0x044).unwrap(), 0x02);
        assert_eq!(s.read(0x045).unwrap(), 0x00);
        // MAC high byte at 0x048 = 0x00 (no high MAC bytes).
        assert_eq!(s.read(0x048).unwrap(), 0x00);
        // chip_rev at 0x05C = 0.
        assert_eq!(s.read(0x05C).unwrap(), 0x00);
    }

    #[test]
    fn efuse_unknown_offset_reads_zero() {
        let s = EfuseStub::new();
        assert_eq!(s.read(0x100).unwrap(), 0);
    }
}
