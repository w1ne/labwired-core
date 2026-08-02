// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SX1278 / RA-02 LoRa module — SPI register shell (no RF air link).
//!
//! Version register 0x42 returns 0x12 (SX127x). Enough for RadioLib / LMIC
//! style probes.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const REG_VERSION: u8 = 0x42;
const REG_OPMODE: u8 = 0x01;

pub struct LoraSx1278 {
    cs_pin: String,
    regs: [u8; 0x80],
    phase: Phase,
    writing: bool,
    reg: u8,
    component_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Address,
    Data,
}

impl LoraSx1278 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 0x80];
        regs[REG_VERSION as usize] = 0x12;
        regs[REG_OPMODE as usize] = 0x09; // LoRa sleep-ish
        Self {
            cs_pin: cs_pin.into(),
            regs,
            phase: Phase::Address,
            writing: false,
            reg: 0,
            component_id: None,
        }
    }
}

impl SpiDevice for LoraSx1278 {
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
                // bit7 = write on SX127x
                self.writing = (mosi & 0x80) != 0;
                self.reg = mosi & 0x7F;
                self.phase = Phase::Data;
                0x00
            }
            Phase::Data => {
                let idx = (self.reg as usize).min(self.regs.len() - 1);
                if self.writing {
                    self.regs[idx] = mosi;
                    0x00
                } else {
                    self.regs[idx]
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

pub struct LoraSx1278Kit;
pub static LORA_SX1278_KIT: LoraSx1278Kit = LoraSx1278Kit;

static LORA_SX1278_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "lora-sx1278",
    label: "LoRa SX1278 / RA-02",
    summary: "Semtech SX1278 SPI LoRa register shell (no RF).",
    detail: "Version reg 0x42 = 0x12. OpMode and companion regs R/W. \
             Packet air interface is not simulated.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "SPI NSS GPIO pin. Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for LoraSx1278Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &LORA_SX1278_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = LoraSx1278::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_0x12() {
        let mut d = LoraSx1278::new("PA4");
        d.cs_select();
        d.transfer(REG_VERSION); // read
        assert_eq!(d.transfer(0x00), 0x12);
    }
}
