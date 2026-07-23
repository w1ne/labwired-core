// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MFRC522 / RC522 RFID reader — SPI register shell (no RF air interface).
//!
//! Exposes VersionReg (0x37) = 0x92 (MFRC522 v2.0) and a minimal CommandReg
//! so common init probes succeed. Card UID / ISO14443 is not modelled.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const REG_COMMAND: u8 = 0x01;
const REG_VERSION: u8 = 0x37;
const CMD_IDLE: u8 = 0x00;
const CMD_SOFTRESET: u8 = 0x0F;

/// Address byte: bit 7 = R/W (1=read), bits 6:1 = reg, bit 0 = 0.
fn reg_from_addr(addr: u8) -> u8 {
    (addr >> 1) & 0x3F
}

fn is_read(addr: u8) -> bool {
    (addr & 0x80) != 0
}

pub struct Rc522 {
    cs_pin: String,
    regs: [u8; 0x40],
    phase: Phase,
    reading: bool,
    reg: u8,
    component_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Address,
    Data,
}

impl Rc522 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 0x40];
        regs[REG_VERSION as usize] = 0x92;
        regs[REG_COMMAND as usize] = CMD_IDLE;
        Self {
            cs_pin: cs_pin.into(),
            regs,
            phase: Phase::Address,
            reading: false,
            reg: 0,
            component_id: None,
        }
    }
}

impl SpiDevice for Rc522 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.phase = Phase::Address;
    }

    fn cs_release(&mut self) {
        self.phase = Phase::Address;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        match self.phase {
            Phase::Address => {
                self.reg = reg_from_addr(mosi);
                self.reading = is_read(mosi);
                self.phase = Phase::Data;
                // First MISO during address phase is undefined / status-ish; return 0.
                0x00
            }
            Phase::Data => {
                let idx = (self.reg as usize).min(self.regs.len() - 1);
                if self.reading {
                    let v = self.regs[idx];
                    // Auto-increment not required for version probe
                    v
                } else {
                    if self.reg == REG_COMMAND && mosi == CMD_SOFTRESET {
                        self.regs[REG_COMMAND as usize] = CMD_IDLE;
                        self.regs[REG_VERSION as usize] = 0x92;
                    } else {
                        self.regs[idx] = mosi;
                    }
                    0x00
                }
            }
        }
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Rc522Kit;
pub static RC522_KIT: Rc522Kit = Rc522Kit;

static RC522_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "rc522",
    label: "RC522 RFID",
    summary: "MFRC522 SPI RFID reader register shell (no RF).",
    detail: "VersionReg 0x37 returns 0x92. Soft reset via CommandReg is accepted. \
             ISO14443 card UID / crypto is not simulated.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "SPI CS GPIO pin name (module SDA/NSS). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for Rc522Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &RC522_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = Rc522::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_register_is_0x92() {
        let mut d = Rc522::new("PA4");
        d.cs_select();
        d.transfer(0x80 | (REG_VERSION << 1)); // read VersionReg
        let v = d.transfer(0x00);
        assert_eq!(v, 0x92);
        d.cs_release();
    }
}
