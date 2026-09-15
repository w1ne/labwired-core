// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! APS6408 octal PSRAM device (the "R8" half of an ESP32-S3-N16R8 module).
//!
//! This is the chip on the far side of MSPI CS1. It is a *device* model, not a
//! controller model: it answers the OPI commands the boot path sends and owns
//! the 8 MiB array. The controller
//! ([`SpiMemFlash`](super::spi_mem_flash::SpiMemFlash)) decides which chip
//! select is active and hands the command over.
//!
//! ## Why the device has to be real
//!
//! ESP-IDF refuses to boot a `CONFIG_SPIRAM=y` image whose PSRAM it cannot
//! identify — `esp_psram_impl_octal.c` reads the mode registers through
//! `esp_rom_opiflash_exec_cmd` and aborts with "PSRAM ID read error" if the
//! vendor id is not `0x0D`. Answering with a plausible constant would be a
//! thunk; answering from mode registers the firmware itself programmed is the
//! behaviour of the part.
//!
//! ## Commands (`esp_psram_impl_octal.c`)
//!
//! | opcode   | meaning              | notes                                  |
//! |---------:|----------------------|----------------------------------------|
//! | `0x0000` | SYNC READ            | array read, 32-bit address             |
//! | `0x8080` | SYNC WRITE           | array write                            |
//! | `0x4040` | MODE REGISTER READ   | address = MR index (0, 2, 4, 8)        |
//! | `0xC0C0` | MODE REGISTER WRITE  | address = MR index                     |
//!
//! Mode-register reads are *consecutive*: IDF asks for 16 bits at MR0 and
//! expects MR0 then MR1, and 16 bits at MR2 for MR2 then MR3. Returning the
//! same register twice reads back as vendor id `0x28` and fails the probe.
//!
//! ## Register contents
//!
//! MR1/MR2/MR3 are chip identity and are read-only; MR0 and MR8 are programmed
//! by the firmware during init and read back. The defaults here reproduce a
//! physically measured N16R8 boot log (2026-08-16, MAC `ac:a7:04:2c:80:3c`):
//!
//! ```text
//! vendor id    : 0x0d (AP)          dev id       : 0x02 (generation 3)
//! density      : 0x03 (64 Mbit)     good-die     : 0x01 (Pass)
//! Latency      : 0x01 (Fixed)       VCC          : 0x01 (3V)
//! SRF          : 0x01 (Fast Refresh)
//! ```

use std::sync::{Arc, Mutex};

/// Octal PSRAM array size for an N16R8 module: 64 Mbit.
pub const PSRAM_SIZE_BYTES: usize = 8 * 1024 * 1024;

// OPI opcodes, from `esp_psram_impl_octal.c`.
pub const OPI_SYNC_READ: u16 = 0x0000;
pub const OPI_SYNC_WRITE: u16 = 0x8080;
pub const OPI_REG_READ: u16 = 0x4040;
pub const OPI_REG_WRITE: u16 = 0xC0C0;

/// Number of addressable mode registers (MR0..MR8).
const MODE_REG_COUNT: usize = 9;

/// MR0: drive_str[1:0]=0 (1/1), read_latency[4:2]=2 (10 cycles), lt[5]=1 (fixed).
const MR0_DEFAULT: u8 = 0b0010_1000;
/// MR1: vendor_id[4:0] = 0x0D ("AP" — AP Memory). Read-only identity.
const MR1_DEFAULT: u8 = 0x0D;
/// MR2: density[2:0]=3 (64 Mbit), dev_id[4:3]=2 (generation 3), gb[7]=1 (pass).
const MR2_DEFAULT: u8 = 0b1001_0011;
/// MR3: srf[5]=1 (fast refresh), vcc[6]=1 (3V). Read-only identity.
const MR3_DEFAULT: u8 = 0b0110_0000;
/// MR4: rf[3]=1, wr_latency[7:5]=1.
const MR4_DEFAULT: u8 = 0b0010_1000;
/// MR8: bl[1:0]=1 (32-byte burst), bt[2]=1 (hybrid wrap).
const MR8_DEFAULT: u8 = 0b0000_0101;

/// Mode registers that describe the silicon rather than its configuration.
/// Firmware may write them; the part ignores it, and so do we — otherwise a
/// stray write during timing tuning could erase the vendor id mid-boot.
const READ_ONLY_REGS: [usize; 3] = [1, 2, 3];

/// An APS6408 8 MiB octal PSRAM on MSPI CS1.
#[derive(Debug)]
pub struct PsramDevice {
    /// The 8 MiB array. Shared with the flash-cache windows: an MMU entry
    /// tagged `SOC_MMU_ACCESS_SPIRAM` reads and writes these bytes directly,
    /// exactly as the cache does on silicon.
    array: Arc<Mutex<Vec<u8>>>,
    mode_regs: [u8; MODE_REG_COUNT],
}

impl Default for PsramDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl PsramDevice {
    pub fn new() -> Self {
        let mut mode_regs = [0u8; MODE_REG_COUNT];
        mode_regs[0] = MR0_DEFAULT;
        mode_regs[1] = MR1_DEFAULT;
        mode_regs[2] = MR2_DEFAULT;
        mode_regs[3] = MR3_DEFAULT;
        mode_regs[4] = MR4_DEFAULT;
        mode_regs[8] = MR8_DEFAULT;

        Self {
            // PSRAM powers up with undefined contents; zero is the honest
            // choice for a deterministic twin (and IDF's memory test writes
            // before it reads).
            array: Arc::new(Mutex::new(vec![0u8; PSRAM_SIZE_BYTES])),
            mode_regs,
        }
    }

    /// The array, for the cache windows to map through the MMU.
    pub fn array(&self) -> Arc<Mutex<Vec<u8>>> {
        self.array.clone()
    }

    /// Execute one OPI transaction. `read_len` is the number of bytes the
    /// controller will hand back to firmware; `write_data` is the data phase
    /// of a write command. Returns the read data (padded with zeros if the
    /// command produces none).
    pub fn exec(&mut self, cmd: u16, addr: u32, write_data: &[u8], read_len: usize) -> Vec<u8> {
        let mut out = vec![0u8; read_len];

        match cmd {
            OPI_REG_READ => {
                // Consecutive registers from `addr`, wrapping is not a thing:
                // an out-of-range index reads 0, like an unimplemented MR.
                for (i, b) in out.iter_mut().enumerate() {
                    *b = self.mode_regs.get(addr as usize + i).copied().unwrap_or(0);
                }
            }
            OPI_REG_WRITE => {
                for (i, v) in write_data.iter().enumerate() {
                    let idx = addr as usize + i;
                    if idx < MODE_REG_COUNT && !READ_ONLY_REGS.contains(&idx) {
                        self.mode_regs[idx] = *v;
                    }
                }
            }
            OPI_SYNC_READ => {
                let array = self.array.lock().unwrap();
                for (i, b) in out.iter_mut().enumerate() {
                    *b = array.get(addr as usize + i).copied().unwrap_or(0);
                }
            }
            OPI_SYNC_WRITE => {
                let mut array = self.array.lock().unwrap();
                for (i, v) in write_data.iter().enumerate() {
                    let idx = addr as usize + i;
                    if idx < array.len() {
                        array[idx] = *v;
                    }
                }
            }
            _ => {
                // Unknown opcode: the part would drive nothing. Leave zeros —
                // and say so, because a silent zero here looks exactly like the
                // ID-read failure this model exists to fix.
                if std::env::var("LABWIRED_SPI_DEBUG").is_ok() {
                    eprintln!("psram: unhandled OPI command 0x{cmd:04x} addr=0x{addr:08x}");
                }
            }
        }

        out
    }

    /// Mode register value, for tests.
    #[allow(dead_code)]
    pub fn mode_reg(&self, index: usize) -> u8 {
        self.mode_regs.get(index).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe IDF actually performs: 16 bits at MR0 must yield MR0 then MR1,
    /// and the vendor id in MR1 must be 0x0D or `esp_psram_init` aborts the boot.
    #[test]
    fn reg_read_returns_consecutive_registers() {
        let mut psram = PsramDevice::new();

        let mr0_1 = psram.exec(OPI_REG_READ, 0x0, &[], 2);
        assert_eq!(mr0_1[0], MR0_DEFAULT);
        assert_eq!(mr0_1[1] & 0x1F, 0x0D, "vendor id must read as AP (0x0D)");

        let mr2_3 = psram.exec(OPI_REG_READ, 0x2, &[], 2);
        assert_eq!(mr2_3[0] & 0x07, 0x03, "density must read as 64 Mbit");
        assert_eq!((mr2_3[0] >> 3) & 0x03, 0x02, "dev id must read as gen 3");
        assert_eq!(mr2_3[1], MR3_DEFAULT);
    }

    /// Firmware programs MR0/MR8 during init and reads them back to confirm.
    #[test]
    fn configuration_registers_round_trip() {
        let mut psram = PsramDevice::new();

        psram.exec(OPI_REG_WRITE, 0x0, &[0x2C], 0);
        assert_eq!(psram.exec(OPI_REG_READ, 0x0, &[], 1)[0], 0x2C);

        psram.exec(OPI_REG_WRITE, 0x8, &[0x07], 0);
        assert_eq!(psram.exec(OPI_REG_READ, 0x8, &[], 1)[0], 0x07);
    }

    /// A write to the identity registers must not stick: losing the vendor id
    /// mid-boot would fail the probe in a way that looks like a missing chip.
    #[test]
    fn identity_registers_ignore_writes() {
        let mut psram = PsramDevice::new();

        psram.exec(OPI_REG_WRITE, 0x1, &[0x00], 0);
        psram.exec(OPI_REG_WRITE, 0x2, &[0x00], 0);

        assert_eq!(psram.mode_reg(1), MR1_DEFAULT);
        assert_eq!(psram.mode_reg(2), MR2_DEFAULT);
    }

    #[test]
    fn array_round_trips_and_is_eight_mib() {
        let mut psram = PsramDevice::new();

        psram.exec(OPI_SYNC_WRITE, 0x1234, &[0xDE, 0xAD, 0xBE, 0xEF], 0);
        assert_eq!(
            psram.exec(OPI_SYNC_READ, 0x1234, &[], 4),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );

        assert_eq!(psram.array().lock().unwrap().len(), 8 * 1024 * 1024);
    }
}
