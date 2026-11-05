// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 GPIO model for the DevKitC's onboard RGB LED data pin.
//!
//! Only GPIO27 is exposed through this model. Its GPIO_OUT latch, output
//! enable, GPIO_FUNC27_OUT_SEL_CFG and shared IO_MUX function select are live;
//! other pads, GPIO inputs, interrupts, and peripheral-matrix routes are not.

use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock};

const GPIO27: u8 = 27;
const GPIO27_MASK: u32 = 1 << GPIO27;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x34;
const ENABLE_W1TS: u64 = 0x38;
const ENABLE_W1TC: u64 = 0x3C;
const IN: u64 = 0x64;
const FUNC27_OUT_SEL: u64 = 0xB30;
const SIG_GPIO_OUT: u32 = 256;
const OE_SEL: u32 = 1 << 10;

const GPIO_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c5/gpio_gpio27.yaml");

fn gpio_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(GPIO_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C5 GPIO27 descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32c5Gpio {
    out: u32,
    enable: u32,
    func27_out_sel: u32,
    io_mux_gpio27: Option<super::io_mux::PadControl>,
    observers: Vec<Arc<dyn crate::peripherals::device::GpioObserver>>,
    cycle: u64,
}

impl Esp32c5Gpio {
    pub fn new() -> Self {
        Self {
            out: 0,
            enable: 0,
            // GPIO_FUNCn_OUT_SEL_CFG reset value is 0x100; OE_SEL is enabled
            // explicitly by firmware before it drives the pad.
            func27_out_sel: SIG_GPIO_OUT,
            io_mux_gpio27: None,
            observers: Vec::new(),
            cycle: 0,
        }
    }

    pub fn add_observer(&mut self, observer: Arc<dyn crate::peripherals::device::GpioObserver>) {
        self.observers.push(observer);
    }

    pub(crate) fn set_io_mux_gpio27(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio27 = Some(control);
    }

    fn io_mux_selects_gpio(&self) -> bool {
        self.io_mux_gpio27.as_ref().is_some_and(|control| {
            super::io_mux::gpio_function_selected(
                *control
                    .read()
                    .expect("ESP32-C5 GPIO27 IO_MUX control poisoned"),
            )
        })
    }

    fn gpio27_output_selected(&self) -> bool {
        self.enable & GPIO27_MASK != 0
            && self.func27_out_sel & 0x1FF == SIG_GPIO_OUT
            && self.func27_out_sel & OE_SEL != 0
            && self.io_mux_selects_gpio()
    }

    fn gpio27_level(&self) -> bool {
        self.gpio27_output_selected() && self.out & GPIO27_MASK != 0
    }

    fn notify_output_transition(&self, before: Option<bool>, after: Option<bool>) {
        if let (Some(from), Some(to)) = (before, after) {
            if from != to {
                for observer in &self.observers {
                    observer.on_pin_change(GPIO27, from, to, self.cycle);
                }
            }
        }
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable,
            IN => u32::from(self.gpio27_level()) << GPIO27,
            FUNC27_OUT_SEL => self.func27_out_sel,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let before = self
            .gpio27_output_selected()
            .then(|| self.out & GPIO27_MASK != 0);
        match offset {
            OUT => self.out = value & GPIO27_MASK,
            OUT_W1TS => self.out |= value & GPIO27_MASK,
            OUT_W1TC => self.out &= !(value & GPIO27_MASK),
            ENABLE => self.enable = value & GPIO27_MASK,
            ENABLE_W1TS => self.enable |= value & GPIO27_MASK,
            ENABLE_W1TC => self.enable &= !(value & GPIO27_MASK),
            FUNC27_OUT_SEL => self.func27_out_sel = value,
            _ => return,
        }
        let after = self
            .gpio27_output_selected()
            .then(|| self.out & GPIO27_MASK != 0);
        self.notify_output_transition(before, after);
    }
}

impl Default for Esp32c5Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c5Gpio {
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
        let byte_mask = 0xFFu32 << shift;
        if matches!(word_offset, OUT_W1TS | OUT_W1TC | ENABLE_W1TS | ENABLE_W1TC) {
            self.write_word(word_offset, u32::from(value) << shift);
        } else {
            let word = (self.read_word(word_offset) & !byte_mask) | (u32::from(value) << shift);
            self.write_word(word_offset, word);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_word(offset, value);
            Ok(())
        } else {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xFF) as u8)?;
            }
            Ok(())
        }
    }

    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        match offset & !3 {
            IN => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        (pin == GPIO27).then(|| self.gpio27_level())
    }

    fn read_gpio_input_word(&self) -> u32 {
        u32::from(self.gpio27_level()) << GPIO27
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        (pin == GPIO27).then(|| self.out & GPIO27_MASK != 0)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        (pin == GPIO27).then(|| GpioRouting {
            mode: if self.gpio27_output_selected() {
                GpioMode::Output
            } else {
                GpioMode::Input
            },
            func: None,
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
            "gpio27_out": self.out & GPIO27_MASK != 0,
            "gpio27_enabled": self.enable & GPIO27_MASK != 0,
            "gpio27_out_sel": self.func27_out_sel,
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio27_out").and_then(serde_json::Value::as_bool) {
            self.out = if value { GPIO27_MASK } else { 0 };
        }
        if let Some(value) = state
            .get("gpio27_enabled")
            .and_then(serde_json::Value::as_bool)
        {
            self.enable = if value { GPIO27_MASK } else { 0 };
        }
        if let Some(value) = state
            .get("gpio27_out_sel")
            .and_then(serde_json::Value::as_u64)
        {
            self.func27_out_sel = value as u32;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&(self.out, self.enable, self.func27_out_sel, self.cycle))
            .expect("serialize ESP32-C5 GPIO27 state")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (out, enable, func27_out_sel, cycle): (u32, u32, u32, u64) =
                bincode::deserialize(bytes).map_err(|error| {
                    crate::SimulationError::NotImplemented(format!(
                        "ESP32-C5 GPIO runtime snapshot decode: {error}"
                    ))
                })?;
            self.out = out & GPIO27_MASK;
            self.enable = enable & GPIO27_MASK;
            self.func27_out_sel = func27_out_sel;
            self.cycle = cycle;
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

impl PinPort for Esp32c5Gpio {
    fn pin_count(&self) -> u8 {
        GPIO27 + 1
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        if pin != GPIO27 {
            return None;
        }
        Some(if self.gpio27_output_selected() {
            PadDriver::output(self.out & GPIO27_MASK != 0, false)
        } else {
            PadDriver::released(Pull::None)
        })
    }

    fn external(&self, _pin: u8) -> External {
        External::Released
    }

    fn set_external(&mut self, _pin: u8, _ext: External) -> Option<InputChange> {
        None
    }

    fn input(&self, pin: u8) -> Option<bool> {
        (pin == GPIO27).then(|| self.gpio27_level())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::device::GpioObserver;

    #[derive(Debug, Default)]
    struct Edges(std::sync::Mutex<Vec<(u8, bool, bool)>>);

    impl GpioObserver for Edges {
        fn on_pin_change(&self, pin: u8, from: bool, to: bool, _sim_cycle: u64) {
            self.0.lock().unwrap().push((pin, from, to));
        }
    }

    #[test]
    fn gpio27_output_observer_requires_gpio_mux_and_gpio_output_selection() {
        let mut gpio = Esp32c5Gpio::new();
        let iomux = super::super::io_mux::Esp32c5IoMux::new();
        gpio.set_io_mux_gpio27(iomux.pad_control());
        let edges = Arc::new(Edges::default());
        gpio.add_observer(edges.clone());

        // Enabling the GPIO latch without the matrix OE_SEL has no pad drive.
        gpio.write_u32(ENABLE_W1TS, GPIO27_MASK).unwrap();
        gpio.write_u32(OUT_W1TS, GPIO27_MASK).unwrap();
        assert!(edges.0.lock().unwrap().is_empty());

        // Select the direct GPIO output plus OE and drive low before toggling.
        gpio.write_u32(OUT_W1TC, GPIO27_MASK).unwrap();
        gpio.write_u32(FUNC27_OUT_SEL, SIG_GPIO_OUT | OE_SEL)
            .unwrap();
        gpio.write_u32(OUT_W1TS, GPIO27_MASK).unwrap();
        assert_eq!(*edges.0.lock().unwrap(), vec![(GPIO27, false, true)]);

        // Leaving GPIO function 1 disconnects the pin from the board LED.
        let mut mux = iomux;
        mux.write_u32(0x6C, 2 << 12).unwrap();
        assert_eq!(gpio.read_gpio_input(GPIO27), Some(false));
        assert_eq!(*edges.0.lock().unwrap(), vec![(GPIO27, false, true)]);
    }
}
