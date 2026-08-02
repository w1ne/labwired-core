// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! MCP2515 CAN controller SPI register shell — no CAN bus frames required.
//!
//! Supports RESET, READ, WRITE, READ_STATUS, RX_STATUS, BIT_MODIFY for the
//! register file used by common Arduino MCP_CAN libraries.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;

const INST_WRITE: u8 = 0x02;
const INST_READ: u8 = 0x03;
const INST_BITMOD: u8 = 0x05;
const INST_READ_STATUS: u8 = 0xA0;
const INST_RX_STATUS: u8 = 0xB0;
const INST_RESET: u8 = 0xC0;

const REG_CANSTAT: u8 = 0x0E;
const REG_CANCTRL: u8 = 0x0F;

pub struct Mcp2515 {
    cs_pin: String,
    regs: [u8; 128],
    phase: Phase,
    inst: u8,
    addr: u8,
    bitmod_mask: u8,
    component_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Instruction,
    Address,
    Data,
    BitModMask,
    BitModData,
}

impl Mcp2515 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        let mut regs = [0u8; 128];
        // Power-on: configuration mode (REQOP = 100)
        regs[REG_CANCTRL as usize] = 0x87;
        regs[REG_CANSTAT as usize] = 0x80;
        Self {
            cs_pin: cs_pin.into(),
            regs,
            phase: Phase::Instruction,
            inst: 0,
            addr: 0,
            bitmod_mask: 0,
            component_id: None,
        }
    }

    fn sync_canstat_from_ctrl(&mut self) {
        let op = self.regs[REG_CANCTRL as usize] & 0xE0;
        self.regs[REG_CANSTAT as usize] = (self.regs[REG_CANSTAT as usize] & 0x1F) | op;
    }
}

impl SpiDevice for Mcp2515 {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.phase = Phase::Instruction;
    }

    fn cs_release(&mut self) {
        self.phase = Phase::Instruction;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        match self.phase {
            Phase::Instruction => {
                self.inst = mosi;
                match mosi {
                    INST_RESET => {
                        self.regs = [0u8; 128];
                        self.regs[REG_CANCTRL as usize] = 0x87;
                        self.regs[REG_CANSTAT as usize] = 0x80;
                        self.phase = Phase::Instruction;
                        0
                    }
                    INST_READ | INST_WRITE => {
                        self.phase = Phase::Address;
                        0
                    }
                    INST_BITMOD => {
                        self.phase = Phase::Address;
                        0
                    }
                    INST_READ_STATUS => {
                        self.phase = Phase::Instruction;
                        // TX0IF | empty
                        0x00
                    }
                    INST_RX_STATUS => {
                        self.phase = Phase::Instruction;
                        0x00
                    }
                    _ => 0,
                }
            }
            Phase::Address => {
                self.addr = mosi;
                self.phase = if self.inst == INST_BITMOD {
                    Phase::BitModMask
                } else {
                    Phase::Data
                };
                0
            }
            Phase::BitModMask => {
                self.bitmod_mask = mosi;
                self.phase = Phase::BitModData;
                0
            }
            Phase::BitModData => {
                let idx = self.addr as usize % self.regs.len();
                let cur = self.regs[idx];
                self.regs[idx] = (cur & !self.bitmod_mask) | (mosi & self.bitmod_mask);
                if self.addr == REG_CANCTRL {
                    self.sync_canstat_from_ctrl();
                }
                self.phase = Phase::Instruction;
                0
            }
            Phase::Data => {
                let idx = self.addr as usize % self.regs.len();
                let miso = if self.inst == INST_READ {
                    self.regs[idx]
                } else {
                    0
                };
                if self.inst == INST_WRITE {
                    self.regs[idx] = mosi;
                    if self.addr == REG_CANCTRL {
                        self.sync_canstat_from_ctrl();
                    }
                }
                self.addr = self.addr.wrapping_add(1);
                miso
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

pub struct Mcp2515Kit;
pub static MCP2515_KIT: Mcp2515Kit = Mcp2515Kit;

static MCP2515_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "mcp2515",
    label: "MCP2515 CAN",
    summary: "SPI CAN controller register shell (no bus frames).",
    detail: "Microchip MCP2515 RESET/READ/WRITE/BIT_MODIFY for CANCTRL/CANSTAT and \
             friends. CAN wire protocol is not simulated in this thin shell.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Chip-select GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for Mcp2515Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &MCP2515_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = Mcp2515::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_read_canctrl() {
        let mut dev = Mcp2515::new("PA4");
        dev.cs_select();
        dev.transfer(INST_RESET);
        dev.cs_release();
        dev.cs_select();
        dev.transfer(INST_READ);
        dev.transfer(REG_CANCTRL);
        let v = dev.transfer(0x00);
        assert_eq!(v, 0x87);
    }

    #[test]
    fn write_canctrl_updates_canstat_opmode() {
        let mut dev = Mcp2515::new("PA4");
        dev.cs_select();
        dev.transfer(INST_WRITE);
        dev.transfer(REG_CANCTRL);
        dev.transfer(0x00); // normal mode
        dev.cs_release();
        dev.cs_select();
        dev.transfer(INST_READ);
        dev.transfer(REG_CANSTAT);
        let st = dev.transfer(0);
        assert_eq!(st & 0xE0, 0x00);
    }
}
