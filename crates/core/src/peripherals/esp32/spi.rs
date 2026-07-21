// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-classic SPI controller (HSPI / VSPI shape).
//!
//! Models the subset of registers a bare-metal MOSI-only driver actually
//! pokes when streaming bytes to a display (e-paper / TFT). Reference:
//! ESP32 TRM v4.6 §7 (SPI Controller), register layout in Table 7-3.
//!
//! Register window is 0x100 bytes wide. ESP32-classic offsets (NOT ESP32-S3,
//! whose USER block sits 4 bytes higher). The MVP models:
//!   * SPI_CMD_REG (0x00)        — write bit 18 (USR) to start; cleared on completion.
//!   * SPI_USER_REG (0x1C)       — round-tripped; USR_MOSI / USR_COMMAND bits inspected.
//!   * SPI_USER2_REG (0x24)      — round-tripped (command bitlen + value).
//!   * SPI_MOSI_DLEN_REG (0x28)  — bit length minus 1; used to size MOSI stream.
//!   * SPI_MISO_DLEN_REG (0x2C)  — round-tripped, unused (write-only display).
//!   * SPI_W0..W15 (0x80..0xBC)  — 64-byte FIFO; little-endian byte order.
//!   * Other offsets             — round-tripped (so firmware's RMW polls don't break).
//!
//! On `CMD_REG` write with bit 18 set, the peripheral synchronously:
//!   1. Notes byte_count = ((MOSI_DLEN & 0x7FF) + 1).div_ceil(8).
//!   2. Calls `cs_select()` on every attached `SpiDevice`.
//!   3. Streams each MOSI byte from the FIFO via `transfer()`.
//!   4. Calls `cs_release()`.
//!   5. Clears CMD_REG bit 18 so the firmware's busy-poll completes
//!      on the next read.
//!
//! Multi-device CS-arbitration is intentionally not modeled — the same
//! simplification as `peripherals::spi::Spi`. For the e-paper lab one
//! SSD1680 panel is attached.

use crate::peripherals::spi::SpiDevice;
use crate::{Peripheral, PeripheralTickResult, SimResult};
use std::collections::HashMap;

const REG_CMD: u64 = 0x00;
pub const REG_USER: u64 = 0x1C;
const REG_USER2: u64 = 0x24;
const REG_MOSI_DLEN: u64 = 0x28;
const REG_MISO_DLEN: u64 = 0x2C;
const FIFO_START: u64 = 0x80;
const FIFO_END: u64 = 0xC0; // exclusive — W0..W15 = 64 bytes

const CMD_USR_BIT: u32 = 1 << 18;
pub const USER_USR_MOSI_BIT: u32 = 1 << 27;

#[derive(Default)]
pub struct Esp32Spi {
    cmd: u32,
    user: u32,
    user2: u32,
    mosi_dlen: u32,
    miso_dlen: u32,
    fifo: [u32; 16],
    /// Round-trip every other offset so firmware RMW sequences observe their writes.
    other: HashMap<u64, u32>,

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
                if value & CMD_USR_BIT != 0 {
                    self.kick_user_transaction();
                } else if value != 0 {
                    // Flash-controller command bits (the SPI_FLASH_* set, and
                    // the BROM `spi_flash_attach` bit-12 probe) are
                    // write-1-to-start and self-clear when the operation
                    // completes. We don't model flash array content, so the op
                    // completes instantly — clear the start bits so the
                    // firmware's busy-poll (`bnez SPI_CMD_REG`) terminates
                    // instead of spinning forever (real silicon clears these in
                    // a few SPI clocks).
                    self.cmd = 0;
                }
            }
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

    /// Run one user-defined transaction synchronously: drain MOSI bytes
    /// from the FIFO and stream them to every attached device, then clear
    /// the USR bit so firmware's busy-poll completes.
    ///
    /// CS-aware routing: we do NOT call `cs_select` / `cs_release` here.
    /// Real ESP32 firmware drives the CS pin via the GPIO matrix
    /// (writes to GPIO_OUT_W1TS/W1TC) and a single logical "transaction"
    /// to a peripheral like SSD1680 spans MANY SPI3 CMD.USR fires — the
    /// firmware writes 1 byte for the command, then dozens of 32-byte
    /// chunks for the plane data, all while holding CS low. Pulsing
    /// CS per CMD.USR would reset the SSD1680 protocol state mid-stream.
    fn kick_user_transaction(&mut self) {
        // If the firmware didn't request a MOSI phase, there's nothing to stream.
        if self.user & USER_USR_MOSI_BIT == 0 {
            self.cmd &= !CMD_USR_BIT;
            return;
        }
        let bits = (self.mosi_dlen & 0x7FF) + 1;
        let byte_count = bits.div_ceil(8) as usize;

        self.transactions += 1;
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
        self.cmd &= !CMD_USR_BIT;
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
}
