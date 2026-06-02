// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SPIMEM1 flash-command controller (`0x6000_2000`).
//!
//! Proper-model component (replaces the auto-clear stub): the ROM /
//! bootloader / esp_flash driver issue user commands through this controller
//! and read the result out of the data buffer. We execute the command against
//! a real flash backing so reads return real data, then clear the USR trigger
//! so the firmware's completion poll exits.
//!
//! Register map (offsets from base), as used by the firmware
//! (derived from `bootloader_flash_execute_command_common`):
//!   +0x00 CMD       bit 18 = SPI_MEM_USR — set to launch, auto-clears on done
//!   +0x04 ADDR      flash byte address in bits[31:8] (24-bit addr, <<8)
//!   +0x18 USER      command/addr/dummy phase enables
//!   +0x1C USER1     addr/dummy bit lengths
//!   +0x20 USER2     bits[15:0] = command opcode, bits[31:28] = cmd bitlen-1
//!   +0x24 MOSI_DLEN write data bitlen-1
//!   +0x28 MISO_DLEN read data bitlen-1
//!   +0x58 W0..W15   data buffer (MOSI out / MISO in)

use crate::{Peripheral, SimResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const CMD: u64 = 0x00;
const ADDR: u64 = 0x04;
const USER2: u64 = 0x20;
const MISO_DLEN: u64 = 0x28;
const W0: u64 = 0x58;
const USR_BIT: u32 = 1 << 18;

/// Flash command opcodes the controller emulates.
const CMD_READ: u8 = 0x03; // READ
const CMD_FAST_READ: u8 = 0x0B; // FAST READ
const CMD_RDSR: u8 = 0x05; // read status register
const CMD_RDID: u8 = 0x9F; // read JEDEC id

#[derive(Debug)]
pub struct SpiMemFlash {
    regs: HashMap<u64, u32>,
    flash: Arc<Mutex<Vec<u8>>>,
}

impl SpiMemFlash {
    pub fn new(flash: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            regs: HashMap::new(),
            flash,
        }
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, off: u64, val: u32) {
        self.regs.insert(off, val);
    }

    /// Execute the user command currently programmed in the registers, placing
    /// any read result in the W buffer, then clear the USR trigger.
    fn execute_user_command(&mut self) {
        let cmd = (self.reg(USER2) & 0xFFFF) as u8;
        // ESP32-S3 SPI_MEM_ADDR_REG carries the 24-bit flash byte address in
        // the high bits (the low 8 bits are the dummy/alignment slot).
        let addr_reg = self.reg(ADDR);
        let addr = (addr_reg >> 8) as usize;
        // MISO_DLEN holds (bits - 1); bytes = (bits)/8.
        let read_bytes = ((self.reg(MISO_DLEN) as usize) + 1) / 8;
        let debug = std::env::var("LABWIRED_SPI_DEBUG").is_ok();
        if debug {
            let f = self.flash.lock().unwrap();
            let peek: Vec<u8> = (0..4).map(|i| f.get(addr + i).copied().unwrap_or(0xFF)).collect();
            eprintln!(
                "spimem1: cmd=0x{cmd:02x} ADDR_REG=0x{addr_reg:08x} addr=0x{addr:06x} bytes={read_bytes} flash[addr..]={peek:02x?}"
            );
        }

        match cmd {
            CMD_READ | CMD_FAST_READ => {
                let flash = self.flash.lock().unwrap();
                // Pack the read data into W0.. little-endian, mirroring how the
                // hardware fills the buffer the firmware then reads back.
                let mut buf = vec![0u8; read_bytes];
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = flash.get(addr + i).copied().unwrap_or(0xFF);
                }
                drop(flash);
                for (w, chunk) in buf.chunks(4).enumerate() {
                    let mut word = 0u32;
                    for (j, &b) in chunk.iter().enumerate() {
                        word |= (b as u32) << (8 * j);
                    }
                    self.set_reg(W0 + (w as u64) * 4, word);
                }
            }
            CMD_RDSR => {
                // Status register: WIP (busy) = bit 0 = 0 → flash idle.
                self.set_reg(W0, 0x0000_0000);
            }
            CMD_RDID => {
                // JEDEC id: a generic 4 MB part (mfg 0x20, type 0x40, cap 0x16).
                self.set_reg(W0, 0x0016_4020);
            }
            _ => {
                // Unmodeled command: no data, just complete.
            }
        }
        // Clear the USR trigger (and any other command-launch bits) so the
        // firmware's "wait until CMD == 0" poll exits.
        self.set_reg(CMD, 0);
    }
}

impl Peripheral for SpiMemFlash {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.reg(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let mut word = self.reg(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.set_reg(word_off, word);
        // Byte-wise write that sets the USR bit in CMD launches the command.
        if word_off == CMD && (word & USR_BIT) != 0 {
            self.execute_user_command();
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.set_reg(offset & !3, value);
        if (offset & !3) == CMD && (value & USR_BIT) != 0 {
            self.execute_user_command();
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctrl_with(flash: Vec<u8>) -> SpiMemFlash {
        SpiMemFlash::new(Arc::new(Mutex::new(flash)))
    }

    #[test]
    fn read_command_returns_flash_bytes() {
        // flash[0x100..] = 0x11,0x22,0x33,0x44
        let mut flash = vec![0u8; 0x200];
        flash[0x100..0x104].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);
        let mut c = ctrl_with(flash);
        // program: READ (0x03), addr 0x100 (in high bits), 32 bits (4 bytes)
        c.write_u32(USER2, 0x03).unwrap();
        c.write_u32(ADDR, 0x100 << 8).unwrap();
        c.write_u32(MISO_DLEN, 32 - 1).unwrap();
        // launch
        c.write_u32(CMD, USR_BIT).unwrap();
        // CMD auto-cleared (poll would exit)
        assert_eq!(c.read_u32(CMD).unwrap(), 0);
        // W0 holds the 4 bytes little-endian
        assert_eq!(c.read_u32(W0).unwrap(), 0x4433_2211);
    }

    #[test]
    fn rdsr_reports_not_busy() {
        let mut c = ctrl_with(vec![0u8; 0x10]);
        c.write_u32(USER2, CMD_RDSR as u32).unwrap();
        c.write_u32(MISO_DLEN, 8 - 1).unwrap();
        c.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(c.read_u32(W0).unwrap() & 1, 0); // WIP clear
        assert_eq!(c.read_u32(CMD).unwrap(), 0);
    }
}
