// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-P4 IO_MUX GPIO8 pad function-select register.

use crate::{MmioAccessClass, Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const GPIO8: u64 = 0x24;
const PAD_RESET: u32 = 0x1800; // FUN_DRV=2, MCU_SEL=1 (GPIO8).

pub(crate) type PadControl = Arc<RwLock<u32>>;

const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32p4/io_mux.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-P4 IO_MUX descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32p4IoMux {
    gpio8: PadControl,
}

impl Esp32p4IoMux {
    pub fn new() -> Self {
        Self {
            gpio8: Arc::new(RwLock::new(PAD_RESET)),
        }
    }

    pub(crate) fn gpio8_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio8)
    }

    fn is_gpio8_register(offset: u64) -> bool {
        offset & !3 == GPIO8
    }
}

impl Default for Esp32p4IoMux {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32p4IoMux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = if Self::is_gpio8_register(offset) {
            *self
                .gpio8
                .read()
                .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned")
        } else {
            0
        };
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        if Self::is_gpio8_register(offset) {
            let shift = (offset & 3) * 8;
            let mut word = self
                .gpio8
                .write()
                .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned");
            *word = (*word & !(0xFF << shift)) | (u32::from(value) << shift);
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 && Self::is_gpio8_register(offset) {
            *self
                .gpio8
                .write()
                .expect("ESP32-P4 IO_MUX GPIO8 pad control poisoned") = value;
        } else if offset & 3 != 0 {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xFF) as u8)?;
            }
        }
        Ok(())
    }

    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        if Self::is_gpio8_register(offset) {
            MmioAccessClass::SideEffecting
        } else {
            MmioAccessClass::SideEffectFree
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "gpio8": *self.gpio8.read().expect("ESP32-P4 IO_MUX pad control poisoned"),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio8").and_then(serde_json::Value::as_u64) {
            *self
                .gpio8
                .write()
                .expect("ESP32-P4 IO_MUX pad control poisoned") = value as u32;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(
            &*self
                .gpio8
                .read()
                .expect("ESP32-P4 IO_MUX pad control poisoned"),
        )
        .expect("serialize ESP32-P4 IO_MUX GPIO8 pad")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let value: u32 = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-P4 IO_MUX runtime snapshot decode: {error}"
                ))
            })?;
            *self
                .gpio8
                .write()
                .expect("ESP32-P4 IO_MUX pad control poisoned") = value;
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(io_mux_descriptor().clone())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
