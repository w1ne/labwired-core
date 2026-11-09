// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO7 input and GPIO8 output slice for ESP32-S31.
//!
//! Offsets follow Espressif's ESP-IDF `esp32s31/register/soc/gpio_reg.h`.
//! The remainder of the S31 GPIO matrix is deliberately unsupported.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::{MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock};

const GPIO7: u8 = 7;
const GPIO8: u8 = 8;
const GPIO7_MASK: u32 = 1 << GPIO7;
const GPIO8_MASK: u32 = 1 << GPIO8;
const SUPPORTED_MASK: u32 = GPIO7_MASK | GPIO8_MASK;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x34;
const ENABLE_W1TS: u64 = 0x38;
const ENABLE_W1TC: u64 = 0x3C;
const IN: u64 = 0x64;
const FUNC7_OUT_SEL: u64 = 0xB10;
const FUNC8_OUT_SEL: u64 = 0xB14;

const SIG_GPIO_OUT: u32 = 256;
const OE_SEL: u32 = 1 << 10;
const GPIO_FUNCTION: u32 = 1;
const MCU_SEL_MASK: u32 = 0b111 << 12;
const FUN_IE: u32 = 1 << 9;
const FUN_PU: u32 = 1 << 8;

const GPIO_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32s31/gpio.yaml");

fn gpio_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(GPIO_DESCRIPTOR_YAML)
            .expect("embedded ESP32-S31 GPIO descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32s31Gpio {
    out: u32,
    enable: u32,
    func7_out_sel: u32,
    func8_out_sel: u32,
    gpio7_external: Option<bool>,
    io_mux_gpio7: Option<super::io_mux::PadControl>,
    io_mux_gpio8: Option<super::io_mux::PadControl>,
    observers: Vec<Arc<dyn crate::peripherals::device::GpioObserver>>,
    cycle: u64,
}

impl Esp32s31Gpio {
    pub fn new() -> Self {
        Self {
            out: 0,
            enable: 0,
            func7_out_sel: SIG_GPIO_OUT,
            func8_out_sel: SIG_GPIO_OUT,
            gpio7_external: None,
            io_mux_gpio7: None,
            io_mux_gpio8: None,
            observers: Vec::new(),
            cycle: 0,
        }
    }

    pub fn add_observer(&mut self, observer: Arc<dyn crate::peripherals::device::GpioObserver>) {
        self.observers.push(observer);
    }

    pub(crate) fn set_io_mux_gpio7(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio7 = Some(control);
    }

    pub(crate) fn set_io_mux_gpio8(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio8 = Some(control);
    }

    fn pad_word(control: &Option<super::io_mux::PadControl>) -> Option<u32> {
        control.as_ref().map(|control| {
            *control
                .read()
                .expect("ESP32-S31 IO_MUX pad control poisoned")
        })
    }

    fn io_mux_selects_gpio(word: Option<u32>) -> bool {
        word.is_some_and(|word| (word & MCU_SEL_MASK) >> 12 == GPIO_FUNCTION)
    }

    fn gpio7_level(&self) -> Option<bool> {
        let word = Self::pad_word(&self.io_mux_gpio7)?;
        if !Self::io_mux_selects_gpio(Some(word)) || word & FUN_IE == 0 {
            return None;
        }
        Some(self.gpio7_external.unwrap_or(word & FUN_PU != 0))
    }

    fn gpio8_output_selected(&self) -> bool {
        self.enable & GPIO8_MASK != 0
            && Self::io_mux_selects_gpio(Self::pad_word(&self.io_mux_gpio8))
            && self.func8_out_sel & 0x1FF == SIG_GPIO_OUT
            && self.func8_out_sel & OE_SEL != 0
    }

    fn gpio8_level(&self) -> Option<bool> {
        self.gpio8_output_selected()
            .then_some(self.out & GPIO8_MASK != 0)
    }

    fn notify_gpio8_transition(&self, before: Option<bool>, after: Option<bool>) {
        if let (Some(from), Some(to)) = (before, after) {
            if from != to {
                for observer in &self.observers {
                    observer.on_pin_change(GPIO8, from, to, self.cycle);
                }
            }
        }
    }

    fn input_word(&self) -> u32 {
        let mut value = self
            .gpio7_level()
            .map_or(0, |level| u32::from(level) << GPIO7);
        value |= self
            .gpio8_level()
            .map_or(0, |level| u32::from(level) << GPIO8);
        value
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out & GPIO8_MASK,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable & SUPPORTED_MASK,
            IN => self.input_word(),
            FUNC7_OUT_SEL => self.func7_out_sel,
            FUNC8_OUT_SEL => self.func8_out_sel,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let before = self.gpio8_level();
        match offset {
            OUT => self.out = value & GPIO8_MASK,
            OUT_W1TS => self.out |= value & GPIO8_MASK,
            OUT_W1TC => self.out &= !(value & GPIO8_MASK),
            ENABLE => self.enable = value & SUPPORTED_MASK,
            ENABLE_W1TS => self.enable |= value & SUPPORTED_MASK,
            ENABLE_W1TC => self.enable &= !(value & SUPPORTED_MASK),
            FUNC7_OUT_SEL => self.func7_out_sel = value,
            FUNC8_OUT_SEL => self.func8_out_sel = value,
            _ => return,
        }
        self.notify_gpio8_transition(before, self.gpio8_level());
    }
}

impl Default for Esp32s31Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s31Gpio {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let shift = (offset & 3) * 8;
        if matches!(word_offset, OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC) {
            self.write_word(word_offset, u32::from(value) << shift);
        } else {
            let byte_mask = 0xFFu32 << shift;
            let word = (self.read_word(word_offset) & !byte_mask) | (u32::from(value) << shift);
            self.write_word(word_offset, word);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
        } else {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xFF) as u8)?;
            }
        }
        Ok(())
    }

    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        match offset & !3 {
            IN | FUNC7_OUT_SEL | FUNC8_OUT_SEL => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.input_word()
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        match pin {
            GPIO7 => self.gpio7_level(),
            GPIO8 => self.gpio8_level(),
            _ => None,
        }
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin == GPIO8).then_some(self.out & GPIO8_MASK != 0)
    }

    fn set_gpio_input(&mut self, pin: u8, level: bool) -> bool {
        if pin != GPIO7 {
            return false;
        }
        self.gpio7_external = Some(level);
        true
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        let pad = match pin {
            GPIO7 => Self::pad_word(&self.io_mux_gpio7),
            GPIO8 => Self::pad_word(&self.io_mux_gpio8),
            _ => return None,
        };
        if !Self::io_mux_selects_gpio(pad) {
            return Some(GpioRouting {
                mode: GpioMode::Af,
                func: None,
            });
        }
        if pin == GPIO8 && self.gpio8_output_selected() {
            return Some(GpioRouting {
                mode: GpioMode::Output,
                func: None,
            });
        }
        Some(if pad.is_some_and(|word| word & FUN_IE != 0) {
            GpioRouting {
                mode: GpioMode::Input,
                func: None,
            }
        } else {
            GpioRouting {
                mode: GpioMode::Analog,
                func: None,
            }
        })
    }

    fn tick(&mut self) -> PeripheralTickResult {
        self.cycle = self.cycle.wrapping_add(1);
        PeripheralTickResult::default()
    }

    fn sync_to(&mut self, now_cycle: u64) {
        self.cycle = self.cycle.max(now_cycle);
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "gpio8_out": self.out & GPIO8_MASK != 0,
            "enable": self.enable,
            "func7_out_sel": self.func7_out_sel,
            "func8_out_sel": self.func8_out_sel,
            "gpio7_external": self.gpio7_external,
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio8_out").and_then(serde_json::Value::as_bool) {
            self.out = if value { GPIO8_MASK } else { 0 };
        }
        if let Some(value) = state.get("enable").and_then(serde_json::Value::as_u64) {
            self.enable = value as u32 & SUPPORTED_MASK;
        }
        if let Some(value) = state
            .get("func7_out_sel")
            .and_then(serde_json::Value::as_u64)
        {
            self.func7_out_sel = value as u32;
        }
        if let Some(value) = state
            .get("func8_out_sel")
            .and_then(serde_json::Value::as_u64)
        {
            self.func8_out_sel = value as u32;
        }
        self.gpio7_external = state
            .get("gpio7_external")
            .and_then(serde_json::Value::as_bool);
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let pad7 = Self::pad_word(&self.io_mux_gpio7);
        let pad8 = Self::pad_word(&self.io_mux_gpio8);
        bincode::serialize(&(
            self.out,
            self.enable,
            self.func7_out_sel,
            self.func8_out_sel,
            self.gpio7_external,
            self.cycle,
            pad7,
            pad8,
        ))
        .expect("serialize ESP32-S31 GPIO state")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (out, enable, func7, func8, gpio7_external, cycle, pad7, pad8): (
                u32,
                u32,
                u32,
                u32,
                Option<bool>,
                u64,
                Option<u32>,
                Option<u32>,
            ) = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-S31 GPIO runtime snapshot decode: {error}"
                ))
            })?;
            self.out = out & GPIO8_MASK;
            self.enable = enable & SUPPORTED_MASK;
            self.func7_out_sel = func7;
            self.func8_out_sel = func8;
            self.gpio7_external = gpio7_external;
            self.cycle = cycle;
            if let (Some(control), Some(value)) = (&self.io_mux_gpio7, pad7) {
                *control
                    .write()
                    .expect("ESP32-S31 IO_MUX GPIO7 control poisoned") = value;
            }
            if let (Some(control), Some(value)) = (&self.io_mux_gpio8, pad8) {
                *control
                    .write()
                    .expect("ESP32-S31 IO_MUX GPIO8 control poisoned") = value;
            }
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(gpio_descriptor().clone())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
