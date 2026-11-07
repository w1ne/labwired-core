// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! LiteX SPI-flash bitbang controller with a Micron MT25Q on its bus — the
//! `spi` + `flash` pair Renode's `litex_common.repl` maps at 0x60005000.
//!
//! Register map (32-bit CSRs, 4-byte strides; Renode `LiteX_SPI_Flash`):
//!
//! | Offset | Name | Access | Meaning |
//! | --- | --- | --- | --- |
//! | 0x00 | BitBang | W | bit0 MOSI, bit1 CLK, bit2 CS_N, bit3 DIR (1 = MISO input phase) |
//! | 0x04 | Miso | R | bit0 = the flash's current output bit |
//! | 0x08 | BitBangEnable | R/W | bit0 hands the pads to the BitBang register |
//!
//! BitBang writes are ignored while BitBangEnable is clear. A CS_N rising
//! edge ends the flash transaction. Each CLK rising edge advances the MISO
//! shift (MSB first) and, while CS_N is low and DIR is clear, latches MOSI;
//! eight latched bits are one byte to the flash, and the flash's answer
//! drives MISO for the next eight rising edges. This is Renode's
//! `BitBangHelper` with `outputMsbFirst: true`, including its byte-late
//! answer: the 8th edge that sends a byte also reloads MISO, so firmware
//! reads an answer by sending a dummy byte and then clocking eight DIR=1
//! edges, as the LiteX BIOS does.
//!
//! The flash side follows Renode's `GenericSpiFlash` as `Micron_MT25Q` builds
//! it over the 32 MiB `flash_mem` window. The command byte itself answers 0.
//! Modelled commands:
//! - READ ID (0x9F / 0xAF): 20 0xBB 0x19 0x10 0x00 0x00, then 14 zero
//!   unique-ID bytes; reads beyond the 20-byte table answer 0.
//! - READ STATUS (0x05): bit1 is the write-enable latch, every other bit 0.
//! - WRITE ENABLE (0x06) / WRITE DISABLE (0x04) set / clear the latch.
//!
//! Not modelled: array reads, program, erase, SFDP and the configuration
//! registers. Those opcodes answer 0 on MISO. The XIP window at 0x20000000
//! is mapped as plain flash memory, not through this controller.

use crate::SimResult;

const REG_BITBANG: u64 = 0x00;
const REG_MISO: u64 = 0x04;
const REG_BITBANG_EN: u64 = 0x08;

const BB_MOSI: u32 = 1 << 0;
const BB_CLK: u32 = 1 << 1;
const BB_CS_N: u32 = 1 << 2;
const BB_DIR: u32 = 1 << 3;

const CMD_READ_ID: u8 = 0x9F;
const CMD_MULTIPLE_IO_READ_ID: u8 = 0xAF;
const CMD_READ_STATUS: u8 = 0x05;
const CMD_WRITE_ENABLE: u8 = 0x06;
const CMD_WRITE_DISABLE: u8 = 0x04;

/// MT25Q READ ID table for the 32 MiB part: manufacturer 0x20 (Micron),
/// memory type 0xBB (1.8 V), capacity 0x19 (2^25 bytes), remaining-ID
/// length 0x10, extended device ID 0x00, device configuration 0x00, then the
/// unique ID bytes (zero in Renode).
const DEVICE_ID: [u8; 20] = {
    let mut id = [0u8; 20];
    id[0] = 0x20;
    id[1] = 0xBB;
    id[2] = 0x19;
    id[3] = 0x10;
    id
};

const SR_WEL: u8 = 1 << 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum FlashOp {
    /// Waiting for a command byte.
    Recognize,
    ReadId,
    ReadStatus,
    /// An opcode this model does not implement; data bytes answer 0.
    Unsupported,
    /// A command with no data phase (WREN/WRDI) already ran.
    Done,
}

/// The MT25Q command responder behind the controller.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Mt25q {
    op: FlashOp,
    handled: usize,
    write_enable: bool,
}

impl Mt25q {
    fn new() -> Self {
        Self {
            op: FlashOp::Recognize,
            handled: 0,
            write_enable: false,
        }
    }

    fn transmit(&mut self, byte: u8) -> u8 {
        match self.op {
            FlashOp::Recognize => {
                self.handled = 0;
                self.op = match byte {
                    CMD_READ_ID | CMD_MULTIPLE_IO_READ_ID => FlashOp::ReadId,
                    CMD_READ_STATUS => FlashOp::ReadStatus,
                    CMD_WRITE_ENABLE => {
                        self.write_enable = true;
                        FlashOp::Done
                    }
                    CMD_WRITE_DISABLE => {
                        self.write_enable = false;
                        FlashOp::Done
                    }
                    _ => FlashOp::Unsupported,
                };
                0
            }
            FlashOp::ReadId => {
                let out = DEVICE_ID.get(self.handled).copied().unwrap_or(0);
                self.handled += 1;
                out
            }
            // The datasheet repeats the status byte for as long as CS stays low.
            FlashOp::ReadStatus => {
                if self.write_enable {
                    SR_WEL
                } else {
                    0
                }
            }
            FlashOp::Unsupported | FlashOp::Done => 0,
        }
    }

    fn finish(&mut self) {
        self.op = FlashOp::Recognize;
        self.handled = 0;
    }
}

/// LiteX SPI-flash bitbang controller.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct LitexSpiFlash {
    bitbang_en: bool,
    prev_clk: bool,
    prev_cs_n: bool,
    /// MOSI bits latched so far in the current byte (MSB first).
    out_shift: u8,
    out_bits: u8,
    /// The flash's last answer byte and the bit MISO presents. `None` until
    /// a byte has been exchanged or once all eight bits have been shifted.
    in_byte: u8,
    in_pos: Option<u8>,
    flash: Mt25q,
}

impl Default for LitexSpiFlash {
    fn default() -> Self {
        Self::new()
    }
}

impl LitexSpiFlash {
    pub fn new() -> Self {
        Self {
            bitbang_en: false,
            prev_clk: false,
            prev_cs_n: true,
            out_shift: 0,
            out_bits: 0,
            in_byte: 0,
            in_pos: None,
            flash: Mt25q::new(),
        }
    }

    fn miso(&self) -> bool {
        match self.in_pos {
            Some(pos) if pos < 8 => self.in_byte & (1 << pos) != 0,
            _ => false,
        }
    }

    fn write_bitbang(&mut self, value: u32) {
        if !self.bitbang_en {
            return;
        }
        let clk = value & BB_CLK != 0;
        let cs_n = value & BB_CS_N != 0;
        let latch = value & BB_DIR == 0 && !cs_n;

        if cs_n && !self.prev_cs_n {
            self.flash.finish();
        }
        self.prev_cs_n = cs_n;

        if clk && !self.prev_clk {
            // Advance MISO first, then latch MOSI (Renode's helper order).
            self.in_pos = match self.in_pos {
                Some(0) | None => None,
                Some(pos) => Some(pos - 1),
            };
            if latch {
                self.out_shift = (self.out_shift << 1) | (value & BB_MOSI) as u8;
                self.out_bits += 1;
                if self.out_bits == 8 {
                    let answer = self.flash.transmit(self.out_shift);
                    self.out_shift = 0;
                    self.out_bits = 0;
                    self.in_byte = answer;
                    // Bit 7 appears on the next rising edge.
                    self.in_pos = Some(8);
                }
            }
        }
        self.prev_clk = clk;
    }

    fn read_reg(&self, reg: u64) -> u32 {
        match reg {
            REG_MISO => self.miso() as u32,
            REG_BITBANG_EN => self.bitbang_en as u32,
            // BitBang is write-only; the rest of the CSR page is unused.
            _ => 0,
        }
    }

    fn write_reg(&mut self, reg: u64, value: u32) {
        match reg {
            REG_BITBANG => self.write_bitbang(value),
            REG_BITBANG_EN => self.bitbang_en = value & 1 != 0,
            _ => {}
        }
    }
}

impl crate::Peripheral for LitexSpiFlash {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let byte = (offset % 4) as u32;
        Ok(((self.read_reg(offset & !3) >> (byte * 8)) & 0xFF) as u8)
    }

    /// Every field sits in byte 0 of its CSR. A byte store there is the whole
    /// register write; stores to bytes 1..3 carry no field.
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value as u32);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset % 4 == 0 {
            self.write_reg(offset, value);
        }
        Ok(())
    }

    fn on_system_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LitexSpiFlash snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize LitexSpiFlash")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LitexSpiFlash runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn bb(spi: &mut LitexSpiFlash, value: u32) {
        spi.write_u32(REG_BITBANG, value).unwrap();
    }

    /// Shift one byte out, MSB first: MOSI with CLK low, then CLK high.
    fn send(spi: &mut LitexSpiFlash, out: u8) {
        for bit in (0..8).rev() {
            let mosi = ((out >> bit) & 1) as u32;
            bb(spi, mosi);
            bb(spi, mosi | BB_CLK);
        }
        bb(spi, 0);
    }

    /// Read one answer the way the LiteX BIOS does under Renode: a dummy
    /// byte fetches the flash's answer, then eight DIR (input-phase) clocks
    /// shift it out, sampling MISO while CLK is high.
    fn receive(spi: &mut LitexSpiFlash) -> u8 {
        send(spi, 0);
        let mut input = 0u8;
        for _ in 0..8 {
            bb(spi, BB_DIR);
            bb(spi, BB_DIR | BB_CLK);
            input = (input << 1) | spi.read_u32(REG_MISO).unwrap() as u8;
        }
        bb(spi, 0);
        input
    }

    fn command(spi: &mut LitexSpiFlash, cmd: u8, reads: usize) -> Vec<u8> {
        spi.write_u32(REG_BITBANG_EN, 1).unwrap();
        bb(spi, BB_CS_N);
        bb(spi, 0);
        send(spi, cmd);
        let out = (0..reads).map(|_| receive(spi)).collect();
        bb(spi, BB_CS_N);
        out
    }

    #[test]
    fn reset_reads_zero_and_bitbang_disabled() {
        let spi = LitexSpiFlash::new();
        assert_eq!(spi.read_u32(REG_BITBANG).unwrap(), 0);
        assert_eq!(spi.read_u32(REG_MISO).unwrap(), 0);
        assert_eq!(spi.read_u32(REG_BITBANG_EN).unwrap(), 0);
    }

    #[test]
    fn bitbang_enable_reads_back() {
        let mut spi = LitexSpiFlash::new();
        spi.write_u32(REG_BITBANG_EN, 0xFFFF_FFFF).unwrap();
        assert_eq!(spi.read_u32(REG_BITBANG_EN).unwrap(), 1);
    }

    #[test]
    fn read_id_returns_the_mt25q_32mib_table() {
        let mut spi = LitexSpiFlash::new();
        assert_eq!(
            command(&mut spi, CMD_READ_ID, 5),
            vec![0x20, 0xBB, 0x19, 0x10, 0x00]
        );
    }

    #[test]
    fn write_enable_sets_the_status_latch_and_write_disable_clears_it() {
        let mut spi = LitexSpiFlash::new();
        assert_eq!(command(&mut spi, CMD_READ_STATUS, 1), vec![0x00]);
        command(&mut spi, CMD_WRITE_ENABLE, 0);
        assert_eq!(command(&mut spi, CMD_READ_STATUS, 2), vec![SR_WEL, SR_WEL]);
        command(&mut spi, CMD_WRITE_DISABLE, 0);
        assert_eq!(command(&mut spi, CMD_READ_STATUS, 1), vec![0x00]);
    }

    #[test]
    fn writes_are_ignored_while_bitbang_is_disabled() {
        let mut spi = LitexSpiFlash::new();
        spi.write_u32(REG_BITBANG_EN, 1).unwrap();
        bb(&mut spi, 0);
        spi.write_u32(REG_BITBANG_EN, 0).unwrap();
        // Clock a READ ID with the controller disabled: nothing reaches the flash.
        for bit in (0..8).rev() {
            let mosi = ((CMD_READ_ID >> bit) & 1) as u32;
            bb(&mut spi, mosi);
            bb(&mut spi, mosi | BB_CLK);
        }
        assert_eq!(spi.flash.op, FlashOp::Recognize);
    }

    #[test]
    fn dir_input_phase_does_not_latch_mosi() {
        let mut spi = LitexSpiFlash::new();
        spi.write_u32(REG_BITBANG_EN, 1).unwrap();
        bb(&mut spi, BB_CS_N);
        bb(&mut spi, 0);
        for _ in 0..8 {
            bb(&mut spi, BB_DIR | BB_MOSI);
            bb(&mut spi, BB_DIR | BB_MOSI | BB_CLK);
        }
        assert_eq!(spi.flash.op, FlashOp::Recognize);
        assert_eq!(spi.out_bits, 0);
    }

    #[test]
    fn unsupported_opcode_answers_zero() {
        let mut spi = LitexSpiFlash::new();
        // 0x03 READ: the array is not served by this model.
        assert_eq!(command(&mut spi, 0x03, 4), vec![0, 0, 0, 0]);
    }

    #[test]
    fn cs_rising_edge_ends_the_transaction() {
        let mut spi = LitexSpiFlash::new();
        command(&mut spi, CMD_READ_ID, 1);
        assert_eq!(spi.flash.op, FlashOp::Recognize);
        // A fresh READ ID starts from the manufacturer byte again.
        assert_eq!(command(&mut spi, CMD_READ_ID, 1), vec![0x20]);
    }

    #[test]
    fn runtime_snapshot_round_trips_mid_transaction() {
        let mut spi = LitexSpiFlash::new();
        spi.write_u32(REG_BITBANG_EN, 1).unwrap();
        bb(&mut spi, BB_CS_N);
        bb(&mut spi, 0);
        send(&mut spi, CMD_READ_ID);
        assert_eq!(receive(&mut spi), 0x20);
        let bytes = spi.runtime_snapshot();
        let mut resumed = LitexSpiFlash::new();
        resumed.restore_runtime_snapshot(&bytes).unwrap();
        assert_eq!(receive(&mut resumed), 0xBB);
    }
}
