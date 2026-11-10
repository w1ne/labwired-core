// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 GPIO slice for DevKitC I2C sensors and the onboard RGB LED.
//!
//! GPIO6 and GPIO10 carry the modeled I2C0 GPIO-matrix route. GPIO27 carries
//! either direct GPIO output or RMT channel 0 to the onboard WS2812 LED. Other
//! pads and GPIO features are outside this profile.

use crate::peripherals::esp32c3::i2c::{C3I2cMatrixRoute, C3I2cMatrixRouteState};
use crate::peripherals::gpio::{GpioMode, GpioRouting};
use crate::pins::{External, InputChange, PadDriver, PinPort, Pull};
use crate::{MmioAccessClass, Peripheral, PeripheralTickResult, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, Mutex, OnceLock};

const GPIO6: u8 = 6;
const GPIO10: u8 = 10;
const GPIO27: u8 = 27;
const GPIO6_MASK: u32 = 1 << GPIO6;
const GPIO10_MASK: u32 = 1 << GPIO10;
const GPIO27_MASK: u32 = 1 << GPIO27;
const MODELED_PINS_MASK: u32 = GPIO6_MASK | GPIO10_MASK | GPIO27_MASK;

const OUT: u64 = 0x04;
const OUT_W1TS: u64 = 0x08;
const OUT_W1TC: u64 = 0x0C;
const ENABLE: u64 = 0x34;
const ENABLE_W1TS: u64 = 0x38;
const ENABLE_W1TC: u64 = 0x3C;
const IN: u64 = 0x64;

// ESP-IDF C5 gpio_reg.h: output selector words are per-pad. In particular,
// GPIO_FUNC27_OUT_SEL_CFG is 0xB40; 0xB30 belongs to GPIO_FUNC23.
const FUNC6_OUT_SEL: u64 = 0xAEC;
const FUNC10_OUT_SEL: u64 = 0xAFC;
const FUNC27_OUT_SEL: u64 = 0xB40;
// Input selector words are indexed by peripheral input signal, not GPIO pad.
const FUNC46_IN_SEL: u64 = 0x38C; // I2CEXT0_SCL
const FUNC47_IN_SEL: u64 = 0x390; // I2CEXT0_SDA

const SIG_I2CEXT0_SCL: u32 = 46;
const SIG_I2CEXT0_SDA: u32 = 47;
const SIG_RMT_OUT0: u32 = 64;
const SIG_RMT_OUT1: u32 = 65;
const SIG_GPIO_OUT: u32 = 256;
const OE_SEL: u32 = 1 << 10;
const INPUT_MATRIX_SELECT: u32 = 1 << 8;
const INPUT_PAD_MASK: u32 = 0x7F;

const GPIO_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c5/gpio_i2c_rgb.yaml");

fn gpio_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(GPIO_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C5 GPIO I2C/RGB descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32c5Gpio {
    out: u32,
    enable: u32,
    func6_out_sel: u32,
    func10_out_sel: u32,
    func27_out_sel: u32,
    rmt27_level: Option<bool>,
    func46_in_sel: u32,
    func47_in_sel: u32,
    io_mux_gpio6: Option<super::io_mux::PadControl>,
    io_mux_gpio10: Option<super::io_mux::PadControl>,
    io_mux_gpio27: Option<super::io_mux::PadControl>,
    i2c_matrix_route: C3I2cMatrixRoute,
    i2c_lines: Option<Arc<crate::peripherals::esp32c3::i2c::I2cLineLevels>>,
    observers: Vec<Arc<dyn crate::peripherals::device::GpioObserver>>,
    cycle: u64,
}

impl Esp32c5Gpio {
    pub fn new() -> Self {
        Self {
            out: 0,
            enable: 0,
            // GPIO_FUNCn_OUT_SEL_CFG resets to GPIO_OUT (256). Firmware must
            // select the GPIO matrix output-enable path before driving a pad.
            func6_out_sel: SIG_GPIO_OUT,
            func10_out_sel: SIG_GPIO_OUT,
            func27_out_sel: SIG_GPIO_OUT,
            rmt27_level: None,
            // C5 GPIO_FUNCn_IN_SEL_CFG resets to constant-low input (0x60).
            func46_in_sel: 0x60,
            func47_in_sel: 0x60,
            io_mux_gpio6: None,
            io_mux_gpio10: None,
            io_mux_gpio27: None,
            i2c_matrix_route: Arc::new(Mutex::new(C3I2cMatrixRouteState::default())),
            i2c_lines: None,
            observers: Vec::new(),
            cycle: 0,
        }
    }

    pub fn add_observer(&mut self, observer: Arc<dyn crate::peripherals::device::GpioObserver>) {
        self.observers.push(observer);
    }

    pub(crate) fn set_io_mux_gpio6(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio6 = Some(control);
        self.refresh_i2c_matrix_route();
    }

    pub(crate) fn set_io_mux_gpio10(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio10 = Some(control);
        self.refresh_i2c_matrix_route();
    }

    pub(crate) fn set_io_mux_gpio27(&mut self, control: super::io_mux::PadControl) {
        self.io_mux_gpio27 = Some(control);
    }

    pub(crate) fn i2c_matrix_route_state(&self) -> C3I2cMatrixRoute {
        self.i2c_matrix_route.clone()
    }

    pub(crate) fn set_i2c_lines(
        &mut self,
        lines: Arc<crate::peripherals::esp32c3::i2c::I2cLineLevels>,
    ) {
        self.i2c_lines = Some(lines);
    }

    fn io_mux_selects_gpio(&self, pin: u8) -> bool {
        let control = match pin {
            GPIO6 => self.io_mux_gpio6.as_ref(),
            GPIO10 => self.io_mux_gpio10.as_ref(),
            GPIO27 => self.io_mux_gpio27.as_ref(),
            _ => None,
        };
        control.is_some_and(|control| {
            super::io_mux::gpio_function_selected(
                *control
                    .read()
                    .expect("ESP32-C5 IO_MUX pad control poisoned"),
            )
        })
    }

    fn output_selector(&self, pin: u8) -> Option<u32> {
        Some(match pin {
            GPIO6 => self.func6_out_sel,
            GPIO10 => self.func10_out_sel,
            GPIO27 => self.func27_out_sel,
            _ => return None,
        })
    }

    fn matrix_output_selected(&self, pin: u8, signal: u32) -> bool {
        let mask = 1u32 << pin;
        self.enable & mask != 0
            && self.io_mux_selects_gpio(pin)
            && self
                .output_selector(pin)
                .is_some_and(|selector| selector & 0x1FF == signal && selector & OE_SEL != 0)
    }

    fn gpio27_output_selected(&self) -> bool {
        self.enable & GPIO27_MASK != 0
            && self.io_mux_selects_gpio(GPIO27)
            && self.func27_out_sel & OE_SEL != 0
    }

    fn gpio27_level(&self) -> bool {
        if !self.gpio27_output_selected() {
            return false;
        }
        match self.func27_out_sel & 0x1FF {
            SIG_GPIO_OUT => self.out & GPIO27_MASK != 0,
            SIG_RMT_OUT0 | SIG_RMT_OUT1 => self.rmt27_level.unwrap_or(false),
            _ => false,
        }
    }

    /// Resolve an active C5 GPIO-matrix signal to the modeled board pad.
    pub(crate) fn pads_for_output_signal(&self, signal: u32) -> Vec<u8> {
        if self.matrix_output_selected(GPIO27, signal) {
            vec![GPIO27]
        } else {
            Vec::new()
        }
    }

    /// Deliver a timed peripheral edge through the selected GPIO27 matrix
    /// route, then notify attached board devices such as the WS2812 model.
    pub(crate) fn drive_pad_output_at(&mut self, pin: u8, level: bool, sim_cycle: u64) -> bool {
        if pin != GPIO27
            || !self.gpio27_output_selected()
            || !matches!(self.func27_out_sel & 0x1FF, SIG_RMT_OUT0 | SIG_RMT_OUT1)
        {
            return false;
        }
        let before = self.pin_level(GPIO27);
        self.rmt27_level = Some(level);
        self.cycle = sim_cycle;
        let after = self.pin_level(GPIO27);
        self.notify_gpio27_transition(before, after);
        true
    }

    fn i2c_pad_level(&self, pin: u8) -> Option<bool> {
        if self.matrix_output_selected(pin, SIG_I2CEXT0_SCL) {
            return self.i2c_lines.as_ref().map(|lines| lines.scl());
        }
        if self.matrix_output_selected(pin, SIG_I2CEXT0_SDA) {
            return self.i2c_lines.as_ref().map(|lines| lines.sda());
        }
        None
    }

    fn pin_level(&self, pin: u8) -> Option<bool> {
        match pin {
            GPIO27 => Some(self.gpio27_level()),
            GPIO6 | GPIO10 => self.i2c_pad_level(pin),
            _ => None,
        }
    }

    fn notify_gpio27_transition(&self, before: Option<bool>, after: Option<bool>) {
        if let (Some(from), Some(to)) = (before, after) {
            if from != to {
                for observer in &self.observers {
                    observer.on_pin_change(GPIO27, from, to, self.cycle);
                }
            }
        }
    }

    fn selected_input_pin(selector: u32) -> Option<u8> {
        if selector & INPUT_MATRIX_SELECT == 0 {
            return None;
        }
        let pin = (selector & INPUT_PAD_MASK) as u8;
        (pin < 29).then_some(pin)
    }

    fn refresh_i2c_matrix_route(&self) {
        let sda_output_mask = if self.matrix_output_selected(GPIO6, SIG_I2CEXT0_SDA) {
            GPIO6_MASK
        } else {
            0
        };
        let scl_output_mask = if self.matrix_output_selected(GPIO10, SIG_I2CEXT0_SCL) {
            GPIO10_MASK
        } else {
            0
        };
        self.i2c_matrix_route
            .lock()
            .expect("ESP32-C5 I2C matrix route poisoned")
            .set(
                sda_output_mask,
                scl_output_mask,
                Self::selected_input_pin(self.func47_in_sel),
                Self::selected_input_pin(self.func46_in_sel),
            );
    }

    fn read_word(&self, offset: u64) -> u32 {
        match offset {
            OUT | OUT_W1TS | OUT_W1TC => self.out & MODELED_PINS_MASK,
            ENABLE | ENABLE_W1TS | ENABLE_W1TC => self.enable & MODELED_PINS_MASK,
            IN => [GPIO6, GPIO10, GPIO27]
                .into_iter()
                .filter_map(|pin| self.pin_level(pin).map(|level| (pin, level)))
                .fold(0, |word, (pin, level)| word | (u32::from(level) << pin)),
            FUNC6_OUT_SEL => self.func6_out_sel,
            FUNC10_OUT_SEL => self.func10_out_sel,
            FUNC27_OUT_SEL => self.func27_out_sel,
            FUNC46_IN_SEL => self.func46_in_sel,
            FUNC47_IN_SEL => self.func47_in_sel,
            _ => 0,
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let before_gpio27 = self.gpio27_output_selected().then(|| self.gpio27_level());
        match offset {
            OUT => self.out = value & MODELED_PINS_MASK,
            OUT_W1TS => self.out |= value & MODELED_PINS_MASK,
            OUT_W1TC => self.out &= !(value & MODELED_PINS_MASK),
            ENABLE => self.enable = value & MODELED_PINS_MASK,
            ENABLE_W1TS => self.enable |= value & MODELED_PINS_MASK,
            ENABLE_W1TC => self.enable &= !(value & MODELED_PINS_MASK),
            FUNC6_OUT_SEL => self.func6_out_sel = value,
            FUNC10_OUT_SEL => self.func10_out_sel = value,
            FUNC27_OUT_SEL => self.func27_out_sel = value,
            FUNC46_IN_SEL => self.func46_in_sel = value,
            FUNC47_IN_SEL => self.func47_in_sel = value,
            _ => return,
        }
        self.refresh_i2c_matrix_route();
        let after_gpio27 = self.gpio27_output_selected().then(|| self.gpio27_level());
        self.notify_gpio27_transition(before_gpio27, after_gpio27);
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
            IN | FUNC6_OUT_SEL | FUNC10_OUT_SEL | FUNC27_OUT_SEL | FUNC46_IN_SEL
            | FUNC47_IN_SEL => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn read_gpio_input_word(&self) -> u32 {
        self.read_word(IN)
    }

    fn read_gpio_input(&self, pin: u8) -> Option<bool> {
        self.pin_level(pin)
    }

    fn read_gpio_output(&self, pin: u8) -> Option<bool> {
        self.output_selector(pin)
            .map(|_| self.out & (1u32 << pin) != 0)
    }

    fn gpio_routing(&self, pin: u8) -> Option<GpioRouting> {
        let selector = self.output_selector(pin)?;
        if self.enable & (1u32 << pin) == 0 || !self.io_mux_selects_gpio(pin) {
            return Some(GpioRouting {
                mode: GpioMode::Input,
                func: None,
            });
        }
        let signal = selector & 0x1FF;
        Some(if signal == SIG_GPIO_OUT && selector & OE_SEL != 0 {
            GpioRouting {
                mode: GpioMode::Output,
                func: None,
            }
        } else {
            let func = match signal {
                SIG_I2CEXT0_SCL => Some("I2CEXT0_SCL".to_owned()),
                SIG_I2CEXT0_SDA => Some("I2CEXT0_SDA".to_owned()),
                SIG_RMT_OUT0 => Some("RMT_SIG_OUT0".to_owned()),
                SIG_RMT_OUT1 => Some("RMT_SIG_OUT1".to_owned()),
                _ => None,
            };
            GpioRouting {
                mode: GpioMode::Af,
                func,
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
            "gpio6_out": self.out & GPIO6_MASK != 0,
            "gpio10_out": self.out & GPIO10_MASK != 0,
            "gpio27_out": self.out & GPIO27_MASK != 0,
            "gpio27_rmt_level": self.rmt27_level,
            "enable": self.enable,
            "func6_out_sel": self.func6_out_sel,
            "func10_out_sel": self.func10_out_sel,
            "func27_out_sel": self.func27_out_sel,
            "func46_in_sel": self.func46_in_sel,
            "func47_in_sel": self.func47_in_sel,
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        for (key, mask) in [
            ("gpio6_out", GPIO6_MASK),
            ("gpio10_out", GPIO10_MASK),
            ("gpio27_out", GPIO27_MASK),
        ] {
            if let Some(value) = state.get(key).and_then(serde_json::Value::as_bool) {
                if value {
                    self.out |= mask;
                } else {
                    self.out &= !mask;
                }
            }
        }
        self.rmt27_level = state
            .get("gpio27_rmt_level")
            .and_then(serde_json::Value::as_bool);
        if let Some(value) = state.get("enable").and_then(serde_json::Value::as_u64) {
            self.enable = value as u32 & MODELED_PINS_MASK;
        }
        for (key, field) in [
            ("func6_out_sel", &mut self.func6_out_sel),
            ("func10_out_sel", &mut self.func10_out_sel),
            ("func27_out_sel", &mut self.func27_out_sel),
            ("func46_in_sel", &mut self.func46_in_sel),
            ("func47_in_sel", &mut self.func47_in_sel),
        ] {
            if let Some(value) = state.get(key).and_then(serde_json::Value::as_u64) {
                *field = value as u32;
            }
        }
        self.refresh_i2c_matrix_route();
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&(
            self.out,
            self.enable,
            self.func6_out_sel,
            self.func10_out_sel,
            self.func27_out_sel,
            self.rmt27_level,
            self.func46_in_sel,
            self.func47_in_sel,
            self.cycle,
        ))
        .expect("serialize ESP32-C5 GPIO state")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (out, enable, func6, func10, func27, rmt27_level, in46, in47, cycle): (
                u32,
                u32,
                u32,
                u32,
                u32,
                Option<bool>,
                u32,
                u32,
                u64,
            ) = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-C5 GPIO runtime snapshot decode: {error}"
                ))
            })?;
            self.out = out & MODELED_PINS_MASK;
            self.enable = enable & MODELED_PINS_MASK;
            self.func6_out_sel = func6;
            self.func10_out_sel = func10;
            self.func27_out_sel = func27;
            self.rmt27_level = rmt27_level;
            self.func46_in_sel = in46;
            self.func47_in_sel = in47;
            self.cycle = cycle;
            self.refresh_i2c_matrix_route();
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
        29
    }

    fn driver(&self, pin: u8) -> Option<PadDriver> {
        match pin {
            GPIO27 => Some(if self.gpio27_output_selected() {
                PadDriver::output(self.gpio27_level(), false)
            } else {
                PadDriver::released(Pull::None)
            }),
            GPIO6 | GPIO10 => Some(match self.i2c_pad_level(pin) {
                Some(level) => PadDriver::output(level, true),
                None => PadDriver::released(Pull::None),
            }),
            _ => None,
        }
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
    fn gpio27_uses_its_c5_selector_and_requires_both_muxes() {
        let mut gpio = Esp32c5Gpio::new();
        let mut iomux = super::super::io_mux::Esp32c5IoMux::new();
        gpio.set_io_mux_gpio27(iomux.gpio27_pad_control());
        let edges = Arc::new(Edges::default());
        gpio.add_observer(edges.clone());

        gpio.write_u32(ENABLE_W1TS, GPIO27_MASK).unwrap();
        gpio.write_u32(FUNC27_OUT_SEL, SIG_GPIO_OUT | OE_SEL)
            .unwrap();
        gpio.write_u32(OUT_W1TS, GPIO27_MASK).unwrap();
        assert_eq!(edges.0.lock().unwrap().len(), 1);

        // 0xB30 is GPIO_FUNC23_OUT_SEL_CFG on C5, not GPIO27.
        gpio.write_u32(0xB30, SIG_GPIO_OUT).unwrap();
        iomux.write_u32(0x6C, 2 << 12).unwrap();
        gpio.write_u32(OUT_W1TC, GPIO27_MASK).unwrap();
        assert_eq!(edges.0.lock().unwrap().len(), 1);
    }
}
