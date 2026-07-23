// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! AT24C256 32 KiB I²C EEPROM shell (AliExpress common breakout).
//!
//! Random write: two address bytes then data. Sequential read after address
//! pointer set. Storage is a 256-byte window for sim size (wraps).

use crate::peripherals::i2c::I2cDevice;

const ADDR_DEFAULT: u8 = 0x50;
const MEM_SIZE: usize = 256;

pub struct At24c256 {
    address: u8,
    mem: [u8; MEM_SIZE],
    ptr: u16,
    addr_bytes: u8,
    component_id: Option<String>,
}

impl At24c256 {
    pub fn new(address: u8) -> Self {
        Self {
            address,
            mem: [0xFF; MEM_SIZE],
            ptr: 0,
            addr_bytes: 0,
            component_id: None,
        }
    }
}

impl I2cDevice for At24c256 {
    fn address(&self) -> u8 {
        self.address
    }

    fn start(&mut self) {
        self.addr_bytes = 0;
    }

    fn write(&mut self, data: u8) {
        match self.addr_bytes {
            0 => {
                self.ptr = (data as u16) << 8;
                self.addr_bytes = 1;
            }
            1 => {
                self.ptr = (self.ptr & 0xFF00) | data as u16;
                self.addr_bytes = 2;
            }
            _ => {
                let idx = (self.ptr as usize) % MEM_SIZE;
                self.mem[idx] = data;
                self.ptr = self.ptr.wrapping_add(1);
            }
        }
    }

    fn read(&mut self) -> u8 {
        let idx = (self.ptr as usize) % MEM_SIZE;
        let v = self.mem[idx];
        self.ptr = self.ptr.wrapping_add(1);
        v
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct At24c256Kit;
pub static AT24C256_KIT: At24c256Kit = At24c256Kit;

static AT24C256_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "at24c256",
    label: "AT24C256 EEPROM",
    summary: "I²C 32 KiB EEPROM shell (256-byte sim window).",
    detail: "Default address 0x50. Two-byte address pointer then sequential R/W. \
             Simulated memory is 256 bytes wrapping for compactness.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address. Defaults to 0x50.",
    }],
    labs: &[],
};

impl PeripheralKit for At24c256Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &AT24C256_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(ADDR_DEFAULT)?;
        let mut dev = At24c256::new(address);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_i2c_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_byte() {
        let mut d = At24c256::new(0x50);
        d.start();
        d.write(0x00);
        d.write(0x10);
        d.write(0xAB);
        d.start();
        d.write(0x00);
        d.write(0x10);
        d.start();
        assert_eq!(d.read(), 0xAB);
    }
}
