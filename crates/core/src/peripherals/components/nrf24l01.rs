// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! nRF24L01+ SPI register shell — no air link.
//!
//! Supports R_REGISTER / W_REGISTER for the common CONFIG/EN_AA/…/STATUS map
//! used by RF24-style drivers. STATUS is returned on the first MISO byte of
//! every SPI frame (silicon behaviour).

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const CMD_R_REGISTER: u8 = 0x00;
const CMD_W_REGISTER: u8 = 0x20;
const CMD_NOP: u8 = 0xFF;

const REG_CONFIG: u8 = 0x00;
const REG_STATUS: u8 = 0x07;

pub struct Nrf24l01 {
    cs_pin: String,
    regs: [u8; 0x18],
    /// SPI transaction phase after CS select.
    phase: Phase,
    cmd: u8,
    reg: u8,
    component_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Command,
    Data,
}

impl Nrf24l01 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 0x18];
        regs[REG_CONFIG as usize] = 0x08; // EN_CRC
        regs[REG_STATUS as usize] = 0x0E;
        regs[0x02] = 0x03; // EN_RXADDR
        regs[0x03] = 0x03; // SETUP_AW
        regs[0x05] = 0x02; // RF_CH
        regs[0x06] = 0x0F; // RF_SETUP
        Self {
            cs_pin: cs_pin.into(),
            regs,
            phase: Phase::Command,
            cmd: 0,
            reg: 0,
            component_id: None,
        }
    }

    fn status(&self) -> u8 {
        self.regs[REG_STATUS as usize]
    }
}

impl SpiDevice for Nrf24l01 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.phase = Phase::Command;
    }

    fn cs_release(&mut self) {
        self.phase = Phase::Command;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        match self.phase {
            Phase::Command => {
                self.cmd = mosi;
                self.phase = Phase::Data;
                if mosi == CMD_NOP {
                    return self.status();
                }
                let op = mosi & 0xE0;
                self.reg = mosi & 0x1F;
                if op == CMD_R_REGISTER || op == CMD_W_REGISTER {
                    return self.status();
                }
                self.status()
            }
            Phase::Data => {
                let op = self.cmd & 0xE0;
                let idx = self.reg as usize;
                if op == CMD_R_REGISTER {
                    let v = if idx < self.regs.len() {
                        self.regs[idx]
                    } else {
                        0
                    };
                    self.reg = self.reg.wrapping_add(1);
                    v
                } else if op == CMD_W_REGISTER {
                    if idx < self.regs.len() && idx != REG_STATUS as usize {
                        self.regs[idx] = mosi;
                    }
                    // STATUS bits clear-on-write for RX_DR/TX_DS/MAX_RT when writing STATUS
                    if idx == REG_STATUS as usize {
                        self.regs[REG_STATUS as usize] &= !mosi;
                    }
                    self.reg = self.reg.wrapping_add(1);
                    self.status()
                } else {
                    self.status()
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

pub struct Nrf24l01Kit;
pub static NRF24L01_KIT: Nrf24l01Kit = Nrf24l01Kit;

static NRF24L01_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "nrf24l01",
    label: "nRF24L01+",
    summary: "2.4 GHz SPI transceiver register shell (no air link).",
    detail: "Nordic nRF24L01+ SPI R_REGISTER/W_REGISTER model for CONFIG/STATUS and \
             companion registers. RF packet radio is not simulated.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "CSN GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for Nrf24l01Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &NRF24L01_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = Nrf24l01::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_config_register() {
        let mut dev = Nrf24l01::new("PA4");
        dev.cs_select();
        let st = dev.transfer(CMD_R_REGISTER | REG_CONFIG);
        assert_eq!(st & 0x0E, 0x0E);
        let cfg = dev.transfer(0x00);
        assert_eq!(cfg, 0x08);
        dev.cs_release();
    }

    #[test]
    fn write_rf_channel() {
        let mut dev = Nrf24l01::new("PA4");
        dev.cs_select();
        dev.transfer(CMD_W_REGISTER | 0x05);
        dev.transfer(0x4C);
        dev.cs_release();
        dev.cs_select();
        dev.transfer(CMD_R_REGISTER | 0x05);
        assert_eq!(dev.transfer(0), 0x4C);
    }
}
