// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AHT20 I²C temperature + humidity sensor as an [`I2cDevice`].
//!
//! Aosong AHT20 datasheet (rev 1.1) summary:
//! - 7-bit address `0x38` (fixed).
//! - Command-stream protocol (no register pointer):
//!   - `0xBA`               — soft reset
//!   - `0xBE 0x08 0x00`     — init (one-shot after power-up)
//!   - `0xAC 0x33 0x00`     — trigger measurement
//! - After trigger, firmware **polls** the status byte (BUSY = bit 7) until
//!   it clears, then reads 7 bytes:
//!   `[status, hum19:12, hum11:4, hum3:0|temp19:16, temp15:8, temp7:0, crc8]`
//! - CRC8 polynomial 0x31, init 0xFF, no final XOR.
//!
//! For deterministic simulation we don't actually model elapsed time. Instead
//! we treat BUSY as a small counter: the first `BUSY_TICKS` reads of the
//! status byte after a trigger return `BUSY | CAL`, then it clears. This
//! exercises the polling-loop firmware path without depending on `tick()`.
//!
//! Fixed measurement: **25.0 °C, 50 %RH** — encoded once at construction.

use crate::peripherals::i2c::I2cDevice;

const AHT20_ADDR: u8 = 0x38;
const BUSY_TICKS: u8 = 2;
const STATUS_BUSY: u8 = 0x80;
const STATUS_CAL: u8 = 0x08;
const STATUS_READY: u8 = STATUS_CAL; // calibrated, idle

/// AHT20 mock model.
#[derive(Debug)]
pub struct Aht20 {
    /// 7-byte read payload (status + 5 data + crc). Index 0 is overwritten
    /// dynamically with BUSY semantics, indices 1..=6 are fixed.
    payload: [u8; 7],
    /// Counts how many status reads since last trigger should still return
    /// BUSY. Saturates at 0.
    busy_remaining: u8,
    /// Position in `payload` for the next `read()` call. Reset on `start()`.
    read_idx: usize,
    /// Position in the current command stream from the master (write phase).
    /// Reset on `start()`. Used to interpret multi-byte command sequences.
    write_idx: u8,
    /// Last command opcode (the first write after start). Used to dispatch
    /// subsequent parameter bytes.
    last_cmd: u8,
}

impl Aht20 {
    pub fn new() -> Self {
        // Fixed 25.0 °C, 50 %RH encoded per datasheet:
        //   raw_h (20-bit) = 50 * 2^20 / 100 = 0x80000
        //   raw_t (20-bit) = (25 + 50) * 2^20 / 200 = 0x60000
        // Byte layout:
        //   [1] = hum[19:12] = 0x80
        //   [2] = hum[11:4]  = 0x00
        //   [3] = (hum[3:0] << 4) | temp[19:16] = 0x06
        //   [4] = temp[15:8] = 0x00
        //   [5] = temp[7:0]  = 0x00
        let mut payload = [
            STATUS_READY,
            0x80,
            0x00,
            0x06,
            0x00,
            0x00,
            0x00, // crc filled in below
        ];
        payload[6] = crc8(&payload[..6]);
        Self {
            payload,
            busy_remaining: 0,
            read_idx: 0,
            write_idx: 0,
            last_cmd: 0,
        }
    }
}

impl Default for Aht20 {
    fn default() -> Self {
        Self::new()
    }
}

impl I2cDevice for Aht20 {
    fn address(&self) -> u8 {
        AHT20_ADDR
    }

    fn start(&mut self) {
        self.read_idx = 0;
        self.write_idx = 0;
    }

    fn write(&mut self, data: u8) {
        match self.write_idx {
            0 => {
                self.last_cmd = data;
                match data {
                    0xAC => {
                        // Trigger measurement: arm BUSY counter so the next
                        // few status reads return BUSY before clearing.
                        self.busy_remaining = BUSY_TICKS;
                    }
                    0xBA => {
                        // Soft reset — clear everything except calibration.
                        self.busy_remaining = 0;
                    }
                    _ => {}
                }
            }
            _ => {
                // Command parameter bytes (0x33 0x00 for 0xAC, 0x08 0x00 for 0xBE).
                // Mock ignores them — real silicon validates parameter framing.
            }
        }
        self.write_idx = self.write_idx.saturating_add(1);
    }

    fn read(&mut self) -> u8 {
        // Byte 0 of every read is the status byte with BUSY semantics.
        // Bytes 1..6 are the fixed payload.
        if self.read_idx == 0 {
            let byte = if self.busy_remaining > 0 {
                self.busy_remaining = self.busy_remaining.saturating_sub(1);
                STATUS_BUSY | STATUS_CAL
            } else {
                self.payload[0]
            };
            self.read_idx += 1;
            byte
        } else if self.read_idx < self.payload.len() {
            let byte = self.payload[self.read_idx];
            self.read_idx += 1;
            byte
        } else {
            // Past end of payload — return 0xFF (no data).
            0xFF
        }
    }
}

/// CRC8 with polynomial 0x31, init 0xFF, no final XOR. Per AHT20 datasheet.
fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &byte in data {
        crc ^= byte;
        for _ in 0..8 {
            if (crc & 0x80) != 0 {
                crc = (crc << 1) ^ 0x31;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Aht20Kit;
pub static AHT20_KIT: Aht20Kit = Aht20Kit;

static AHT20_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "aht20",
    label: "AHT20 Temp/Humidity",
    summary: "Aosong AHT20 I²C temperature and humidity sensor.",
    detail: "Command-stream protocol at 0x38 with fixed 25 °C / 50 %RH measurement \
             and BUSY-poll semantics for firmware that triggers then reads.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x38 (fixed on real silicon).",
    }],
    labs: &[],
};

impl PeripheralKit for Aht20Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &AHT20_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let _address = ctx.i2c_address_or(0x38)?;
        ctx.attach_i2c_device(Box::new(Aht20::new()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_0x38() {
        assert_eq!(Aht20::new().address(), 0x38);
    }

    #[test]
    fn first_status_read_returns_busy_after_trigger() {
        let mut d = Aht20::new();
        // Trigger measurement: 0xAC 0x33 0x00
        d.start();
        d.write(0xAC);
        d.write(0x33);
        d.write(0x00);
        // First read after restart should return BUSY|CAL.
        d.start();
        let s = d.read();
        assert_eq!(
            s & STATUS_BUSY,
            STATUS_BUSY,
            "BUSY must be set right after trigger"
        );
    }

    #[test]
    fn busy_clears_after_n_reads() {
        let mut d = Aht20::new();
        d.start();
        d.write(0xAC);
        d.write(0x33);
        d.write(0x00);
        // Poll the status byte until BUSY clears. Each poll = start+read.
        for _ in 0..BUSY_TICKS {
            d.start();
            let s = d.read();
            assert_eq!(s & STATUS_BUSY, STATUS_BUSY);
        }
        d.start();
        let s = d.read();
        assert_eq!(s & STATUS_BUSY, 0, "BUSY must clear after BUSY_TICKS polls");
        assert_eq!(s & STATUS_CAL, STATUS_CAL, "CAL bit stays set");
    }

    #[test]
    fn payload_carries_25c_50rh_with_valid_crc() {
        let d = Aht20::new();
        // Status, hum, hum, hum/temp split, temp, temp, crc
        assert_eq!(d.payload[1], 0x80);
        assert_eq!(d.payload[2], 0x00);
        assert_eq!(d.payload[3], 0x06);
        assert_eq!(d.payload[4], 0x00);
        assert_eq!(d.payload[5], 0x00);
        // CRC validates against the same algorithm firmware will use.
        assert_eq!(d.payload[6], crc8(&d.payload[..6]));
    }

    #[test]
    fn full_measurement_read_sequence() {
        let mut d = Aht20::new();
        // Trigger
        d.start();
        d.write(0xAC);
        d.write(0x33);
        d.write(0x00);
        // Drain BUSY
        for _ in 0..BUSY_TICKS {
            d.start();
            let _ = d.read();
        }
        // Now read all 7 bytes in one transaction
        d.start();
        let bytes: Vec<u8> = (0..7).map(|_| d.read()).collect();
        assert_eq!(bytes[0] & STATUS_BUSY, 0);
        assert_eq!(bytes[1], 0x80);
        assert_eq!(
            bytes[6],
            crc8(&[bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]])
        );
    }
}
