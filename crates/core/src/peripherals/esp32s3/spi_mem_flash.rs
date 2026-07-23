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
/// SPI_MEM_USER1_REG: bits[31:27] = SPI_MEM_USR_ADDR_BITLEN (address-phase
/// length minus one). The byte address is left-justified in ADDR, so the
/// right-shift to recover it depends on this width (24-bit phase → >>8,
/// 32-bit phase → >>0). The boot ROM uses a 24-bit phase; MCUboot's
/// `bootloader_flash_read` uses a 32-bit phase, so a hardcoded >>8 misreads
/// every slot0 access by a factor of 256.
const USER1: u64 = 0x1C;
const USER2: u64 = 0x20;
const MISO_DLEN: u64 = 0x28;
/// SPI_MEM_RD_STATUS_REG: holds the flash status register value captured by a
/// dedicated `FLASH_RDSR` / `FLASH_RES` command (bits[15:0]).
const RD_STATUS: u64 = 0x2C;
const W0: u64 = 0x58;
const USR_BIT: u32 = 1 << 18;
/// All command-trigger bits in `SPI_MEM_CMD_REG` (bits [31:17]): the generic
/// user command (`USR`, bit 18) plus the dedicated flash commands the ROM
/// issues directly — `FLASH_RES` (bit 20, resume-from-powerdown), `FLASH_RDSR`
/// (bit 27), `FLASH_RDID` (bit 28), `FLASH_WRDI` (bit 29), `FLASH_WREN`
/// (bit 30), `FLASH_READ` (bit 31), etc. Hardware auto-clears whichever bit
/// launched once the operation completes; firmware busy-polls `CMD == 0`.
const CMD_TRIGGER_MASK: u32 = 0xFFFE_0000;

// Dedicated `SPI_MEM_CMD_REG` flash-command bits.
const FLASH_BE: u32 = 1 << 23; // block erase
const FLASH_SE: u32 = 1 << 24; // sector erase
const FLASH_PP: u32 = 1 << 25; // page program
const FLASH_WRSR: u32 = 1 << 26; // write status register
const FLASH_RDSR: u32 = 1 << 27; // read status register
const FLASH_WRDI: u32 = 1 << 29; // write disable
const FLASH_WREN: u32 = 1 << 30; // write enable

// Flash status-register bits (SPI NOR standard).
const STATUS_WIP: u16 = 1 << 0; // write-in-progress (busy)
const STATUS_WEL: u16 = 1 << 1; // write-enable latch

/// Flash command opcodes the controller emulates (USR-path `USER2` opcode).
const CMD_READ: u8 = 0x03; // READ
const CMD_FAST_READ: u8 = 0x0B; // FAST READ
const CMD_READ_DUAL_OUT: u8 = 0x3B; // Dual Output Fast Read
const CMD_READ_DUAL_IO: u8 = 0xBB; // Dual I/O Fast Read (DIOR) — MCUboot
const CMD_READ_QUAD_OUT: u8 = 0x6B; // Quad Output Fast Read
const CMD_READ_QUAD_IO: u8 = 0xEB; // Quad I/O Fast Read (QIOR)
const CMD_RDSR: u8 = 0x05; // read status register 1 (WIP/WEL)
const CMD_RDSR2: u8 = 0x35; // read status register 2 (QE/…)
const CMD_RDSR3: u8 = 0x15; // read status register 3
const CMD_WRSR: u8 = 0x01; // write status register 1
const CMD_WRSR2: u8 = 0x31; // write status register 2
const CMD_WRSR3: u8 = 0x11; // write status register 3
const CMD_WREN: u8 = 0x06; // write enable
const CMD_WRDI: u8 = 0x04; // write disable
const CMD_SFDP: u8 = 0x5A; // read SFDP
const CMD_RDUID: u8 = 0x4B; // read unique ID (64-bit)
const CMD_RDID: u8 = 0x9F; // read JEDEC id

#[derive(Debug)]
pub struct SpiMemFlash {
    regs: HashMap<u64, u32>,
    flash: Arc<Mutex<Vec<u8>>>,
    /// Modeled flash status register 1 (WIP/WEL). The simulator has no write
    /// latency, so WIP is never set; WEL tracks WREN/WRDI so the bootloader's
    /// WREN→RDSR→check-WEL flash-driver loop makes progress.
    flash_status: u16,
    /// Status register 2 (QE/SRP1/…). Stored so a write-then-read-back matches,
    /// which esp_flash's `set_io_mode` (quad-enable) verification requires.
    sr2: u8,
    /// Status register 3.
    sr3: u8,
}

impl SpiMemFlash {
    pub fn new(flash: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            regs: HashMap::new(),
            flash,
            flash_status: 0,
            sr2: 0,
            sr3: 0,
        }
    }

    /// Shared SPI NOR backing (same buffer SPIMEM0/1 and FlashXip use).
    pub fn flash_backing(&self) -> Arc<Mutex<Vec<u8>>> {
        self.flash.clone()
    }

    fn reg(&self, off: u64) -> u32 {
        self.regs.get(&off).copied().unwrap_or(0)
    }

    fn set_reg(&mut self, off: u64, val: u32) {
        self.regs.insert(off, val);
    }

    /// Launch whichever command bit is set in CMD. The generic `USR` command
    /// reads its opcode from USER2 and may return data in the W buffer; the
    /// dedicated command bits (RES/WRDI/WREN/DP/…) have no data phase the boot
    /// path consumes — they just need to complete. In all cases the command
    /// trigger auto-clears so the firmware's `CMD == 0` poll exits.
    fn launch_command(&mut self) {
        let cmd = self.reg(CMD);
        if cmd & USR_BIT != 0 {
            self.execute_user_command();
            return;
        }
        // Dedicated flash command. Drive the modeled status register so the
        // firmware's WREN→RDSR→poll-WEL / write→RDSR→poll-WIP loops progress.
        if cmd & FLASH_WREN != 0 {
            self.flash_status |= STATUS_WEL;
        }
        if cmd & FLASH_WRDI != 0 {
            self.flash_status &= !STATUS_WEL;
        }
        if cmd & (FLASH_PP | FLASH_SE | FLASH_BE | FLASH_WRSR) != 0 {
            // Program/erase/write-status complete instantly (no write latency);
            // the write-enable latch is consumed, write-in-progress stays clear.
            self.flash_status &= !STATUS_WEL;
            self.flash_status &= !STATUS_WIP;
        }
        if cmd & FLASH_RDSR != 0 {
            // Capture the status register where the firmware reads it back.
            self.set_reg(RD_STATUS, self.flash_status as u32);
        }
        if std::env::var("LABWIRED_SPI_DEBUG").is_ok() {
            eprintln!(
                "spimem1: dedicated cmd CMD=0x{cmd:08x} status=0x{:04x} → complete",
                self.flash_status
            );
        }
        self.set_reg(CMD, 0);
    }

    /// Execute the user command currently programmed in the registers, placing
    /// any read result in the W buffer, then clear the USR trigger.
    fn execute_user_command(&mut self) {
        let cmd = (self.reg(USER2) & 0xFFFF) as u8;
        // ESP32-S3 SPI_MEM_ADDR_REG holds the flash byte address right-justified
        // (the controller sends the low USER1[31:27]+1 bits MSB-first during the
        // address phase). So the byte address is the register value as-is — NOT a
        // >>8 of a left-justified field. The boot ROM happens to probe only
        // zero-address commands (RDID/RDSR), so the earlier >>8 was never
        // exercised; MCUboot's bootloader_flash_read of slot0 (ADDR_REG=0x00010000
        // for flash 0x10000) is the first real data read and exposed it. Unused
        // high bits are already zero for in-range addresses, and an out-of-range
        // offset reads back 0xFF (flash.get bounds-checks), so no masking is
        // needed. (USER1 carries the address-phase width if a future model wants
        // to validate it.)
        let _ = USER1;
        let addr_reg = self.reg(ADDR);
        let addr = addr_reg as usize;
        // MISO_DLEN holds (bits - 1); bytes = (bits)/8.
        let read_bytes = ((self.reg(MISO_DLEN) as usize) + 1) / 8;
        let debug = std::env::var("LABWIRED_SPI_DEBUG").is_ok();
        if debug {
            let f = self.flash.lock().unwrap();
            let peek: Vec<u8> = (0..4)
                .map(|i| f.get(addr + i).copied().unwrap_or(0xFF))
                .collect();
            eprintln!(
                "spimem1: cmd=0x{cmd:02x} ADDR_REG=0x{addr_reg:08x} addr=0x{addr:06x} bytes={read_bytes} flash[addr..]={peek:02x?}"
            );
        }

        match cmd {
            CMD_READ | CMD_FAST_READ | CMD_READ_DUAL_OUT | CMD_READ_DUAL_IO | CMD_READ_QUAD_OUT
            | CMD_READ_QUAD_IO => {
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
            // Status register 1 (WIP/WEL) — must reflect the modeled status so
            // the app's esp_flash WREN→RDSR→check-WEL and write-completion polls
            // work through the USR path (not just the dedicated CMD bits).
            CMD_RDSR => self.set_reg(W0, self.flash_status as u32),
            // Status registers 2/3 — return the stored values so a write-then-
            // read-back is coherent (esp_flash's set_io_mode quad-enable verify).
            CMD_RDSR2 => self.set_reg(W0, self.sr2 as u32),
            CMD_RDSR3 => self.set_reg(W0, self.sr3 as u32),
            CMD_RDID => {
                // JEDEC id: a generic 4 MB part (mfg 0x20, type 0x40, cap 0x16).
                self.set_reg(W0, 0x0016_4020);
            }
            // Read 64-bit unique ID. esp_flash rejects all-zero / all-ones as
            // ESP_ERR_INVALID_RESPONSE, so return a fixed non-trivial value.
            CMD_RDUID => {
                self.set_reg(W0, 0x1716_1514);
                self.set_reg(W0 + 4, 0x1F1E_1D1C);
            }
            // SFDP: no parameter table modeled — return zeros so esp_flash falls
            // back to RDID-based detection rather than reading stale data.
            CMD_SFDP => {
                for i in 0..read_bytes.max(1) {
                    self.set_reg(W0 + (i as u64 / 4) * 4, 0);
                }
            }
            // Write-enable / disable through the USR path (mirrors the dedicated
            // bits) so WEL is coherent regardless of which path the app uses.
            CMD_WREN => self.flash_status |= STATUS_WEL,
            CMD_WRDI => self.flash_status &= !STATUS_WEL,
            // Write status register: store the written value so the read-back
            // verification matches, then complete (consume WEL, WIP clear).
            // SR1 16-bit write (0x01) carries SR2 in the 2nd byte (W0[15:8]);
            // 0x31 writes SR2 directly, 0x11 writes SR3.
            CMD_WRSR => {
                self.sr2 = ((self.reg(W0) >> 8) & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
            }
            CMD_WRSR2 => {
                self.sr2 = (self.reg(W0) & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
            }
            CMD_WRSR3 => {
                self.sr3 = (self.reg(W0) & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
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
    // Inert walk: flash commands execute atomically at the launching CMD write (trigger bits auto-clear there); tick() is the trait-default no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

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
        // Byte-wise write that sets any command-trigger bit in CMD launches it.
        if word_off == CMD && (word & CMD_TRIGGER_MASK) != 0 {
            self.launch_command();
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        Ok(self.reg(offset & !3))
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        self.set_reg(offset & !3, value);
        if (offset & !3) == CMD && (value & CMD_TRIGGER_MASK) != 0 {
            self.launch_command();
        }
        Ok(())
    }

    fn legacy_tick_active(&self) -> bool {
        false
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
        // program: READ (0x03), addr 0x100 (right-justified byte address),
        // 32 bits (4 bytes)
        c.write_u32(USER2, 0x03).unwrap();
        c.write_u32(ADDR, 0x100).unwrap();
        c.write_u32(MISO_DLEN, 32 - 1).unwrap();
        // launch
        c.write_u32(CMD, USR_BIT).unwrap();
        // CMD auto-cleared (poll would exit)
        assert_eq!(c.read_u32(CMD).unwrap(), 0);
        // W0 holds the 4 bytes little-endian
        assert_eq!(c.read_u32(W0).unwrap(), 0x4433_2211);
    }

    #[test]
    fn dual_io_read_uses_raw_address_for_mcuboot_slot0() {
        // Regression for booting MCUboot/Zephyr: bootloader_flash_read issues a
        // Dual I/O Fast Read (0xBB) of slot0 with ADDR_REG = the raw byte address
        // 0x10000 (right-justified, NOT 0x10000<<8). The earlier model both (a)
        // right-shifted the address by 8 (reading 0x100) and (b) didn't decode
        // 0xBB at all (returning a stale W0), so MCUboot saw a bad image magic.
        let mut flash = vec![0u8; 0x10100];
        flash[0x10000..0x10004].copy_from_slice(&[0x3d, 0xb8, 0xf3, 0x96]); // MCUboot magic
        let mut c = ctrl_with(flash);
        c.write_u32(USER2, CMD_READ_DUAL_IO as u32).unwrap();
        c.write_u32(ADDR, 0x0001_0000).unwrap();
        c.write_u32(MISO_DLEN, 32 - 1).unwrap();
        c.write_u32(CMD, USR_BIT).unwrap();
        assert_eq!(
            c.read_u32(W0).unwrap(),
            0x96f3_b83d,
            "0xBB read of slot0 must return the real MCUboot magic, not a stale/shifted value"
        );
    }

    #[test]
    fn dedicated_command_bits_auto_clear() {
        let mut c = ctrl_with(vec![0u8; 0x10]);
        // FLASH_RES (bit 20) — resume from power-down; no data phase. The ROM
        // writes it and polls CMD until zero.
        c.write_u32(CMD, 1 << 20).unwrap();
        assert_eq!(c.read_u32(CMD).unwrap(), 0, "FLASH_RES must auto-clear");
        // FLASH_WRDI (bit 29).
        c.write_u32(CMD, 1 << 29).unwrap();
        assert_eq!(c.read_u32(CMD).unwrap(), 0, "FLASH_WRDI must auto-clear");
    }

    #[test]
    fn wren_then_rdsr_sets_wel_and_clears_wip() {
        let mut c = ctrl_with(vec![0u8; 0x10]);
        // FLASH_WREN (bit 30): sets the write-enable latch.
        c.write_u32(CMD, FLASH_WREN).unwrap();
        // FLASH_RDSR (bit 27): captures status into RD_STATUS (0x2C).
        c.write_u32(CMD, FLASH_RDSR).unwrap();
        let status = c.read_u32(RD_STATUS).unwrap();
        assert_eq!(
            status & STATUS_WEL as u32,
            STATUS_WEL as u32,
            "WEL must be set"
        );
        assert_eq!(
            status & STATUS_WIP as u32,
            0,
            "WIP must be clear (not busy)"
        );
        // FLASH_WRDI (bit 29): clears the latch.
        c.write_u32(CMD, FLASH_WRDI).unwrap();
        c.write_u32(CMD, FLASH_RDSR).unwrap();
        assert_eq!(
            c.read_u32(RD_STATUS).unwrap() & STATUS_WEL as u32,
            0,
            "WEL cleared"
        );
    }

    #[test]
    fn program_completes_and_consumes_wel() {
        let mut c = ctrl_with(vec![0u8; 0x10]);
        c.write_u32(CMD, FLASH_WREN).unwrap();
        c.write_u32(CMD, FLASH_PP).unwrap(); // page program completes instantly
        c.write_u32(CMD, FLASH_RDSR).unwrap();
        let status = c.read_u32(RD_STATUS).unwrap();
        assert_eq!(status & STATUS_WIP as u32, 0, "WIP clear after program");
        assert_eq!(status & STATUS_WEL as u32, 0, "WEL consumed by program");
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
