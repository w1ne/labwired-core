// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 IO_MUX model for the DevKitC I2C and RGB LED pads.

use crate::{MmioAccessClass, Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const GPIO6: u64 = 0x18;
const GPIO10: u64 = 0x28;
const GPIO27: u64 = 0x6C;
const PAD_RESET: u32 = 0x1800; // FUN_DRV=2, MCU_SEL=1 (Function 1, GPIO)
const MCU_SEL_MASK: u32 = 0b111 << 12;

pub(crate) type PadControl = Arc<RwLock<u32>>;

const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c5/io_mux_i2c_rgb.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C5 GPIO27 IO_MUX descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32c5IoMux {
    gpio6: PadControl,
    gpio10: PadControl,
    gpio27: PadControl,
}

impl Esp32c5IoMux {
    pub fn new() -> Self {
        Self {
            gpio6: Arc::new(RwLock::new(PAD_RESET)),
            gpio10: Arc::new(RwLock::new(PAD_RESET)),
            gpio27: Arc::new(RwLock::new(PAD_RESET)),
        }
    }

    pub(crate) fn gpio6_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio6)
    }

    pub(crate) fn gpio10_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio10)
    }

    pub(crate) fn gpio27_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio27)
    }

    fn read_word(&self, offset: u64) -> u32 {
        let control = match offset {
            GPIO6 => &self.gpio6,
            GPIO10 => &self.gpio10,
            GPIO27 => &self.gpio27,
            _ => return 0,
        };
        *control
            .read()
            .expect("ESP32-C5 IO_MUX pad control poisoned")
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        let control = match offset {
            GPIO6 => &self.gpio6,
            GPIO10 => &self.gpio10,
            GPIO27 => &self.gpio27,
            _ => return,
        };
        *control
            .write()
            .expect("ESP32-C5 IO_MUX pad control poisoned") = value;
    }

    fn is_pad_register(offset: u64) -> bool {
        matches!(offset & !3, GPIO6 | GPIO10 | GPIO27)
    }
}

impl Default for Esp32c5IoMux {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32c5IoMux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        if Self::is_pad_register(word_offset) {
            let shift = (offset & 3) * 8;
            let control = match word_offset {
                GPIO6 => &self.gpio6,
                GPIO10 => &self.gpio10,
                GPIO27 => &self.gpio27,
                _ => unreachable!(),
            };
            let mut word = control
                .write()
                .expect("ESP32-C5 IO_MUX pad control poisoned");
            *word = (*word & !(0xFF << shift)) | (u32::from(value) << shift);
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
        if Self::is_pad_register(offset) {
            MmioAccessClass::SideEffectFree
        } else {
            MmioAccessClass::SideEffecting
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "gpio6": self.read_word(GPIO6),
            "gpio10": self.read_word(GPIO10),
            "gpio27": self.read_word(GPIO27),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        for (key, offset) in [("gpio6", GPIO6), ("gpio10", GPIO10), ("gpio27", GPIO27)] {
            if let Some(value) = state.get(key).and_then(serde_json::Value::as_u64) {
                self.write_word(offset, value as u32);
            }
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(io_mux_descriptor().clone())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&(
            self.read_word(GPIO6),
            self.read_word(GPIO10),
            self.read_word(GPIO27),
        ))
        .expect("serialize C5 IO_MUX pads")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (gpio6, gpio10, gpio27): (u32, u32, u32) =
                bincode::deserialize(bytes).map_err(|error| {
                    crate::SimulationError::NotImplemented(format!(
                        "ESP32-C5 IO_MUX snapshot decode: {error}"
                    ))
                })?;
            self.write_word(GPIO6, gpio6);
            self.write_word(GPIO10, gpio10);
            self.write_word(GPIO27, gpio27);
        }
        Ok(())
    }
}

pub(crate) fn gpio_function_selected(word: u32) -> bool {
    word & MCU_SEL_MASK == 1 << 12
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modeled_pads_reset_to_function_one_and_track_selection() {
        let mut mux = Esp32c5IoMux::new();
        for offset in [GPIO6, GPIO10, GPIO27] {
            assert!(gpio_function_selected(mux.read_word(offset)));
            mux.write_u32(offset, 2 << 12).unwrap();
            assert!(!gpio_function_selected(mux.read_word(offset)));
            mux.write_u32(offset, 1 << 12).unwrap();
            assert!(gpio_function_selected(mux.read_word(offset)));
        }
    }
}
