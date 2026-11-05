// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Bounded PIC32MZ GPIO port model used by the MIPS-family plugin hook.
//!
//! This implements the digital input/output path for a declared port and the
//! PIC32MZ ANSEL/TRIS/PORT/LAT/ODC register groups. It does not model PPS,
//! change notification, pulls, or GPIO interrupts.
//! Register offsets and reset values follow PIC32MZ EF data sheet DS60001320,
//! Table 12-5 (PORTB register map).

use crate::peripherals::gpio::GpioPortOffsets;
use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, Out, PadDriver, PinPort, Pull};
use crate::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use crate::{Peripheral, SimResult};
use labwired_config::PeripheralConfig;

const ANSEL: u64 = 0x00;
const TRIS: u64 = 0x10;
const PORT: u64 = 0x20;
const LAT: u64 = 0x30;
const ODC: u64 = 0x40;

#[derive(Debug, Clone, Copy)]
enum Register {
    Ansel,
    Tris,
    Port,
    Lat,
    Odc,
}

#[derive(Debug)]
struct Pic32GpioPort {
    pin_count: u8,
    ansel: u32,
    tris: u32,
    latch: u32,
    odc: u32,
    external: [External; 32],
}

impl Pic32GpioPort {
    fn new(pin_count: u8) -> Self {
        let pin_mask = if pin_count == 32 {
            u32::MAX
        } else {
            (1u32 << pin_count) - 1
        };
        Self {
            pin_count,
            // The EF family resets analog-capable ANSEL bits high. This
            // bounded model covers a 16-bit PORTB instance.
            ansel: pin_mask,
            tris: pin_mask,
            latch: 0,
            odc: 0,
            external: [External::Released; 32],
        }
    }

    fn pin_mask(&self) -> u32 {
        if self.pin_count == 32 {
            u32::MAX
        } else {
            (1u32 << self.pin_count) - 1
        }
    }

    fn register_for_offset(offset: u64) -> Option<(Register, u8)> {
        let group = offset & !0xF;
        let alias = (offset & 0xF) as u8 & !0x3;
        let register = match group {
            ANSEL => Register::Ansel,
            TRIS => Register::Tris,
            PORT => Register::Port,
            LAT => Register::Lat,
            ODC => Register::Odc,
            _ => return None,
        };
        Some((register, alias))
    }

    fn get_register(&self, register: Register) -> u32 {
        match register {
            Register::Ansel => self.ansel,
            Register::Tris => self.tris,
            Register::Port => self.port_bits(),
            Register::Lat => self.latch,
            Register::Odc => self.odc,
        }
    }

    fn set_register(&mut self, register: Register, value: u32) {
        let value = value & self.pin_mask();
        match register {
            Register::Ansel => self.ansel = value,
            Register::Tris => self.tris = value,
            // PORT is a write alias for the output latch.
            Register::Port | Register::Lat => self.latch = value,
            Register::Odc => self.odc = value,
        }
    }

    fn port_bits(&self) -> u32 {
        (0..self.pin_count).fold(0, |bits, pin| {
            if self.input(pin).unwrap_or(false) {
                bits | (1 << pin)
            } else {
                bits
            }
        })
    }
}

impl Peripheral for Pic32GpioPort {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let Some((register, _alias)) = Self::register_for_offset(offset) else {
            return Ok(0);
        };
        let value = self.get_register(register);
        let byte = ((offset & 0x3) * 8) as u32;
        Ok((value >> byte) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let Some((register, alias)) = Self::register_for_offset(offset) else {
            return Ok(());
        };
        if !matches!(alias, 0 | 4 | 8 | 12) {
            return Ok(());
        }
        let target = if matches!(register, Register::Port) {
            Register::Lat
        } else {
            register
        };
        let shift = ((offset & 0x3) * 8) as u32;
        let byte_mask = 0xFFu32 << shift;
        let byte_value = (value as u32) << shift;
        let current = self.get_register(target);
        let next = match alias {
            4 => current & !(byte_value & byte_mask),
            8 => current | (byte_value & byte_mask),
            12 => current ^ (byte_value & byte_mask),
            _ => (current & !byte_mask) | byte_value,
        };
        self.set_register(target, next);
        Ok(())
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
    }

    fn gpio_port_offsets(&self) -> Option<GpioPortOffsets> {
        Some(GpioPortOffsets {
            output: LAT,
            input: PORT,
        })
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        PinPort::input(self, pin)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then(|| self.latch & (1 << pin) != 0)
    }

    fn read_gpio_is_output(&self, pin: u8) -> Option<bool> {
        (pin < self.pin_count).then(|| self.tris & (1 << pin) == 0)
    }

    fn gpio_input_is_released(&self) -> bool {
        true
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u32 << pin;
        let mode = if self.tris & bit == 0 {
            GpioMode::Output
        } else if self.ansel & bit != 0 {
            GpioMode::Analog
        } else {
            GpioMode::Input
        };
        Some(GpioRouting { mode, func: None })
    }
}

impl PinPort for Pic32GpioPort {
    fn pin_count(&self) -> u8 {
        self.pin_count
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u32 << pin;
        if self.tris & bit != 0 {
            return Some(PadDriver::released(Pull::None));
        }
        let high = self.latch & bit != 0;
        let out = if self.odc & bit != 0 && high {
            Out::Off
        } else {
            Out::from_level(high)
        };
        Some(PadDriver {
            out,
            pull: Pull::None,
        })
    }

    fn external(&self, pin: u8) -> External {
        self.external
            .get(pin as usize)
            .copied()
            .unwrap_or(External::Released)
    }

    fn set_external(&mut self, pin: u8, ext: External) -> Option<InputChange> {
        if pin >= self.pin_count {
            return None;
        }
        let before = self.input(pin)?;
        self.external[pin as usize] = ext;
        let after = self.input(pin)?;
        Some(InputChange { before, after })
    }

    fn input(&self, pin: u8) -> Option<bool> {
        if pin >= self.pin_count {
            return None;
        }
        let bit = 1u32 << pin;
        // Analog mode disconnects the digital input buffer; PORT reads zero.
        if self.ansel & bit != 0 {
            return Some(false);
        }
        let external = self.external(pin);
        if self.tris & bit != 0 {
            Some(external.level().unwrap_or(false))
        } else {
            Some(
                crate::pins::resolve(self.driver(pin)?, external)
                    .level
                    .unwrap_or(false),
            )
        }
    }
}

/// MIPS-only plugin for the PIC32MZ GPIO register block.
#[derive(Debug)]
pub struct Pic32GpioPlugin;

impl ChipPlugin for Pic32GpioPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }

    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        // `pic32mz_port` intentionally avoids the legacy fuzzy `gpio` type
        // canonicalizer; this plugin owns the concrete family register map.
        if ctx.canonical_type != "pic32mz_port" {
            return None;
        }
        Some((|| {
            let pin_count = p_cfg
                .config
                .get("pin_count")
                .and_then(serde_yaml::Value::as_u64)
                .unwrap_or(16);
            let pin_count = u8::try_from(pin_count)
                .ok()
                .filter(|count| (1..=32).contains(count))
                .ok_or_else(|| anyhow::anyhow!("PIC32 GPIO pin_count must be in 1..=32"))?;
            Ok(Box::new(Pic32GpioPort::new(pin_count)) as Box<dyn Peripheral>)
        })())
    }
}
