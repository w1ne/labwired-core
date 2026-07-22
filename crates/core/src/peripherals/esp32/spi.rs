// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic SPI controller (SPI0/SPI1 flash + HSPI/VSPI).
//!
//! Models the register window used by:
//!   * Arduino / bare-metal MOSI streams to displays (SPI2/SPI3), and
//!   * SPI flash command traffic on SPI0/SPI1 (JEDEC RDID, RDSR, WREN, …)
//!     that `esp_flash_init_main` / `bootloader_read_flash_id` issue.
//!
//! Reference: ESP32 TRM v4.6 §7 (SPI Controller), register layout Table 7-3
//! and `soc/esp32/register/soc/spi_reg.h`. Offsets match classic ESP32
//! (NOT ESP32-S3, whose USER block sits 4 bytes higher and whose W buffer
//! starts at 0x58).
//!
//! Register window is 0x100 bytes wide. Modeled fields:
//!   * SPI_CMD_REG (0x00)        — USR (bit 18) + dedicated FLASH_* bits;
//!                                 all trigger bits auto-clear on completion.
//!   * SPI_ADDR_REG (0x04)       — flash byte address for USR address phase.
//!   * SPI_RD_STATUS_REG (0x10)  — result of dedicated FLASH_RDID / FLASH_RDSR.
//!   * SPI_USER_REG (0x1C)       — USR_MOSI / USR_MISO / USR_COMMAND bits.
//!   * SPI_USER2_REG (0x24)      — command opcode (bits[15:0]) + bitlen.
//!   * SPI_MOSI_DLEN_REG (0x28)  — bit length minus 1; sizes MOSI stream.
//!   * SPI_MISO_DLEN_REG (0x2C)  — bit length minus 1; sizes MISO fill.
//!   * SPI_W0..W15 (0x80..0xBC)  — 64-byte FIFO; little-endian byte order.
//!   * Other offsets             — round-tripped (firmware RMW / polls).
//!
//! On `CMD_REG` write with bit 18 (USR) set, the peripheral synchronously:
//!   1. If USR_MOSI: streams MOSI bytes from the FIFO to attached devices.
//!   2. If USR_MISO: fills the FIFO from the modeled flash-command response
//!      (JEDEC id, status, optional READ of flash backing).
//!   3. Clears CMD so firmware's busy-poll (`CMD == 0`) exits.
//!
//! Dedicated FLASH_* command bits (RDID/RDSR/WREN/…) complete the same way
//! and deposit results in `SPI_RD_STATUS` where the ROM/driver expects them.
//!
//! Multi-device CS-arbitration is intentionally not modeled — the same
//! simplification as `peripherals::spi::Spi`. For the e-paper lab one
//! SSD1680 panel is attached on SPI3.

use crate::peripherals::spi::SpiDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const REG_CMD: u64 = 0x00;
const REG_ADDR: u64 = 0x04;
/// Classic SPI_RD_STATUS_REG — dedicated FLASH_RDID / FLASH_RDSR land here.
const REG_RD_STATUS: u64 = 0x10;
pub const REG_USER: u64 = 0x1C;
const REG_USER2: u64 = 0x24;
const REG_MOSI_DLEN: u64 = 0x28;
const REG_MISO_DLEN: u64 = 0x2C;
const FIFO_START: u64 = 0x80;
const FIFO_END: u64 = 0xC0; // exclusive — W0..W15 = 64 bytes

const CMD_USR_BIT: u32 = 1 << 18;
/// All CMD bits that launch an operation and auto-clear (bits [31:16]:
/// FLASH_READ..FLASH_PER plus USR). Firmware busy-polls until CMD == 0.
const CMD_TRIGGER_MASK: u32 = 0xFFFF_0000;

// Dedicated SPI_CMD_REG flash-command bits (classic ESP32 / SPI1).
const FLASH_RES: u32 = 1 << 20;
const FLASH_BE: u32 = 1 << 23;
const FLASH_SE: u32 = 1 << 24;
const FLASH_PP: u32 = 1 << 25;
const FLASH_WRSR: u32 = 1 << 26;
const FLASH_RDSR: u32 = 1 << 27;
const FLASH_RDID: u32 = 1 << 28;
const FLASH_WRDI: u32 = 1 << 29;
const FLASH_WREN: u32 = 1 << 30;
const FLASH_READ: u32 = 1 << 31;

pub const USER_USR_MOSI_BIT: u32 = 1 << 27;
const USER_USR_MISO_BIT: u32 = 1 << 28;
const USER_USR_COMMAND_BIT: u32 = 1 << 31;

// SPI NOR opcodes used by esp_flash / bootloader flash paths.
const CMD_READ: u8 = 0x03;
const CMD_FAST_READ: u8 = 0x0B;
const CMD_DREAD: u8 = 0x3B;
const CMD_QREAD: u8 = 0x6B;
const CMD_DIO_READ: u8 = 0xBB;
const CMD_QIO_READ: u8 = 0xEB;
const CMD_RDSR: u8 = 0x05;
const CMD_RDSR2: u8 = 0x35;
const CMD_RDSR3: u8 = 0x15;
const CMD_WRSR: u8 = 0x01;
const CMD_WRSR2: u8 = 0x31;
const CMD_WRSR3: u8 = 0x11;
const CMD_WREN: u8 = 0x06;
const CMD_WRDI: u8 = 0x04;
const CMD_SFDP: u8 = 0x5A;
const CMD_RDUID: u8 = 0x4B;
const CMD_RDID: u8 = 0x9F;
const CMD_PAGE_PROGRAM: u8 = 0x02;
const CMD_SECTOR_ERASE: u8 = 0x20; // 4 KiB
const CMD_BLOCK_ERASE_32K: u8 = 0x52;
const CMD_BLOCK_ERASE_64K: u8 = 0xD8;
const CMD_CHIP_ERASE: u8 = 0xC7;

// Flash status-register bits (SPI NOR standard).
const STATUS_WIP: u16 = 1 << 0;
const STATUS_WEL: u16 = 1 << 1;

/// JEDEC id returned for RDID: Winbond W25Q32-class (mfg 0xEF, type 0x40,
/// capacity 0x16 = 4 MiB). Matches the post-BROM `g_rom_flashchip` seed and
/// the bytes `memspi_host_read_id_hs` expects on the MISO wire (little-endian
/// in W0: 0xEF, 0x40, 0x16 → word 0x0016_40EF).
const JEDEC_ID: u32 = 0x0016_40EF;

#[derive(Default)]
pub struct Esp32Spi {
    cmd: u32,
    addr: u32,
    rd_status: u32,
    user: u32,
    user2: u32,
    mosi_dlen: u32,
    miso_dlen: u32,
    fifo: [u32; 16],
    /// Round-trip every other offset so firmware RMW sequences observe their writes.
    other: HashMap<u64, u32>,

    /// Modeled flash status register 1 (WIP/WEL). Instant-complete model:
    /// WIP never stays set; WEL tracks WREN/WRDI.
    flash_status: u16,
    sr2: u8,
    sr3: u8,
    /// Optional flash array for USR READ/FAST_READ (and dedicated FLASH_READ).
    /// When absent, READ returns 0xFF bytes (erased flash).
    flash: Option<Arc<Mutex<Vec<u8>>>>,

    pub attached_devices: Vec<Box<dyn SpiDevice>>,

    /// Optional capture of every byte streamed via CMD.USR. Off by default;
    /// enable with `record_bytes()` so tests can inspect the wire trace
    /// without reaching into the attached device.
    record_enabled: bool,
    captured_bytes: Vec<u8>,
    transactions: u64,
}

impl std::fmt::Debug for Esp32Spi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Esp32Spi(cmd=0x{:08x} user=0x{:08x} mosi_dlen={} attached={})",
            self.cmd,
            self.user,
            self.mosi_dlen,
            self.attached_devices.len()
        )
    }
}

impl Esp32Spi {
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a shared flash backing so USR READ/FAST_READ (and dedicated
    /// FLASH_READ) return real bytes. SPI0/SPI1 flash controllers can share
    /// the same backing the XIP windows use.
    pub fn set_flash_backing(&mut self, flash: Arc<Mutex<Vec<u8>>>) {
        self.flash = Some(flash);
    }

    /// Raw device push — does NOT wrap for tracing. The only production caller
    /// is the bus choke point [`crate::bus::SystemBus::attach_spi_device`],
    /// which wraps first.
    pub(crate) fn push_device(&mut self, device: Box<dyn SpiDevice>) {
        self.attached_devices.push(device);
    }

    /// Enable wire-byte capture. Every byte streamed via CMD.USR after this
    /// call is appended to an internal buffer; `cap` caps memory by dropping
    /// older bytes once the buffer reaches that length.
    pub fn enable_byte_capture(&mut self, cap: usize) {
        self.record_enabled = true;
        self.captured_bytes.reserve(cap);
    }

    pub fn captured_bytes(&self) -> &[u8] {
        &self.captured_bytes
    }

    pub fn transactions(&self) -> u64 {
        self.transactions
    }

    fn read_word(&self, off: u64) -> u32 {
        match off {
            REG_CMD => self.cmd,
            REG_ADDR => self.addr,
            REG_RD_STATUS => self.rd_status,
            REG_USER => self.user,
            REG_USER2 => self.user2,
            REG_MOSI_DLEN => self.mosi_dlen,
            REG_MISO_DLEN => self.miso_dlen,
            off if (FIFO_START..FIFO_END).contains(&off) => {
                self.fifo[((off - FIFO_START) / 4) as usize]
            }
            other => self.other.get(&other).copied().unwrap_or(0),
        }
    }

    fn write_word(&mut self, off: u64, value: u32) {
        match off {
            REG_CMD => {
                self.cmd = value;
                if value & CMD_TRIGGER_MASK != 0 {
                    self.launch_command();
                } else {
                    // Lower CMD bits are reserved; never leave a sticky non-zero
                    // value that would hang `while (SPI_CMD_REG != 0)` polls.
                    self.cmd = 0;
                }
            }
            REG_ADDR => self.addr = value,
            REG_RD_STATUS => self.rd_status = value,
            REG_USER => self.user = value,
            REG_USER2 => self.user2 = value,
            REG_MOSI_DLEN => self.mosi_dlen = value,
            REG_MISO_DLEN => self.miso_dlen = value,
            off if (FIFO_START..FIFO_END).contains(&off) => {
                self.fifo[((off - FIFO_START) / 4) as usize] = value;
            }
            other => {
                self.other.insert(other, value);
            }
        }
    }

    /// Launch USR or a dedicated FLASH_* command; always auto-clear trigger bits.
    fn launch_command(&mut self) {
        let cmd = self.cmd;
        if cmd & CMD_USR_BIT != 0 {
            self.kick_user_transaction();
            return;
        }
        // Dedicated flash command bits (SPI1 flash controller / ROM helpers).
        if cmd & FLASH_WREN != 0 {
            self.flash_status |= STATUS_WEL;
        }
        if cmd & FLASH_WRDI != 0 {
            self.flash_status &= !STATUS_WEL;
        }
        let addr = (self.addr >> 8) as usize;
        if cmd & FLASH_PP != 0 {
            // Page program: W0.. holds payload; mosi_dlen is bit length.
            let nbytes = ((((self.mosi_dlen & 0x7FF) + 1) / 8) as usize).max(1);
            self.program_flash(addr, nbytes);
            self.flash_status &= !(STATUS_WEL | STATUS_WIP);
        }
        if cmd & FLASH_SE != 0 {
            self.erase_flash(addr, 4 * 1024);
            self.flash_status &= !(STATUS_WEL | STATUS_WIP);
        }
        if cmd & FLASH_BE != 0 {
            self.erase_flash(addr, 64 * 1024);
            self.flash_status &= !(STATUS_WEL | STATUS_WIP);
        }
        if cmd & FLASH_WRSR != 0 {
            self.flash_status &= !(STATUS_WEL | STATUS_WIP);
        }
        if cmd & FLASH_RDSR != 0 {
            self.rd_status = self.flash_status as u32;
        }
        if cmd & FLASH_RDID != 0 {
            // TRM: device id is read out to SPI_RD_STATUS.
            self.rd_status = JEDEC_ID;
        }
        if cmd & FLASH_READ != 0 {
            // Dedicated FLASH_READ deposits data in W0..; address in ADDR.
            let nbytes = (((self.miso_dlen & 0x7FF) + 1) / 8) as usize;
            self.fill_fifo_from_flash(addr, nbytes.max(1));
        }
        if cmd & FLASH_RES != 0 {
            // Resume-from-powerdown: no data the boot path consumes.
        }
        if std::env::var("LABWIRED_SPI_DEBUG").is_ok() {
            eprintln!(
                "esp32_spi: dedicated cmd=0x{cmd:08x} status=0x{:04x} rd_status=0x{:08x}",
                self.flash_status, self.rd_status
            );
        }
        self.cmd = 0;
    }

    /// Run one user-defined transaction synchronously.
    ///
    /// MOSI path: drain FIFO bytes to every attached `SpiDevice` (display /
    /// external SPI). CS is **not** pulsed per CMD.USR — real firmware holds
    /// CS via GPIO across many transfers.
    ///
    /// MISO path: fill FIFO from the modeled SPI-NOR response for the opcode
    /// in USER2 (RDID/RDSR/READ/…). Used by SPI0/SPI1 flash init.
    fn kick_user_transaction(&mut self) {
        self.transactions += 1;

        // USR MOSI → attached devices only. Flash array program/erase is handled
        // by dedicated FLASH_PP/SE/BE bits in `launch_command` (not USR opcodes).
        if self.user & USER_USR_MOSI_BIT != 0 {
            let bits = (self.mosi_dlen & 0x7FF) + 1;
            let byte_count = bits.div_ceil(8) as usize;
            for i in 0..byte_count {
                let word = self.fifo[i / 4];
                let byte = ((word >> ((i % 4) * 8)) & 0xFF) as u8;
                if self.record_enabled {
                    self.captured_bytes.push(byte);
                }
                for dev in &mut self.attached_devices {
                    dev.transfer(byte);
                }
            }
        }

        if self.user & USER_USR_MISO_BIT != 0 {
            self.execute_flash_user_miso();
        }

        // Clear every command-launch bit so poll-until-CMD==0 exits.
        self.cmd = 0;
    }

    /// Fill W0.. from the flash opcode in USER2 (bits[15:0]).
    fn execute_flash_user_miso(&mut self) {
        let opcode = if self.user & USER_USR_COMMAND_BIT != 0 {
            (self.user2 & 0xFFFF) as u8
        } else {
            (self.user2 & 0xFFFF) as u8
        };
        let read_bytes = (((self.miso_dlen & 0x7FF) + 1) / 8) as usize;
        let addr = (self.addr >> 8) as usize;

        if std::env::var("LABWIRED_SPI_DEBUG").is_ok() {
            eprintln!(
                "esp32_spi: USR miso op=0x{opcode:02x} addr=0x{addr:06x} bytes={read_bytes} user2=0x{:08x}",
                self.user2
            );
        }

        match opcode {
            CMD_RDID => {
                // Wire order mfg, type, capacity → LE word 0x001640EF.
                self.fifo[0] = JEDEC_ID;
            }
            CMD_RDSR => self.fifo[0] = self.flash_status as u32,
            CMD_RDSR2 => self.fifo[0] = self.sr2 as u32,
            CMD_RDSR3 => self.fifo[0] = self.sr3 as u32,
            CMD_WREN => self.flash_status |= STATUS_WEL,
            CMD_WRDI => self.flash_status &= !STATUS_WEL,
            CMD_WRSR => {
                self.sr2 = ((self.fifo[0] >> 8) & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
            }
            CMD_WRSR2 => {
                self.sr2 = (self.fifo[0] & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
            }
            CMD_WRSR3 => {
                self.sr3 = (self.fifo[0] & 0xFF) as u8;
                self.flash_status &= !(STATUS_WEL | STATUS_WIP);
            }
            CMD_RDUID => {
                // Non-trivial unique id (esp_flash rejects all-0 / all-1).
                self.fifo[0] = 0x1716_1514;
                self.fifo[1] = 0x1F1E_1D1C;
            }
            CMD_SFDP => {
                // No SFDP table → zeros so driver falls back to RDID size.
                let n = read_bytes.max(1).div_ceil(4).min(self.fifo.len());
                for w in self.fifo.iter_mut().take(n) {
                    *w = 0;
                }
            }
            // Data-read family (single/dual/quad). Without these, DIO/QIO host
            // modes leave W0.. at 0 and NVS marks blank pages CORRUPT.
            CMD_READ | CMD_FAST_READ | CMD_DREAD | CMD_QREAD | CMD_DIO_READ | CMD_QIO_READ => {
                self.fill_fifo_from_flash(addr, read_bytes.max(1));
            }
            _ => {
                // Unknown opcode with MISO: leave FIFO unchanged (usually 0).
            }
        }
    }

    fn fill_fifo_from_flash(&mut self, addr: usize, nbytes: usize) {
        let nbytes = nbytes.min(64);
        let mut buf = vec![0xFFu8; nbytes];
        if let Some(flash) = &self.flash {
            if let Ok(f) = flash.lock() {
                for (i, b) in buf.iter_mut().enumerate() {
                    *b = f.get(addr + i).copied().unwrap_or(0xFF);
                }
            }
        }
        for (w, chunk) in buf.chunks(4).enumerate() {
            if w >= self.fifo.len() {
                break;
            }
            let mut word = 0u32;
            for (j, &b) in chunk.iter().enumerate() {
                word |= (b as u32) << (8 * j);
            }
            self.fifo[w] = word;
        }
    }

    /// SPI NOR page program: FIFO bytes AND into flash (only 1→0 without erase).
    fn program_flash(&mut self, addr: usize, nbytes: usize) {
        let Some(flash) = &self.flash else {
            return;
        };
        let Ok(mut f) = flash.lock() else {
            return;
        };
        let nbytes = nbytes.min(256).min(64); // hardware FIFO is 64B
        for i in 0..nbytes {
            let word = self.fifo[i / 4];
            let byte = ((word >> ((i % 4) * 8)) & 0xFF) as u8;
            let at = addr + i;
            if at < f.len() {
                f[at] &= byte;
            }
        }
    }

    /// SPI NOR erase: set region to 0xFF. `len == usize::MAX` → whole array.
    fn erase_flash(&mut self, addr: usize, len: usize) {
        let Some(flash) = &self.flash else {
            return;
        };
        let Ok(mut f) = flash.lock() else {
            return;
        };
        if len == usize::MAX {
            f.fill(0xFF);
            return;
        }
        let start = addr.min(f.len());
        let end = (addr.saturating_add(len)).min(f.len());
        if start < end {
            f[start..end].fill(0xFF);
        }
    }
}

impl Peripheral for Esp32Spi {
    // Inert walk: SPI transactions run atomically at the launching CMD write (USR auto-clears there); tick() is an explicit no-op.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        let word = self.read_word(word_off);
        Ok(((word >> byte_off) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_off = offset & !3;
        let byte_off = (offset & 3) * 8;
        // Read current word so partial byte writes preserve the rest.
        let mut word = self.read_word(word_off);
        word &= !(0xFFu32 << byte_off);
        word |= (value as u32) << byte_off;
        self.write_word(word_off, word);
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        // Word-granular fast path — many ESP32 drivers issue STA on the FIFO
        // and SPI_CMD as full word writes. Avoiding the per-byte RMW means
        // CMD_USR isn't fired four times per word write.
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            // Misaligned — fall back to byte path.
            for i in 0..4 {
                self.write(offset + i, ((value >> (i * 8)) & 0xFF) as u8)?;
            }
            Ok(())
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "layout": "esp32_spi",
            "cmd": self.cmd,
            "user": self.user,
            "mosi_dlen": self.mosi_dlen,
            "attached_devices": self.attached_devices.len(),
        })
    }

    /// Runtime snapshot — controller registers plus per-attached-device
    /// blobs. Device blobs are keyed by CS pin so the restorer can match
    /// them up regardless of attach order.
    fn runtime_snapshot(&self) -> Vec<u8> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            cmd: u32,
            user: u32,
            mosi_dlen: u32,
            captured_bytes: Vec<u8>,
            transactions: u64,
            // (cs_pin, opaque device snapshot bytes)
            devices: Vec<(String, Vec<u8>)>,
        }
        let snap = Snap {
            cmd: self.cmd,
            user: self.user,
            mosi_dlen: self.mosi_dlen,
            captured_bytes: self.captured_bytes.clone(),
            transactions: self.transactions,
            devices: self
                .attached_devices
                .iter()
                .map(|d| (d.cs_pin().to_string(), d.runtime_snapshot()))
                .collect(),
        };
        bincode::serialize(&snap).expect("bincode serialize Esp32Spi")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Snap {
            cmd: u32,
            user: u32,
            mosi_dlen: u32,
            captured_bytes: Vec<u8>,
            transactions: u64,
            devices: Vec<(String, Vec<u8>)>,
        }
        let snap: Snap = bincode::deserialize(bytes).map_err(|e| {
            crate::SimulationError::NotImplemented(format!("Esp32Spi snapshot decode: {e}"))
        })?;
        self.cmd = snap.cmd;
        self.user = snap.user;
        self.mosi_dlen = snap.mosi_dlen;
        self.captured_bytes = snap.captured_bytes;
        self.transactions = snap.transactions;
        for (cs_pin, blob) in snap.devices {
            if let Some(dev) = self
                .attached_devices
                .iter_mut()
                .find(|d| d.cs_pin() == cs_pin)
            {
                dev.restore_runtime_snapshot(&blob)?;
            }
        }
        Ok(())
    }

    fn tick(&mut self) -> PeripheralTickResult {
        PeripheralTickResult::default()
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn for_each_attached_sim_input(
        &mut self,
        f: &mut dyn FnMut(&mut dyn crate::sim_input::SimInput) -> bool,
    ) -> bool {
        for dev in self.attached_devices.iter_mut() {
            if let Some(si) = dev.as_sim_input_mut() {
                if f(si) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simple SpiDevice that records every byte received.
    #[derive(Default)]
    struct Recorder {
        bytes: Vec<u8>,
        cs_low: u32,
        cs_high: u32,
    }
    impl SpiDevice for Recorder {
        fn cs_select(&mut self) {
            self.cs_low += 1;
        }
        fn cs_release(&mut self) {
            self.cs_high += 1;
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            self.bytes.push(mosi);
            0
        }
        fn cs_pin(&self) -> &str {
            "GPIO5"
        }
        fn as_any(&self) -> Option<&dyn std::any::Any> {
            Some(self)
        }
        fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
            Some(self)
        }
    }

    fn write32(spi: &mut Esp32Spi, off: u64, value: u32) {
        spi.write_u32(off, value).unwrap();
    }

    fn read32(spi: &Esp32Spi, off: u64) -> u32 {
        let mut acc = 0u32;
        for i in 0..4u64 {
            acc |= (spi.read(off + i).unwrap() as u32) << (i * 8);
        }
        acc
    }

    #[test]
    fn round_trips_user_and_user2() {
        let mut spi = Esp32Spi::new();
        write32(&mut spi, REG_USER, 0x1234_5678);
        write32(&mut spi, REG_USER2, 0xAB_CDEF);
        assert_eq!(read32(&spi, REG_USER), 0x1234_5678);
        assert_eq!(read32(&spi, REG_USER2), 0xAB_CDEF);
    }

    #[test]
    fn fifo_word_round_trip() {
        let mut spi = Esp32Spi::new();
        for i in 0..16u64 {
            write32(&mut spi, FIFO_START + i * 4, 0xAA_BB_CC_00 | i as u32);
        }
        for i in 0..16u64 {
            assert_eq!(read32(&spi, FIFO_START + i * 4), 0xAA_BB_CC_00 | i as u32);
        }
    }

    #[test]
    fn cmd_usr_streams_mosi_bytes_to_attached_device() {
        let mut spi = Esp32Spi::new();
        spi.push_device(Box::new(Recorder::default()));

        // Stream 5 bytes: 0x12, 0x34, 0x56, 0x78, 0x9A.
        write32(&mut spi, FIFO_START, 0x78_56_34_12); // bytes 0..3
        write32(&mut spi, FIFO_START + 4, 0x0000_009A); // byte 4

        write32(&mut spi, REG_USER, USER_USR_MOSI_BIT);
        write32(&mut spi, REG_MOSI_DLEN, (5 * 8) - 1);
        write32(&mut spi, REG_CMD, CMD_USR_BIT);

        // CMD should clear immediately (synchronous model).
        assert_eq!(read32(&spi, REG_CMD) & CMD_USR_BIT, 0);

        let rec = spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<Recorder>()
            .unwrap();
        assert_eq!(rec.bytes, vec![0x12, 0x34, 0x56, 0x78, 0x9A]);
        // CS is GPIO-controlled by firmware, not auto-pulsed per CMD.USR.
        assert_eq!(rec.cs_low, 0);
        assert_eq!(rec.cs_high, 0);
    }

    #[test]
    fn cmd_usr_without_usr_mosi_bit_does_not_send_bytes() {
        let mut spi = Esp32Spi::new();
        spi.push_device(Box::new(Recorder::default()));

        write32(&mut spi, FIFO_START, 0xDE_AD_BE_EF);
        write32(&mut spi, REG_USER, 0); // USR_MOSI cleared
        write32(&mut spi, REG_MOSI_DLEN, 32 - 1);
        write32(&mut spi, REG_CMD, CMD_USR_BIT);

        let rec = spi.attached_devices[0]
            .as_any()
            .unwrap()
            .downcast_ref::<Recorder>()
            .unwrap();
        assert!(rec.bytes.is_empty());
        assert_eq!(read32(&spi, REG_CMD) & CMD_USR_BIT, 0);
    }

    #[test]
    fn usr_rdid_fills_w0_with_jedec_id() {
        // memspi_host_read_id_hs: CMD_RDID, miso_len=3 → W0 LE mfg|type|cap.
        let mut spi = Esp32Spi::new();
        write32(&mut spi, REG_USER, USER_USR_COMMAND_BIT | USER_USR_MISO_BIT);
        write32(&mut spi, REG_USER2, CMD_RDID as u32); // opcode 0x9F
        write32(&mut spi, REG_MISO_DLEN, 24 - 1);
        write32(&mut spi, REG_CMD, CMD_USR_BIT);
        assert_eq!(read32(&spi, REG_CMD), 0, "USR auto-clears");
        assert_eq!(read32(&spi, FIFO_START), JEDEC_ID);
        // memspi_host_read_id_hs rejects 0 / 0xFFFFFF.
        assert_ne!(read32(&spi, FIFO_START) & 0x00FF_FFFF, 0);
        assert_ne!(read32(&spi, FIFO_START) & 0x00FF_FFFF, 0x00FF_FFFF);
    }

    #[test]
    fn dedicated_flash_rdid_fills_rd_status() {
        let mut spi = Esp32Spi::new();
        write32(&mut spi, REG_CMD, FLASH_RDID);
        assert_eq!(read32(&spi, REG_CMD), 0);
        assert_eq!(read32(&spi, REG_RD_STATUS), JEDEC_ID);
    }

    #[test]
    fn dedicated_wren_rdsr_sets_wel() {
        let mut spi = Esp32Spi::new();
        write32(&mut spi, REG_CMD, FLASH_WREN);
        write32(&mut spi, REG_CMD, FLASH_RDSR);
        let status = read32(&spi, REG_RD_STATUS);
        assert_eq!(status & STATUS_WEL as u32, STATUS_WEL as u32);
        assert_eq!(status & STATUS_WIP as u32, 0);
    }

    #[test]
    fn usr_rdsr_reports_not_busy() {
        let mut spi = Esp32Spi::new();
        write32(&mut spi, REG_USER, USER_USR_COMMAND_BIT | USER_USR_MISO_BIT);
        write32(&mut spi, REG_USER2, CMD_RDSR as u32);
        write32(&mut spi, REG_MISO_DLEN, 8 - 1);
        write32(&mut spi, REG_CMD, CMD_USR_BIT);
        assert_eq!(read32(&spi, FIFO_START) & 1, 0); // WIP clear
        assert_eq!(read32(&spi, REG_CMD), 0);
    }
}
