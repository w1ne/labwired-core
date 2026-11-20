// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NPCX9 Flash Interface Unit (FIU) user-mode access path.
//!
//! The FIU's 1 KiB byte register window follows Renode `NPCX_FIU.cs` and the
//! internal flash command behavior follows the subset implemented by Renode's
//! `NPCX_Flash.cs`. The 1 MiB medium is sparse and erased (`0xFF`) at reset.
//! UMA supports write-enable/disable, status reads/writes, 3/4-byte addressing,
//! page program, read data, and 64 KiB block erase. Reads and writes use the
//! flash's 1-to-0 programming rule and page wrap.
//!
//! The direct XIP window Renode maps at `0x64000000` is deliberately not
//! declared yet: it must share bytes with the FIU medium before we can claim
//! address-mapped flash access. This model proves transactions through UMA.
//!
//! Not modelled: burst reads, CRC, Quad-SPI, FIU timing, protection pins and
//! direct XIP reads. Renode's NPCX_Flash model also does not implement the
//! standard READ DATA opcode; this model adds it so firmware can verify that a
//! programmed byte and an erased byte are observable through UMA.

use crate::{Peripheral, SimResult};
use std::cell::RefCell;
use std::collections::BTreeMap;

const UMACODE: u64 = 0x16;
const UMA_ADDR0: u64 = 0x17;
const UMA_DATA0: u64 = 0x1A;
const UMA_CTS: u64 = 0x1E;
const UMA_ECTS: u64 = 0x1F;
const UMA_DATA1: u64 = 0x20;
const UMA_ADDR1: u64 = 0x34;
const SPI1_DEV: u64 = 0x3D;

const FLASH_SIZE_DEFAULT: usize = 0x10_0000;
const WRITE_ENABLE: u8 = 0x06;
const WRITE_DISABLE: u8 = 0x04;
const READ_STATUS1: u8 = 0x05;
const READ_STATUS2: u8 = 0x35;
const WRITE_STATUS: u8 = 0x01;
const READ_DATA: u8 = 0x03;
const PAGE_PROGRAM: u8 = 0x02;
const BLOCK_ERASE_64K: u8 = 0xD8;

struct UmaTransaction {
    code: u8,
    address: u32,
    address_width: usize,
    data_width: usize,
    is_write: bool,
    skip_code: bool,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct FlashState {
    write_enabled: bool,
    reset_write_enable_at_end: bool,
    status1: u8,
    status2: u8,
    command: Option<u8>,
    address_bytes: Vec<u8>,
    address_bytes_expected: usize,
    address: u32,
    status_index: u8,
    bytes: BTreeMap<u32, u8>,
}

impl FlashState {
    fn reset(&mut self) {
        self.write_enabled = false;
        self.reset_write_enable_at_end = false;
        self.command = None;
        self.address_bytes.clear();
        self.address_bytes_expected = 0;
        self.address = 0;
        self.status_index = 0;
    }

    fn status1(&self) -> u8 {
        (self.status1 & !0x02) | (u8::from(self.write_enabled) << 1)
    }

    fn transfer(&mut self, value: u8, flash_size: usize) -> u8 {
        let Some(command) = self.command else {
            self.command = Some(value);
            match value {
                WRITE_ENABLE => self.write_enabled = true,
                WRITE_DISABLE => self.write_enabled = false,
                _ => {}
            }
            return 0;
        };

        match command {
            READ_STATUS1 => self.status1(),
            READ_STATUS2 => self.status2,
            READ_DATA | PAGE_PROGRAM | BLOCK_ERASE_64K => {
                if self.address_bytes.len() < self.address_bytes_expected {
                    self.address_bytes.push(value);
                    self.address = self
                        .address_bytes
                        .iter()
                        .enumerate()
                        .fold(0, |address, (index, byte)| {
                            address | (u32::from(*byte) << (index * 8))
                        });
                    if command == BLOCK_ERASE_64K
                        && self.address_bytes.len() == 3
                        && self.write_enabled
                    {
                        let start = self.address as usize;
                        let end = start.saturating_add(0x1_0000).min(flash_size);
                        self.bytes.retain(|address, _| {
                            !(*address as usize >= start && (*address as usize) < end)
                        });
                        self.write_enabled = false;
                    }
                    return 0;
                }

                match command {
                    READ_DATA => {
                        let byte = self.bytes.get(&self.address).copied().unwrap_or(0xFF);
                        self.address = self.address.wrapping_add(1);
                        byte
                    }
                    PAGE_PROGRAM => {
                        if self.write_enabled && (self.address as usize) < flash_size {
                            let current = self.bytes.get(&self.address).copied().unwrap_or(0xFF);
                            let programmed = current & value;
                            if programmed == 0xFF {
                                self.bytes.remove(&self.address);
                            } else {
                                self.bytes.insert(self.address, programmed);
                            }
                            let page = self.address & !0xFF;
                            self.address = if self.address & 0xFF == 0xFF {
                                page
                            } else {
                                self.address + 1
                            };
                            self.reset_write_enable_at_end = true;
                        }
                        0
                    }
                    _ => 0,
                }
            }
            WRITE_STATUS => {
                if self.write_enabled {
                    if self.status_index == 0 {
                        // WEL is controlled by WRITE ENABLE / DISABLE.
                        self.status1 = value & !0x02;
                    } else if self.status_index == 1 {
                        self.status2 = value;
                    }
                    self.status_index = self.status_index.saturating_add(1);
                    self.reset_write_enable_at_end = true;
                }
                0
            }
            _ => 0,
        }
    }

    fn finish(&mut self) {
        self.command = None;
        self.address_bytes.clear();
        self.address = 0;
        self.status_index = 0;
        if self.reset_write_enable_at_end {
            self.write_enabled = false;
        }
        self.reset_write_enable_at_end = false;
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct Registers {
    code: u8,
    address: u32,
    data: u32,
    cts: u8,
    ects: u8,
    spi1_dev: u8,
    locked: bool,
}

impl Default for Registers {
    fn default() -> Self {
        Self {
            code: 0,
            address: 0,
            data: 0,
            cts: 0x40,
            ects: 0x03,
            spi1_dev: 0,
            locked: false,
        }
    }
}

/// NPCX9 FIU and its internal flash command endpoint.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct NpcxFiu {
    regs: RefCell<Registers>,
    flash: RefCell<FlashState>,
    flash_size: usize,
}

impl Default for NpcxFiu {
    fn default() -> Self {
        Self::new(FLASH_SIZE_DEFAULT)
    }
}

impl NpcxFiu {
    pub fn new(flash_size: usize) -> Self {
        Self {
            regs: RefCell::new(Registers::default()),
            flash: RefCell::new(FlashState::default()),
            flash_size,
        }
    }

    fn read_byte(&self, offset: u64) -> u8 {
        let regs = self.regs.borrow();
        match offset {
            UMACODE => regs.code,
            UMA_ADDR0..=0x19 => ((regs.address >> ((offset - UMA_ADDR0) * 8)) & 0xFF) as u8,
            UMA_DATA0..=0x1D => ((regs.data >> ((offset - UMA_DATA0) * 8)) & 0xFF) as u8,
            UMA_CTS => regs.cts,
            UMA_ECTS => (regs.ects & 0xF7) | (u8::from(regs.locked) << 3),
            UMA_DATA1..=0x23 => ((regs.data >> ((offset - UMA_DATA1) * 8)) & 0xFF) as u8,
            UMA_ADDR1..=0x37 => ((regs.address >> ((offset - UMA_ADDR1) * 8)) & 0xFF) as u8,
            SPI1_DEV => regs.spi1_dev & 0xC0,
            _ => 0,
        }
    }

    fn write_byte(&self, offset: u64, value: u8) {
        let mut regs = self.regs.borrow_mut();
        match offset {
            UMACODE => regs.code = value,
            UMA_ADDR0..=0x19 => {
                let shift = (offset - UMA_ADDR0) * 8;
                regs.address = (regs.address & !(0xFF << shift)) | (u32::from(value) << shift);
            }
            UMA_DATA0..=0x1D => {
                let shift = (offset - UMA_DATA0) * 8;
                regs.data = (regs.data & !(0xFF << shift)) | (u32::from(value) << shift);
            }
            UMA_CTS => {
                regs.cts = value & 0x7F;
                if value & 0x80 != 0 && !regs.locked {
                    let data_width = (value & 0x07) as usize;
                    let is_write = value & (1 << 5) != 0;
                    // NPCX_FIU.cs chooses the 3/4-byte length from SPI1_DEV
                    // when CTS.A_SIZE is set; otherwise the width is zero.
                    let address_width = if value & (1 << 3) != 0 {
                        if regs.spi1_dev & 0xC0 != 0 {
                            4
                        } else {
                            3
                        }
                    } else {
                        0
                    };
                    let skip_code = value & (1 << 4) != 0 && !is_write;
                    let code = regs.code;
                    let address = regs.address;
                    let mut data = regs.data;
                    drop(regs);
                    self.execute(
                        UmaTransaction {
                            code,
                            address,
                            address_width,
                            data_width,
                            is_write,
                            skip_code,
                        },
                        &mut data,
                    );
                    regs = self.regs.borrow_mut();
                    if !is_write {
                        regs.data = data;
                    }
                }
            }
            UMA_ECTS => {
                if !regs.locked {
                    regs.ects = (value & 0x77) | 0x01;
                    regs.locked = value & (1 << 3) != 0;
                }
                if value & (1 << 1) == 0 {
                    drop(regs);
                    self.flash.borrow_mut().finish();
                }
            }
            UMA_ADDR1..=0x37 => {
                let shift = (offset - UMA_ADDR1) * 8;
                regs.address = (regs.address & !(0xFF << shift)) | (u32::from(value) << shift);
            }
            SPI1_DEV => {
                regs.spi1_dev = value & 0xC0;
            }
            _ => {}
        }
    }

    fn execute(&self, transaction: UmaTransaction, data: &mut u32) {
        let UmaTransaction {
            code,
            address,
            address_width,
            data_width,
            is_write,
            skip_code,
        } = transaction;
        let mut flash = self.flash.borrow_mut();
        flash.address_bytes_expected = address_width;
        if !skip_code || is_write {
            flash.transfer(code, self.flash_size);
        }
        for index in 0..address_width {
            flash.transfer((address >> (index * 8)) as u8, self.flash_size);
        }
        for index in 0..data_width {
            let input = (*data >> (index * 8)) as u8;
            let output = flash.transfer(input, self.flash_size);
            if !is_write {
                let mask = !(0xFFu32 << (index * 8));
                *data = (*data & mask) | (u32::from(output) << (index * 8));
            }
        }
        let software_chip_select = self.regs.borrow().ects & (1 << 1) != 0;
        if software_chip_select {
            flash.finish();
        }
    }

    fn reset_registers(&mut self) {
        *self.regs.get_mut() = Registers::default();
        self.flash.get_mut().reset();
    }
}

impl Peripheral for NpcxFiu {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.read_byte(offset))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_byte(offset, value);
        Ok(())
    }

    /// FIU state changes only on MMIO transactions; its periodic walk is a
    /// no-op and must not pin WFI in the legacy idle path.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn on_system_reset(&mut self) {
        self.reset_registers();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(fiu: &mut NpcxFiu, offset: u64, value: u8) {
        fiu.write(offset, value).unwrap();
    }

    fn read(fiu: &NpcxFiu, offset: u64) -> u8 {
        fiu.read(offset).unwrap()
    }

    fn execute(fiu: &mut NpcxFiu, code: u8, address: Option<u32>, data: u8, cts: u8) -> u8 {
        write(fiu, UMACODE, code);
        if let Some(address) = address {
            for i in 0..3 {
                write(fiu, UMA_ADDR0 + i, (address >> (i * 8)) as u8);
            }
        }
        write(fiu, UMA_DATA0, data);
        write(fiu, UMA_CTS, cts | 0x80);
        read(fiu, UMA_DATA0)
    }

    #[test]
    fn reset_values_match_renode_register_file() {
        let fiu = NpcxFiu::new(0x10_0000);
        assert_eq!(read(&fiu, UMA_CTS), 0x40);
        assert_eq!(read(&fiu, UMA_ECTS), 0x03);
        assert_eq!(read(&fiu, UMACODE), 0);
    }

    #[test]
    fn uma_program_read_nor_rule_and_block_erase() {
        let mut fiu = NpcxFiu::new(0x10_0000);
        // WREN then status read exposes WEL.
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        assert_eq!(execute(&mut fiu, READ_STATUS1, None, 0, 1) & 0x02, 0x02);

        // Program 0x5A at 0x100, using the FIU's low-byte-first address regs.
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        write(&mut fiu, UMACODE, PAGE_PROGRAM);
        write(&mut fiu, UMA_ADDR0, 0x00);
        write(&mut fiu, UMA_ADDR0 + 1, 0x01);
        write(&mut fiu, UMA_ADDR0 + 2, 0x00);
        write(&mut fiu, UMA_DATA0, 0x5A);
        write(&mut fiu, UMA_CTS, 0x80 | 0x20 | 0x08 | 1);
        assert_eq!(execute(&mut fiu, READ_DATA, Some(0x100), 0, 0x08 | 1), 0x5A);
        assert_eq!(execute(&mut fiu, READ_STATUS1, None, 0, 1) & 0x02, 0);

        // Programming all ones cannot change an already-cleared bit.
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        write(&mut fiu, UMACODE, PAGE_PROGRAM);
        write(&mut fiu, UMA_ADDR0, 0x00);
        write(&mut fiu, UMA_ADDR0 + 1, 0x01);
        write(&mut fiu, UMA_ADDR0 + 2, 0x00);
        write(&mut fiu, UMA_DATA0, 0xFF);
        write(&mut fiu, UMA_CTS, 0x80 | 0x20 | 0x08 | 1);
        assert_eq!(execute(&mut fiu, READ_DATA, Some(0x100), 0, 0x08 | 1), 0x5A);

        // Renode's supported block erase erases 64 KiB from the given address.
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        execute(&mut fiu, BLOCK_ERASE_64K, Some(0), 0, 0x08);
        assert_eq!(execute(&mut fiu, READ_DATA, Some(0x100), 0, 0x08 | 1), 0xFF);
    }

    #[test]
    fn uma_lock_blocks_transactions_until_reset() {
        let mut fiu = NpcxFiu::new(0x10_0000);
        write(&mut fiu, UMA_ECTS, 0x0B);
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        assert_eq!(execute(&mut fiu, READ_STATUS1, None, 0, 1) & 0x02, 0);
        fiu.on_system_reset();
        execute(&mut fiu, WRITE_ENABLE, None, 0, 0);
        assert_eq!(execute(&mut fiu, READ_STATUS1, None, 0, 1) & 0x02, 0x02);
    }
}
