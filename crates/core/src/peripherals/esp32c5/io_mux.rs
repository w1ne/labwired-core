// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C5 IO_MUX model for GPIO27, the DevKitC RGB LED data pad.

use crate::{MmioAccessClass, Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const GPIO27: u64 = 0x6C;
const PAD_RESET: u32 = 0x1800; // FUN_DRV=2, MCU_SEL=1 (Function 1, GPIO)
const MCU_SEL_MASK: u32 = 0b111 << 12;

pub(crate) type PadControl = Arc<RwLock<u32>>;

const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c5/io_mux_gpio27.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C5 GPIO27 IO_MUX descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32c5IoMux {
    gpio27: PadControl,
}

impl Esp32c5IoMux {
    pub fn new() -> Self {
        Self {
            gpio27: Arc::new(RwLock::new(PAD_RESET)),
        }
    }

    pub(crate) fn pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio27)
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == GPIO27 {
            *self
                .gpio27
                .read()
                .expect("ESP32-C5 IO_MUX GPIO27 control poisoned")
        } else {
            0
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == GPIO27 {
            *self
                .gpio27
                .write()
                .expect("ESP32-C5 IO_MUX GPIO27 control poisoned") = value;
        }
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
        if word_offset == GPIO27 {
            let shift = (offset & 3) * 8;
            let mut word = self
                .gpio27
                .write()
                .expect("ESP32-C5 IO_MUX GPIO27 control poisoned");
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
        if offset & !3 == GPIO27 {
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
        serde_json::json!({ "gpio27": self.read_word(GPIO27) })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio27").and_then(serde_json::Value::as_u64) {
            self.write_word(GPIO27, value as u32);
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(io_mux_descriptor().clone())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.read_word(GPIO27)).expect("serialize C5 IO_MUX GPIO27")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let value: u32 = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-C5 IO_MUX snapshot decode: {error}"
                ))
            })?;
            self.write_word(GPIO27, value);
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
    fn gpio27_resets_to_function_one_and_tracks_selection() {
        let mut mux = Esp32c5IoMux::new();
        assert!(gpio_function_selected(mux.read_word(GPIO27)));

        mux.write_u32(GPIO27, 2 << 12).unwrap();
        assert!(!gpio_function_selected(mux.read_word(GPIO27)));
        mux.write_u32(GPIO27, 1 << 12).unwrap();
        assert!(gpio_function_selected(mux.read_word(GPIO27)));
    }
}
