// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! microSD SPI command shell — no filesystem.
//!
//! Handles CMD0 (GO_IDLE), CMD8 (SEND_IF_COND), CMD55/ACMD41, CMD58 (OCR),
//! and CMD17 (READ_SINGLE_BLOCK) with zeros payload. Enough for common
//! Arduino SdFat init paths to leave idle and report a card present.

use crate::peripherals::spi::SpiDevice;
use std::any::Any;
use std::collections::VecDeque;

pub struct MicroSd {
    cs_pin: String,
    cmd_buf: Vec<u8>,
    out: VecDeque<u8>,
    idle: bool,
    initialized: bool,
    component_id: Option<String>,
}

impl MicroSd {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            cmd_buf: Vec::new(),
            out: VecDeque::new(),
            idle: true,
            initialized: false,
            component_id: None,
        }
    }

    fn r1(&self, bits: u8) -> u8 {
        bits
    }

    fn handle_command(&mut self) {
        if self.cmd_buf.len() < 6 {
            return;
        }
        let index = self.cmd_buf[0] & 0x3F;
        let arg = u32::from_be_bytes([
            self.cmd_buf[1],
            self.cmd_buf[2],
            self.cmd_buf[3],
            self.cmd_buf[4],
        ]);
        self.cmd_buf.clear();

        match index {
            0 => {
                // CMD0 GO_IDLE_STATE
                self.idle = true;
                self.initialized = false;
                self.out.push_back(self.r1(0x01)); // idle
            }
            8 => {
                // CMD8 SEND_IF_COND — R7: R1 + echo arg low 32
                self.out.push_back(self.r1(0x01));
                self.out.push_back(0x00);
                self.out.push_back(0x00);
                self.out.push_back(0x01);
                self.out.push_back(0xAA);
            }
            55 => {
                // APP_CMD
                self.out.push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
            }
            41 => {
                // ACMD41 — leave idle
                self.idle = false;
                self.initialized = true;
                self.out.push_back(self.r1(0x00));
            }
            58 => {
                // CMD58 READ_OCR — R3
                self.out.push_back(self.r1(if self.idle { 0x01 } else { 0x00 }));
                self.out.push_back(0xC0); // CCS + power up
                self.out.push_back(0xFF);
                self.out.push_back(0x80);
                self.out.push_back(0x00);
            }
            17 => {
                // CMD17 READ_SINGLE_BLOCK
                let _ = arg;
                self.out.push_back(self.r1(0x00));
                self.out.push_back(0xFE); // data token
                for _ in 0..512 {
                    self.out.push_back(0x00);
                }
                self.out.push_back(0xFF); // CRC
                self.out.push_back(0xFF);
            }
            24 => {
                // CMD24 WRITE — accept and ignore data for shell
                self.out.push_back(self.r1(0x00));
            }
            _ => {
                self.out
                    .push_back(self.r1(if self.idle { 0x01 } else { 0x04 })); // illegal
            }
        }
    }
}

impl SpiDevice for MicroSd {
    fn cs_pin(&self) -> &str {
        &self.cs_pin
    }

    fn cs_select(&mut self) {
        self.cmd_buf.clear();
    }

    fn cs_release(&mut self) {
        self.cmd_buf.clear();
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        if let Some(b) = self.out.pop_front() {
            // Still accept MOSI into command if host sends while draining
            if (mosi & 0xC0) == 0x40 && self.cmd_buf.is_empty() {
                self.cmd_buf.push(mosi);
            } else if !self.cmd_buf.is_empty() && self.cmd_buf.len() < 6 {
                self.cmd_buf.push(mosi);
                if self.cmd_buf.len() == 6 {
                    self.handle_command();
                }
            }
            return b;
        }

        // Idle MISO is 0xFF until a command starts
        if (mosi & 0xC0) == 0x40 || !self.cmd_buf.is_empty() {
            self.cmd_buf.push(mosi);
            if self.cmd_buf.len() == 6 {
                self.handle_command();
                return self.out.pop_front().unwrap_or(0xFF);
            }
            return 0xFF;
        }
        0xFF
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

pub struct MicroSdKit;
pub static MICROSD_KIT: MicroSdKit = MicroSdKit;

static MICROSD_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "microsd",
    label: "microSD (SPI)",
    summary: "SPI SD card command shell (init + single-block read zeros).",
    detail: "CMD0/8/55/41/58/17 shell for common Arduino card-init paths. No real \
             filesystem; CMD17 returns a 512-byte zero block.",
    transport: Transport::Spi,
    category: Category::Spi,
    config_keys: &[ConfigKey {
        name: "cs_pin",
        ty: ConfigType::Str,
        doc: "Card CS GPIO pin (e.g. \"PA4\"). Defaults to PA4.",
    }],
    labs: &[],
};

impl PeripheralKit for MicroSdKit {
    fn metadata(&self) -> &'static KitMetadata {
        &MICROSD_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let cs = ctx.config_str("cs_pin").unwrap_or("PA4").to_string();
        let mut dev = MicroSd::new(cs);
        dev.component_id = Some(ctx.device_id().to_string());
        ctx.attach_spi_device(Box::new(dev))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd0_returns_idle() {
        let mut dev = MicroSd::new("PA4");
        // CMD0 + arg0 + crc 0x95
        let cmd = [0x40, 0, 0, 0, 0, 0x95];
        dev.cs_select();
        let mut last = 0xFF;
        for &b in &cmd {
            last = dev.transfer(b);
        }
        // response may appear on last CRC or next FF
        if last == 0xFF {
            last = dev.transfer(0xFF);
        }
        assert_eq!(last, 0x01);
    }
}
