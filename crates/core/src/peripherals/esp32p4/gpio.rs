// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! GPIO8 output slice for ESP32-P4.
//!
//! Register offsets follow Espressif ESP-IDF's ESP32-P4 `gpio_reg.h`:
//! GPIO8's output latch, output-enable latch, and output-matrix selector are
//! modelled. Other pads and GPIO features are deliberately unsupported.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock};

const GPIO8: u8 = 8;
const GPIO8_MASK: u32 = 1 << GPIO8;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x20;
const ENABLE_W1TS: u64 = 0x24;
const ENABLE_W1TC: u64 = 0x28;
const IN: u64 = 0x3C;
const FUNC8_OUT_SEL: u64 = 0x578; // GPIO base + 0x558 + GPIO8 * 4.

const SIG_GPIO_OUT: u32 = 256;
const OE_SEL: u32 = 1 << 10;
const GPIO_FUNCTION: u32 = 1;
const MCU_SEL_MASK: u32 = 0b111 << 12;

const GPIO_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32p4/gpio.yaml");

fn gpio_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(GPIO_DESCRIPTOR_YAML)
            .expect("embedded ESP32-P4 GPIO descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32p4Gpio {
    out: u32,
    enable: u32,
    func8_out_sel: u32,
    io_mux_gpio8: Option<super::io_mux::PadControl>,
    observers: Vec<Arc<dyn crate::peripherals::device::GpioObserver>>,
    cycle: u64,
}

impl Esp32p4Gpio {
    pub fn new() -> Self {
        Self {
            out: 0,
            enable: 0,
            // GPIO matrix output defaults to the GPIO_OUT latch; output enable
            // remains disabled until firmware sets OE_SEL and GPIO_ENABLE.
            func8_out_sel: SIG_GPIO_OUT,
            io_mux_gpio8: None,
            observers: Vec::new(),
            cycle: 0,
        }
    }

    pub fn add_observer(&mut self, observer: Arc<dyn crate::peripherals::device::GpioObserver>) {
        self.observers.push(observer);
    }

    pub(crate) fn set_io_mux_gpio8(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio8 = Some(control);
    }

    fn io_mux_selects_gpio8(&self) -> bool {
        self.io_mux_gpio8.as_ref().is_some_and(|control| {
            let word = *control
                .read()
                .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned");
            (word & MCU_SEL_MASK) >> 12 == GPIO_FUNCTION
        })
    }

    fn gpio8_output_selected(&self) -> bool {
        self.enable & GPIO8_MASK != 0
            && self.io_mux_selects_gpio8()
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

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out & GPIO8_MASK,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable & GPIO8_MASK,
            IN => u32::from(self.gpio8_level().unwrap_or(false)) << GPIO8,
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
            ENABLE => self.enable = value & GPIO8_MASK,
            ENABLE_W1TS => self.enable |= value & GPIO8_MASK,
            ENABLE_W1TC => self.enable &= !(value & GPIO8_MASK),
            FUNC8_OUT_SEL => self.func8_out_sel = value,
            _ => return,
        }
        self.notify_gpio8_transition(before, self.gpio8_level());
    }

    fn pin_level(&self, pin: u8) -> Option<bool> {
        (pin == GPIO8).then(|| self.gpio8_level()).flatten()
    }
}

impl Default for Esp32p4Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32p4Gpio {
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
            IN | FUNC8_OUT_SEL => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.gpio8_level()
            .map(|level| u32::from(level) << GPIO8)
            .unwrap_or(0)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.pin_level(pin)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin == GPIO8).then_some(self.out & GPIO8_MASK != 0)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        if pin != GPIO8 {
            return None;
        }
        Some(if self.gpio8_output_selected() {
            GpioRouting {
                mode: GpioMode::Output,
                func: None,
            }
        } else if self.io_mux_selects_gpio8() {
            GpioRouting {
                mode: GpioMode::Input,
                func: None,
            }
        } else {
            GpioRouting {
                mode: GpioMode::Af,
                func: None,
            }
        })
    }

    fn pins(&self) -> Option<&dyn PinPort> {
        Some(self)
    }

    fn pins_mut(&mut self) -> Option<&mut dyn PinPort> {
        Some(self)
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
            "func8_out_sel": self.func8_out_sel,
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio8_out").and_then(serde_json::Value::as_bool) {
            self.out = if value { GPIO8_MASK } else { 0 };
        }
        if let Some(value) = state.get("enable").and_then(serde_json::Value::as_u64) {
            self.enable = value as u32 & GPIO8_MASK;
        }
        if let Some(value) = state
            .get("func8_out_sel")
            .and_then(serde_json::Value::as_u64)
        {
            self.func8_out_sel = value as u32;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let io_mux = self.io_mux_gpio8.as_ref().map(|control| {
            *control
                .read()
                .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned")
        });
        bincode::serialize(&(
            self.out,
            self.enable,
            self.func8_out_sel,
            self.cycle,
            io_mux,
        ))
        .expect("serialize ESP32-P4 GPIO state")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (out, enable, selector, cycle, io_mux): (u32, u32, u32, u64, Option<u32>) =
                bincode::deserialize(bytes).map_err(|error| {
                    crate::SimulationError::NotImplemented(format!(
                        "ESP32-P4 GPIO runtime snapshot decode: {error}"
                    ))
                })?;
            self.out = out & GPIO8_MASK;
            self.enable = enable & GPIO8_MASK;
            self.func8_out_sel = selector;
            self.cycle = cycle;
            if let (Some(control), Some(value)) = (&self.io_mux_gpio8, io_mux) {
                *control
                    .write()
                    .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned") = value;
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

impl PinPort for Esp32p4Gpio {
    fn pin_count(&self) -> u8 {
        57
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        (pin == GPIO8).then(|| {
            self.gpio8_level()
                .map(|level| PadDriver::output(level, false))
                .unwrap_or_else(|| PadDriver::released(Pull::None))
        })
    }

    fn external(&self, _pin: u8) -> External {
        External::Released
    }

    fn set_external(&mut self, _pin: u8, _ext: External) -> Option<InputChange> {
        None
    }

    fn input(&self, pin: u8) -> Option<bool> {
        self.pin_level(pin)
    }
}
