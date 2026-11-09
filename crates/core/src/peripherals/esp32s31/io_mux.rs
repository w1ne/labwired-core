// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S31 IO_MUX pad controls for GPIO7 and GPIO8.

use crate::{MmioAccessClass, Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const GPIO7: u64 = 0x1C;
const GPIO8: u64 = 0x20;
const PAD_RESET: u32 = 0x1800; // FUN_DRV=2, MCU_SEL=1.

pub(crate) type PadControl = Arc<RwLock<u32>>;

const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32s31/io_mux.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-S31 IO_MUX descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32s31IoMux {
    gpio7: PadControl,
    gpio8: PadControl,
}

impl Esp32s31IoMux {
    pub fn new() -> Self {
        Self {
            gpio7: Arc::new(RwLock::new(PAD_RESET)),
            gpio8: Arc::new(RwLock::new(PAD_RESET)),
        }
    }

    pub(crate) fn gpio7_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio7)
    }

    pub(crate) fn gpio8_pad_control(&self) -> PadControl {
        Arc::clone(&self.gpio8)
    }

    fn read_register(&self, offset: u64) -> Option<u32> {
        let control = match offset & !3 {
            GPIO7 => &self.gpio7,
            GPIO8 => &self.gpio8,
            _ => return None,
        };
        Some(
            *control
                .read()
                .expect("ESP32-S31 IO_MUX pad control poisoned"),
        )
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        let control = match offset & !3 {
            GPIO7 => &self.gpio7,
            GPIO8 => &self.gpio8,
            _ => return,
        };
        *control
            .write()
            .expect("ESP32-S31 IO_MUX pad control poisoned") = value;
    }
}

impl Default for Esp32s31IoMux {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for Esp32s31IoMux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_register(offset).unwrap_or(0);
        Ok(((word >> ((offset & 3) * 8)) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        let shift = (offset & 3) * 8;
        if let Some(word) = self.read_register(word_offset) {
            let byte_mask = 0xFFu32 << shift;
            self.write_register(
                word_offset,
                (word & !byte_mask) | (u32::from(value) << shift),
            );
        }
        Ok(())
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_register(offset, value);
        } else {
            for byte in 0..4 {
                self.write(offset + byte, ((value >> (byte * 8)) & 0xFF) as u8)?;
            }
        }
        Ok(())
    }

    fn mmio_access_class(&self, offset: u64) -> MmioAccessClass {
        if matches!(offset & !3, GPIO7 | GPIO8) {
            MmioAccessClass::SideEffecting
        } else {
            MmioAccessClass::SideEffectFree
        }
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "gpio7": *self.gpio7.read().expect("ESP32-S31 IO_MUX GPIO7 poisoned"),
            "gpio8": *self.gpio8.read().expect("ESP32-S31 IO_MUX GPIO8 poisoned"),
        })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio7").and_then(serde_json::Value::as_u64) {
            *self.gpio7.write().expect("ESP32-S31 IO_MUX GPIO7 poisoned") = value as u32;
        }
        if let Some(value) = state.get("gpio8").and_then(serde_json::Value::as_u64) {
            *self.gpio8.write().expect("ESP32-S31 IO_MUX GPIO8 poisoned") = value as u32;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        let gpio7 = *self.gpio7.read().expect("ESP32-S31 IO_MUX GPIO7 poisoned");
        let gpio8 = *self.gpio8.read().expect("ESP32-S31 IO_MUX GPIO8 poisoned");
        bincode::serialize(&(gpio7, gpio8)).expect("serialize ESP32-S31 IO_MUX state")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let (gpio7, gpio8): (u32, u32) = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-S31 IO_MUX runtime snapshot decode: {error}"
                ))
            })?;
            *self.gpio7.write().expect("ESP32-S31 IO_MUX GPIO7 poisoned") = gpio7;
            *self.gpio8.write().expect("ESP32-S31 IO_MUX GPIO8 poisoned") = gpio8;
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
