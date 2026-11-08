// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S2 GP-SPI master (SPI2 / FSPI @ `0x3F42_4000`, SPI3 @ `0x3F42_5000`).
//!
//! The S2's GP-SPI is neither of its neighbours. Read against its own header,
//! `soc/esp32s2/register/soc/spi_reg.h`:
//!
//! * like the S3/C3 it launches with `SPI_CMD.USR` at bit **24** and keeps
//!   `CK_OUT_EDGE` in `SPI_USER` bit 9 and `CK_IDLE_EDGE` in `SPI_MISC` bit 29;
//! * like the classic ESP32 it has separate `SPI_MOSI_DLEN` (0x24) and
//!   `SPI_MISO_DLEN` (0x28) and reports completion in `SPI_SLAVE.TRANS_DONE`
//!   (0x30 bit 4), not in a `DMA_INT_RAW` bit;
//! * its own offsets: `USER` 0x18, `USER1` 0x1C, `USER2` 0x20, `CLOCK` 0x14,
//!   `W0..W17` at 0x98..0xDC, `DATE` at 0x3FC (reset `0x1907240`).
//!
//! Neither `esp32::spi::Esp32Spi` (USR at bit 18, flash-command bits across
//! CMD[31:16] — bit 24 there is `FLASH_SE`) nor `esp32s3::gpspi::Esp32s3Spi`
//! (single `MS_DLEN` at 0x1C) can be reused without decoding the wrong bits,
//! so this is a separate, deliberately small model.
//!
//! ## What it does
//!
//! The CPU path Arduino-ESP32's `esp32-hal-spi.c` drives on the S2:
//! program `MOSI_DLEN` / `MISO_DLEN` (bits - 1), fill `W0..`, set
//! `SPI_CMD.USR`, busy-poll `while (cmd.usr)`, read `W0..` back. A launch
//! completes synchronously: each MOSI byte (when `USR_MOSI`) is clocked
//! through every attached [`SpiDevice`]; with `USR_MISO` the first non-zero
//! device response is written back into the same W-buffer lane (the classic
//! model's broadcast rule; one device is the supported case). `USR` clears
//! and `SPI_SLAVE.TRANS_DONE` latches. `CTRL.WR_BIT_ORDER` (26) /
//! `RD_BIT_ORDER` (25) select LSB-first for MOSI / MISO bytes.
//!
//! With no device attached, MISO reads `0xFF` (an undriven, pulled-up line).
//!
//! Every other register is a word-wide register file seeded with its reset
//! value, so the driver's read-modify-write sequences see what they wrote.
//!
//! ## What it does not do
//!
//! No DMA (`SPI_DMA_CONF` and the link registers are stored, never acted
//! on), no slave mode, no command/address/dummy phases (only the data
//! phase reaches devices), no waveform on the pads, and no interrupt: the
//! intr-matrix source (`ETS_SPI2_INTR_SOURCE` = 33) is never asserted, so a
//! driver that waits on `TRANS_DONE` by interrupt instead of polling `USR`
//! (ESP-IDF `spi_master` in interrupt mode) will not complete.

use crate::peripherals::spi::SpiDevice;
use crate::{Peripheral, SimResult};

/// `ETS_SPI2_INTR_SOURCE` in `soc/esp32s2/include/soc/interrupts.h`
/// (SPI1 = 32, SPI2 = 33, SPI3 = 34). Recorded for the descriptor; not raised.
pub const ESP32S2_SPI2_INTR_SOURCE: u32 = 33;

const REG_CMD: u64 = 0x00;
const REG_CTRL: u64 = 0x08;
const REG_CTRL1: u64 = 0x0C;
const REG_CTRL2: u64 = 0x10;
const REG_CLOCK: u64 = 0x14;
const REG_USER: u64 = 0x18;
const REG_USER1: u64 = 0x1C;
const REG_USER2: u64 = 0x20;
const REG_MOSI_DLEN: u64 = 0x24;
const REG_MISO_DLEN: u64 = 0x28;
const REG_MISC: u64 = 0x2C;
const REG_SLAVE: u64 = 0x30;
const REG_DMA_CONF: u64 = 0x4C;
/// `SPI_W0_REG` .. `SPI_W17_REG` (0x98..0xDC): 72-byte data buffer.
const W0: u64 = 0x98;
const W_WORDS: usize = 18;
const REG_DATE: u64 = 0x3FC;

/// The window is 4 KiB but the architected map ends at `DATE` (0x3FC).
const WORDS: usize = 0x400 / 4;

/// `SPI_USR` (CMD bit 24). `SPI_CONF_BITLEN` [22:0] is plain storage.
const CMD_USR: u32 = 1 << 24;
/// `SPI_WR_BIT_ORDER` (CTRL 26) / `SPI_RD_BIT_ORDER` (CTRL 25): 1 = LSB first.
const CTRL_WR_BIT_ORDER: u32 = 1 << 26;
const CTRL_RD_BIT_ORDER: u32 = 1 << 25;
const USER_USR_MISO: u32 = 1 << 28;
const USER_USR_MOSI: u32 = 1 << 27;
/// `SPI_USR_MOSI_BIT_LEN` / `SPI_USR_MISO_BIT_LEN` [22:0].
const DLEN_MASK: u32 = 0x7F_FFFF;
/// `SPI_TRANS_DONE` (SLAVE bit 4).
const SLAVE_TRANS_DONE: u32 = 1 << 4;

/// Reset values of the registers whose header `default:` is non-zero.
const RESETS: &[(u64, u32)] = &[
    // WP_REG 21, D_POL 19, Q_POL 18.
    (REG_CTRL, (1 << 21) | (1 << 19) | (1 << 18)),
    // CS_HOLD_DELAY [19:14] = 1, W16_17_WR_ENA 4.
    (REG_CTRL1, (1 << 14) | (1 << 4)),
    // CS_HOLD_TIME [25:13] = 1.
    (REG_CTRL2, 1 << 13),
    // CLK_EQU_SYSCLK 31, CLKCNT_N 3, CLKCNT_H 1, CLKCNT_L 3.
    (REG_CLOCK, (1 << 31) | (3 << 12) | (1 << 6) | 3),
    // USR_COMMAND 31, CS_SETUP 7, CS_HOLD 6.
    (REG_USER, (1 << 31) | (1 << 7) | (1 << 6)),
    // USR_ADDR_BITLEN [31:27] = 23, USR_DUMMY_CYCLELEN [7:0] = 7.
    (REG_USER1, (23 << 27) | 7),
    // USR_COMMAND_BITLEN [31:28] = 7.
    (REG_USER2, 7 << 28),
    // CS1..CS5_DIS.
    (REG_MISC, 0x3E),
    // INT_TRANS_DONE_EN 9.
    (REG_SLAVE, 1 << 9),
    // OUT_EOF_MODE 9.
    (REG_DMA_CONF, 1 << 9),
    (REG_DATE, 0x0190_7240),
];

pub struct Esp32s2Spi {
    regs: Box<[u32; WORDS]>,
    pub attached_devices: Vec<Box<dyn SpiDevice>>,
    transactions: u64,
}

impl std::fmt::Debug for Esp32s2Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32s2Spi(user=0x{:08x} transactions={} attached={})",
            self.reg(REG_USER),
            self.transactions,
            self.attached_devices.len()
        )
    }
}

impl Default for Esp32s2Spi {
    fn default() -> Self {
        Self::new()
    }
}

impl Esp32s2Spi {
    pub fn new() -> Self {
        let mut regs = Box::new([0u32; WORDS]);
        for &(off, value) in RESETS {
            regs[(off / 4) as usize] = value;
        }
        Self {
            regs,
            attached_devices: Vec::new(),
            transactions: 0,
        }
    }

    /// Completed `SPI_CMD.USR` transactions.
    pub fn transactions(&self) -> u64 {
        self.transactions
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs[(off / 4) as usize]
    }

    fn set_reg(&mut self, off: u64, value: u32) {
        self.regs[(off / 4) as usize] = value;
    }

    fn write_word(&mut self, off: u64, value: u32) {
        if off == REG_DATE {
            return; // read-only version stamp
        }
        if off == REG_CMD && value & CMD_USR != 0 {
            self.set_reg(REG_CMD, value);
            self.run_user_transaction();
            self.set_reg(REG_CMD, value & !CMD_USR);
            return;
        }
        self.set_reg(off, value);
    }

    /// The data phase of one user transaction, completed synchronously.
    fn run_user_transaction(&mut self) {
        self.transactions += 1;
        let user = self.reg(REG_USER);
        let ctrl = self.reg(REG_CTRL);
        let mosi_en = user & USER_USR_MOSI != 0;
        let miso_en = user & USER_USR_MISO != 0;
        let mosi_bits = (self.reg(REG_MOSI_DLEN) & DLEN_MASK) + 1;
        let miso_bits = (self.reg(REG_MISO_DLEN) & DLEN_MASK) + 1;
        let bits = match (mosi_en, miso_en) {
            (true, true) => mosi_bits.max(miso_bits),
            (true, false) => mosi_bits,
            (false, true) => miso_bits,
            (false, false) => 0,
        };
        let bytes = (bits.div_ceil(8) as usize).min(W_WORDS * 4);
        for i in 0..bytes {
            let word = (W0 / 4) as usize + i / 4;
            let shift = (i % 4) * 8;
            let mut mosi = if mosi_en {
                (self.regs[word] >> shift) as u8
            } else {
                0
            };
            if ctrl & CTRL_WR_BIT_ORDER != 0 {
                mosi = mosi.reverse_bits();
            }
            let mut miso = if self.attached_devices.is_empty() {
                0xFF
            } else {
                0
            };
            for dev in &mut self.attached_devices {
                let resp = dev.transfer(mosi);
                if miso == 0 {
                    miso = resp;
                }
            }
            if ctrl & CTRL_RD_BIT_ORDER != 0 {
                miso = miso.reverse_bits();
            }
            if miso_en {
                self.regs[word] = (self.regs[word] & !(0xFF << shift)) | (u32::from(miso) << shift);
            }
        }
        let slave = self.reg(REG_SLAVE);
        self.set_reg(REG_SLAVE, slave | SLAVE_TRANS_DONE);
    }
}

impl Peripheral for Esp32s2Spi {
    fn read(&self, offset: u64) -> SimResult<u8> {
        if offset >= (WORDS * 4) as u64 {
            return Ok(0);
        }
        Ok((self.reg(offset & !3) >> ((offset & 3) * 8)) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset >= (WORDS * 4) as u64 {
            return Ok(());
        }
        let off = offset & !3;
        let shift = (offset & 3) * 8;
        let word = (self.reg(off) & !(0xFF << shift)) | (u32::from(value) << shift);
        self.write_word(off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 != 0 {
            for i in 0..4 {
                self.write(offset + i, (value >> (i * 8)) as u8)?;
            }
            return Ok(());
        }
        if offset < (WORDS * 4) as u64 {
            self.write_word(offset, value);
        }
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "cmd": self.reg(REG_CMD),
            "user": self.reg(REG_USER),
            "mosi_dlen": self.reg(REG_MOSI_DLEN),
            "miso_dlen": self.reg(REG_MISO_DLEN),
            "slave": self.reg(REG_SLAVE),
            "transactions": self.transactions,
        })
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn spi_attached_devices(&self) -> Option<&Vec<Box<dyn SpiDevice>>> {
        Some(&self.attached_devices)
    }

    fn spi_attached_devices_mut(&mut self) -> Option<&mut Vec<Box<dyn SpiDevice>>> {
        Some(&mut self.attached_devices)
    }

    fn for_each_attached_device(&self, f: &mut dyn FnMut(crate::inspect::AttachedDeviceRef<'_>)) {
        for dev in &self.attached_devices {
            crate::inspect::visit_spi_device(&**dev, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers a fixed byte sequence and records what it was sent.
    struct Script {
        out: Vec<u8>,
        sent: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }
    impl Script {
        fn boxed(out: Vec<u8>) -> Box<Self> {
            Box::new(Self {
                out,
                sent: Default::default(),
            })
        }
    }
    impl SpiDevice for Script {
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.sent.lock().unwrap().push(mosi);
            if self.out.is_empty() {
                0
            } else {
                self.out.remove(0)
            }
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }

    fn read32(spi: &Esp32s2Spi, off: u64) -> u32 {
        (0..4).fold(0, |acc, i| acc | (u32::from(spi.read(off + i).unwrap()) << (i * 8)))
    }

    /// The exact sequence `spiTransferByte` runs on the S2 after `spiStartBus`.
    fn transfer_byte(spi: &mut Esp32s2Spi, byte: u8) -> u8 {
        spi.write_u32(REG_MOSI_DLEN, 7).unwrap();
        spi.write_u32(REG_MISO_DLEN, 7).unwrap();
        spi.write_u32(W0, u32::from(byte)).unwrap();
        spi.write_u32(REG_CMD, CMD_USR).unwrap();
        assert_eq!(read32(spi, REG_CMD) & CMD_USR, 0, "USR must self-clear");
        read32(spi, W0) as u8
    }

    fn started() -> Esp32s2Spi {
        let mut spi = Esp32s2Spi::new();
        // spiInitBus clears USER, then spiStartBus sets usr_mosi|usr_miso|doutdin.
        spi.write_u32(REG_USER, USER_USR_MOSI | USER_USR_MISO | 1).unwrap();
        spi.write_u32(REG_CTRL, 0).unwrap();
        spi
    }

    #[test]
    fn resets_match_the_s2_header() {
        let spi = Esp32s2Spi::new();
        assert_eq!(read32(&spi, REG_DATE), 0x0190_7240);
        assert_eq!(read32(&spi, REG_CLOCK) >> 31, 1, "CLK_EQU_SYSCLK resets to 1");
        assert_eq!(read32(&spi, REG_USER1) >> 27, 23);
    }

    #[test]
    fn usr_exchanges_a_byte_with_the_attached_device() {
        let mut spi = started();
        spi.attached_devices.push(Script::boxed(vec![0x01, 0x90, 0x16, 0x00]));
        let frame = (0..4).fold(0u32, |acc, _| (acc << 8) | u32::from(transfer_byte(&mut spi, 0)));
        assert_eq!(frame, 0x0190_1600);
        assert_eq!(spi.transactions(), 4);
        assert_ne!(read32(&spi, REG_SLAVE) & SLAVE_TRANS_DONE, 0);
    }

    #[test]
    fn no_device_reads_an_idle_high_line() {
        let mut spi = started();
        assert_eq!(transfer_byte(&mut spi, 0x5A), 0xFF);
    }

    #[test]
    fn bit_order_bits_reverse_mosi_and_miso() {
        let mut spi = started();
        spi.write_u32(REG_CTRL, CTRL_WR_BIT_ORDER | CTRL_RD_BIT_ORDER).unwrap();
        let dev = Script::boxed(vec![0x01]);
        let sent = dev.sent.clone();
        spi.attached_devices.push(dev);
        assert_eq!(transfer_byte(&mut spi, 0x80), 0x80);
        assert_eq!(*sent.lock().unwrap(), vec![0x01]);
    }

    #[test]
    fn launch_bit_is_s2_bit_24_not_classic_bit_18() {
        let mut spi = started();
        spi.attached_devices.push(Script::boxed(vec![0x42]));
        spi.write_u32(REG_MOSI_DLEN, 7).unwrap();
        spi.write_u32(REG_CMD, 1 << 18).unwrap();
        assert_eq!(spi.transactions(), 0);
    }
}
