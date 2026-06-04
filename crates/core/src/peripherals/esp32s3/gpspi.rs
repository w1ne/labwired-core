// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 GP-SPI master controller (SPI2 / FSPI and SPI3) — digital twin.
//!
//! This is the *general-purpose* SPI master the firmware drives via the
//! `spi_master` / `esp_lcd` / Arduino `SPI` drivers — DISTINCT from the flash
//! controller modeled in `spi_mem_flash.rs` (SPIMEM1 @ `0x6000_2000`). One
//! `Esp32s3Spi` instance models one controller; the parent registers two:
//!   * SPI2 / FSPI @ `0x6002_4000`, intr-matrix source `ETS_SPI2_INTR_SOURCE` = 21
//!   * SPI3        @ `0x6002_5000`, intr-matrix source `ETS_SPI3_INTR_SOURCE` = 22
//!
//! (sources verified from `soc/esp32s3/include/soc/interrupts.h`: the enum
//! anchors `ETS_GPIO_INTR_SOURCE = 16`, then GPIO_NMI=17, GPIO2=18, GPIO_NMI2=19,
//! SPI1=20, SPI2=21, SPI3=22, with `ETS_LCD_CAM_INTR_SOURCE = 24` confirming the
//! count.)
//!
//! ## Register layout (offsets from base, `soc/esp32s3/register/soc/spi_reg.h`)
//!
//! | offset | reg          | behavior                                            |
//! |--------|--------------|-----------------------------------------------------|
//! | 0x00   | SPI_CMD      | bit24 = `SPI_USR` start; set → launch, auto-clears   |
//! | 0x04   | SPI_ADDR     | address phase value (round-trip)                    |
//! | 0x08   | SPI_CTRL     | bit/byte order, dummy/fast-read mode (round-trip)   |
//! | 0x0C   | SPI_CLOCK    | clock divider config (round-trip)                   |
//! | 0x10   | SPI_USER     | phase enables (CMD/ADDR/DUMMY/MOSI/MISO) (round-trip)|
//! | 0x14   | SPI_USER1    | addr/dummy bit lengths (round-trip)                 |
//! | 0x18   | SPI_USER2    | command opcode + bitlen (round-trip)                |
//! | 0x1C   | SPI_MS_DLEN  | `SPI_MS_DATA_BITLEN`[17:0] = transfer bits-1        |
//! | 0x20   | SPI_MISC     | CS / misc config (round-trip)                       |
//! | 0x30   | SPI_DMA_CONF | DMA config (round-trip)                             |
//! | 0x34   | SPI_DMA_INT_ENA | interrupt enable mask                            |
//! | 0x38   | SPI_DMA_INT_CLR | W1C — clears latched raw bits                     |
//! | 0x3C   | SPI_DMA_INT_RAW | latched raw interrupt bits                         |
//! | 0x40   | SPI_DMA_INT_ST  | INT_RAW & INT_ENA (read-only)                     |
//! | 0x98   | SPI_W0       | data buffer word 0 (MOSI out / MISO in)             |
//! | …      | …            | 16 words W0..W15                                    |
//! | 0xD4   | SPI_W15      | data buffer word 15                                 |
//!
//! Note: the GP-SPI controller has a *single* `SPI_MS_DLEN` (0x1C) — there are
//! no separate MISO_DLEN / MOSI_DLEN registers as on SPIMEM. All transaction
//! interrupts (including `SPI_TRANS_DONE`, bit 12) live in the `SPI_DMA_INT_*`
//! register block (0x34/0x38/0x3C/0x40), per the header.
//!
//! ## Transaction model
//!
//! A master programs the phases (USER/USER1/USER2), the bit length
//! (MS_DLEN), and the MOSI payload into `W0..W15`, then sets `SPI_CMD.USR`
//! (bit 24) to launch. Firmware then busy-polls `!(CMD & USR)` for completion.
//! We complete IMMEDIATELY: clear the `USR` bit (so the poll exits) and latch
//! `SPI_TRANS_DONE` in `SPI_DMA_INT_RAW`. With no device attached in scope,
//! the faithful "no device on the bus" behavior is an idle (pulled-high) MISO
//! line, so we fill the MISO data region of `W0..W15` with `0xFF` per byte —
//! exactly the all-ones a real controller shifts in when MISO floats high.
//! The MISO region length is taken from `SPI_MS_DLEN` (`SPI_MS_DATA_BITLEN`+1
//! bits, rounded up to whole bytes, capped at the 64-byte W buffer).
//!
//! `tick()` emits the controller's intr-matrix source while `INT_ST != 0`,
//! mirroring the UART/systimer pattern; the bus routes it through the per-core
//! interrupt matrix.

use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

const CMD: u64 = 0x00;
/// `SPI_ADDR` (0x04) — address phase value, pure round-trip.
#[allow(dead_code)]
const ADDR: u64 = 0x04;
const CTRL: u64 = 0x08;
const CLOCK: u64 = 0x0C;
const USER: u64 = 0x10;
/// `SPI_USER1` (0x14) — addr/dummy bit lengths, pure round-trip.
#[allow(dead_code)]
const USER1: u64 = 0x14;
/// `SPI_USER2` (0x18) — command opcode + bitlen, pure round-trip.
#[allow(dead_code)]
const USER2: u64 = 0x18;
const MS_DLEN: u64 = 0x1C;
/// `SPI_MISC` (0x20) — CS / misc config, pure round-trip.
#[allow(dead_code)]
const MISC: u64 = 0x20;
/// `SPI_DMA_CONF` (0x30) — DMA config, pure round-trip (no DMA modeled).
#[allow(dead_code)]
const DMA_CONF: u64 = 0x30;
const DMA_INT_ENA: u64 = 0x34;
const DMA_INT_CLR: u64 = 0x38;
const DMA_INT_RAW: u64 = 0x3C;
const DMA_INT_ST: u64 = 0x40;
const W0: u64 = 0x98;
/// `SPI_W15` (0xD4) — last word of the 16-word data buffer (documents extent).
#[allow(dead_code)]
const W15: u64 = 0xD4;

/// `SPI_USR` launch bit in `SPI_CMD` (bitpos 24).
const USR_BIT: u32 = 1 << 24;
/// `SPI_TRANS_DONE_INT` bit in the `SPI_DMA_INT_*` block (bitpos 12).
const TRANS_DONE: u32 = 1 << 12;
/// `SPI_MS_DATA_BITLEN` mask in `SPI_MS_DLEN` (bits[17:0]).
const MS_DATA_BITLEN: u32 = 0x0003_FFFF;

/// Reset defaults (from `spi_reg.h` register bit defaults).
/// SPI_CLOCK: CLK_EQU_SYSCLK(b31)=1, CLKCNT_N=3(<<12), CLKCNT_H=1(<<6), CLKCNT_L=3.
const RESET_CLOCK: u32 = (1 << 31) | (3 << 12) | (1 << 6) | 3; // 0x8000_3043
/// SPI_CTRL: WP_POL/HOLD_POL/D_POL/Q_POL (b21..b18) default 1.
const RESET_CTRL: u32 = 0x000F << 18; // 0x003C_0000
/// SPI_USER: USR_COMMAND(b31)=1, CS_SETUP(b7)=1, CS_HOLD(b6)=1.
const RESET_USER: u32 = (1 << 31) | (1 << 7) | (1 << 6); // 0x8000_00C0

pub struct Esp32s3Spi {
    /// Interrupt-matrix source ID (SPI2=21, SPI3=22).
    source_id: u32,
    /// Backing store for all config registers and the W0..W15 data buffer.
    regs: HashMap<u64, u32>,
    /// Latched raw interrupt bits (`SPI_DMA_INT_RAW`); W1C via INT_CLR.
    int_raw: u32,
}

impl std::fmt::Debug for Esp32s3Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s3Spi(src={}, int_raw=0x{:08x}, int_ena=0x{:08x})",
            self.source_id,
            self.int_raw,
            self.reg(DMA_INT_ENA),
        )
    }
}

impl Esp32s3Spi {
    /// A GP-SPI controller instance. `source_id` is the intr-matrix source
    /// (`ETS_SPI2_INTR_SOURCE` = 21 for SPI2/FSPI, `ETS_SPI3_INTR_SOURCE` = 22
    /// for SPI3).
    pub fn new(source_id: u32) -> Self {
        let mut regs = HashMap::new();
        regs.insert(CLOCK, RESET_CLOCK);
        regs.insert(CTRL, RESET_CTRL);
        regs.insert(USER, RESET_USER);
        Self {
            source_id,
            regs,
            int_raw: 0,
        }
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, off: u64, val: u32) {
        self.regs.insert(off, val);
    }

    /// INT_ST = INT_RAW & INT_ENA.
    fn int_st(&self) -> u32 {
        self.int_raw & self.reg(DMA_INT_ENA)
    }

    /// Number of MISO bytes the transaction reads, from `SPI_MS_DLEN`
    /// (`SPI_MS_DATA_BITLEN` is bits-1). Capped at the 64-byte W buffer.
    fn miso_bytes(&self) -> usize {
        let bits = (self.reg(MS_DLEN) & MS_DATA_BITLEN) as usize + 1;
        (bits.div_ceil(8)).min(64)
    }

    /// Launch the user transaction: with no device attached, fill the MISO
    /// region of W0..W15 with 0xFF per byte (idle pulled-high MISO), clear the
    /// `USR` start bit so the firmware's completion poll exits, and latch
    /// `SPI_TRANS_DONE`.
    fn launch_transaction(&mut self) {
        let bytes = self.miso_bytes();
        for w in 0..bytes.div_ceil(4) {
            let off = W0 + (w as u64) * 4;
            // Bytes covered by this word: full 0xFF, partial keep only valid bytes.
            let first_byte = w * 4;
            let mut word = 0u32;
            for b in 0..4 {
                if first_byte + b < bytes {
                    word |= 0xFFu32 << (8 * b);
                }
            }
            self.set_reg(off, word);
        }
        // Clear the USR start bit (auto-clear on done) so `!(CMD & USR)` exits.
        let cmd = self.reg(CMD) & !USR_BIT;
        self.set_reg(CMD, cmd);
        // Latch transaction-done.
        self.int_raw |= TRANS_DONE;
    }
}

impl Peripheral for Esp32s3Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_u32(offset & !3)?;
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(match offset & !3 {
            DMA_INT_RAW => self.int_raw,
            DMA_INT_ST => self.int_st(),
            o => self.reg(o),
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let shift = (offset & 3) * 8;
        // Read-modify-write the affected word, then re-dispatch through the
        // u32 path so launch/W1C side-effects fire consistently.
        let base = match word_off {
            DMA_INT_RAW => self.int_raw,
            _ => self.reg(word_off),
        };
        let merged = (base & !(0xFFu32 << shift)) | ((value as u32) << shift);
        self.write_u32(word_off, merged)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        match offset & !3 {
            // W1C: clear latched raw bits where INT_CLR has a 1.
            DMA_INT_CLR => {
                self.int_raw &= !value;
            }
            // INT_RAW is writable (R/WTC) but firmware rarely writes it; store.
            DMA_INT_RAW => {
                self.int_raw = value;
            }
            // INT_ST is read-only; ignore writes.
            DMA_INT_ST => {}
            CMD => {
                self.set_reg(CMD, value);
                if value & USR_BIT != 0 {
                    self.launch_transaction();
                }
            }
            // Everything else (USER/USER1/USER2/CLOCK/CTRL/MISC/DLEN/ADDR/
            // DMA_CONF/INT_ENA and the W0..W15 buffer) round-trips verbatim.
            o => self.set_reg(o, value),
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult {
            explicit_irqs: if self.int_st() != 0 {
                Some(vec![self.source_id])
            } else {
                None
            },
            ..PeripheralTickResult::default()
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPI2_SOURCE: u32 = 21;
    const SPI3_SOURCE: u32 = 22;

    #[test]
    fn config_registers_round_trip() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        s.write_u32(USER, 0x9000_00C0).unwrap();
        s.write_u32(USER1, 0x0000_5817).unwrap();
        s.write_u32(USER2, 0x7000_0006).unwrap();
        s.write_u32(CLOCK, 0x0000_1001).unwrap();
        s.write_u32(CTRL, 0x003C_0000).unwrap();
        s.write_u32(MISC, 0x0000_003E).unwrap();
        s.write_u32(MS_DLEN, 0x0000_001F).unwrap();
        assert_eq!(s.read_u32(USER).unwrap(), 0x9000_00C0);
        assert_eq!(s.read_u32(USER1).unwrap(), 0x0000_5817);
        assert_eq!(s.read_u32(USER2).unwrap(), 0x7000_0006);
        assert_eq!(s.read_u32(CLOCK).unwrap(), 0x0000_1001);
        assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000);
        assert_eq!(s.read_u32(MISC).unwrap(), 0x0000_003E);
        assert_eq!(s.read_u32(MS_DLEN).unwrap(), 0x0000_001F);
    }

    #[test]
    fn reset_defaults_seeded() {
        let s = Esp32s3Spi::new(SPI2_SOURCE);
        assert_eq!(s.read_u32(CLOCK).unwrap(), 0x8000_3043);
        assert_eq!(s.read_u32(CTRL).unwrap(), 0x003C_0000);
        assert_eq!(s.read_u32(USER).unwrap(), 0x8000_00C0);
    }

    #[test]
    fn w_buffer_round_trips() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        for w in 0..16u64 {
            let off = W0 + w * 4;
            let val = 0xA000_0000 | (w as u32);
            s.write_u32(off, val).unwrap();
        }
        for w in 0..16u64 {
            let off = W0 + w * 4;
            assert_eq!(s.read_u32(off).unwrap(), 0xA000_0000 | (w as u32));
        }
        assert_eq!(W15, W0 + 15 * 4, "W15 offset");
    }

    #[test]
    fn w_buffer_byte_round_trip() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // Byte-wise MOSI fill into W0.
        s.write(W0, 0xDE).unwrap();
        s.write(W0 + 1, 0xAD).unwrap();
        s.write(W0 + 2, 0xBE).unwrap();
        s.write(W0 + 3, 0xEF).unwrap();
        assert_eq!(s.read_u32(W0).unwrap(), 0xEFBE_ADDE);
        assert_eq!(s.read(W0).unwrap(), 0xDE);
        assert_eq!(s.read(W0 + 3).unwrap(), 0xEF);
    }

    #[test]
    fn usr_start_auto_clears_and_latches_trans_done() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // 32-bit transfer.
        s.write_u32(MS_DLEN, 32 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        // USR auto-cleared so `!(CMD & USR)` poll exits.
        assert_eq!(s.read_u32(CMD).unwrap() & USR_BIT, 0, "USR auto-clears");
        // TRANS_DONE latched in INT_RAW.
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            TRANS_DONE,
            "TRANS_DONE latched"
        );
    }

    #[test]
    fn miso_fills_0xff_for_no_device() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // Pre-load W0 with MOSI data; the launch overwrites the MISO region.
        s.write_u32(W0, 0x0000_0000).unwrap();
        s.write_u32(W0 + 4, 0x1234_5678).unwrap();
        // 5-byte (40-bit) transfer → W0 fully 0xFF, W1 low byte 0xFF only.
        s.write_u32(MS_DLEN, 40 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(
            s.read_u32(W0).unwrap(),
            0xFFFF_FFFF,
            "first 4 MISO bytes 0xFF"
        );
        assert_eq!(
            s.read_u32(W0 + 4).unwrap(),
            0x0000_00FF,
            "5th byte 0xFF, rest untouched"
        );
    }

    #[test]
    fn int_clr_is_w1c() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE, TRANS_DONE);
        // W1C: writing the bit clears it; writing 0 leaves others intact.
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert_eq!(
            s.read_u32(DMA_INT_RAW).unwrap() & TRANS_DONE,
            0,
            "W1C cleared"
        );
    }

    #[test]
    fn int_st_masks_with_ena_and_emits_source() {
        let mut s = Esp32s3Spi::new(SPI2_SOURCE);
        // No enable yet → no source emitted even after a transaction.
        s.write_u32(MS_DLEN, 8 - 1).unwrap();
        s.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap(), 0, "ST gated by ENA");
        assert_eq!(s.tick().explicit_irqs, None, "no source while ST==0");
        // Enable TRANS_DONE → ST asserts and the source is emitted while ST!=0.
        s.write_u32(DMA_INT_ENA, TRANS_DONE).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap() & TRANS_DONE, TRANS_DONE);
        assert_eq!(s.tick().explicit_irqs, Some(vec![SPI2_SOURCE]));
        // Clear the raw bit → ST drops, source stops.
        s.write_u32(DMA_INT_CLR, TRANS_DONE).unwrap();
        assert_eq!(s.tick().explicit_irqs, None);
    }

    #[test]
    fn int_st_is_read_only() {
        let mut s = Esp32s3Spi::new(SPI3_SOURCE);
        s.write_u32(DMA_INT_ST, 0xFFFF_FFFF).unwrap();
        assert_eq!(s.read_u32(DMA_INT_ST).unwrap(), 0, "INT_ST ignores writes");
    }
}
