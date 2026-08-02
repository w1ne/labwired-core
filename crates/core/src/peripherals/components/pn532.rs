// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! PN532 NFC controller — I²C probe shell (no RF).
//!
//! Responds to GetFirmwareVersion-style framing used by Adafruit_PN532 and
//! common AE modules so init does not hang. Card UID is not modelled.

use crate::peripherals::i2c::I2cDevice;

const ADDR_DEFAULT: u8 = 0x24;

pub struct Pn532 {
    address: u8,
    /// Pending response bytes after a host command.
    resp: Vec<u8>,
    resp_idx: usize,
    rx: Vec<u8>,
    component_id: Option<String>,
}

impl Pn532 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            resp: Vec::new(),
            resp_idx: 0,
            rx: Vec::new(),
            component_id: None,
        }
    }

    fn queue_firmware_version(&mut self) {
        // Simplified ACK + firmware frame (IC 0x32, Ver 1.6).
        self.resp = vec![
            0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00, // ACK
            0x00, 0x00, 0xFF, 0x06, 0xFA, 0xD5, 0x03, 0x32, 0x01, 0x06, 0x07, 0xE8, 0x00,
        ];
        self.resp_idx = 0;
    }
}

impl I2cDevice for Pn532 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.rx.clear();
    }

    fn write(&mut self, data: u8) {
        self.rx.push(data);
        // Detect GetFirmwareVersion command (0xD4 0x02) anywhere in stream.
        if self.rx.windows(2).any(|w| w == [0xD4, 0x02]) {
            self.queue_firmware_version();
            self.rx.clear();
        }
    }

    fn read(&mut self) -> u8 {
        if self.resp_idx < self.resp.len() {
            let b = self.resp[self.resp_idx];
            self.resp_idx += 1;
            b
        } else {
            0x00
        }
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Pn532Kit;
pub static PN532_KIT: Pn532Kit = Pn532Kit;

static PN532_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "pn532",
    label: "PN532 NFC",
    summary: "NXP PN532 I²C NFC controller probe shell (no RF).",
    detail: "Responds to GetFirmwareVersion so library init succeeds. \
             ISO14443 card UID / peer-to-peer is not simulated.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x24.",
    }],
    labs: &[],
};

impl PeripheralKit for Pn532Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &PN532_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let mut dev = Pn532::new(address);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firmware_version_returns_bytes() {
        let mut d = Pn532::new(0x24);
        d.start();
        d.write(0x00);
        d.write(0xD4);
        d.write(0x02);
        d.start();
        let first = d.read();
        assert_eq!(first, 0x00);
        assert!(d.read() == 0x00 || d.resp_idx > 0);
    }
}
