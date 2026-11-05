// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Narrow ESP32-C61 IO_MUX model for the GPIO8 output used by DevKitC's RGB LED.
//!
//! C61 puts IO_MUX GPIO8 at offset `0x20`. The word is shared with the C61
//! GPIO model so selecting a non-GPIO function really disconnects the modeled
//! GPIO output from the board pin.

use crate::{MmioAccessClass, Peripheral, SimResult};
use labwired_config::PeripheralDescriptor;
use std::sync::{Arc, OnceLock, RwLock};

const GPIO8: u64 = 0x20;
const GPIO8_PIN: usize = 8;
const PAD_COUNT: usize = 22;
const PAD_RESET: u32 = 0x1800; // FUN_DRV=2, MCU_SEL=1 (GPIO function)
const MCU_SEL_MASK: u32 = 0b111 << 12;

pub(crate) type PadControls = Arc<RwLock<[u32; PAD_COUNT]>>;

const IO_MUX_DESCRIPTOR_YAML: &str =
    include_str!("../../../../../configs/peripherals/esp32c61/io_mux_gpio8.yaml");

fn io_mux_descriptor() -> &'static PeripheralDescriptor {
    static DESCRIPTOR: OnceLock<PeripheralDescriptor> = OnceLock::new();
    DESCRIPTOR.get_or_init(|| {
        PeripheralDescriptor::from_yaml(IO_MUX_DESCRIPTOR_YAML)
            .expect("embedded ESP32-C61 GPIO8 IO_MUX descriptor is valid")
    })
}

#[derive(Debug)]
pub struct Esp32c61IoMux {
    pads: PadControls,
}

impl Esp32c61IoMux {
    pub fn new() -> Self {
        let mut pads = [0; PAD_COUNT];
        pads[GPIO8_PIN] = PAD_RESET;
        Self {
            pads: Arc::new(RwLock::new(pads)),
        }
    }

    pub(crate) fn pad_controls(&self) -> PadControls {
        Arc::clone(&self.pads)
    }

    fn read_word(&self, offset: u64) -> u32 {
        if offset == GPIO8 {
            self.pads
                .read()
                .expect("ESP32-C61 IO_MUX pad controls poisoned")[GPIO8_PIN]
        } else {
            0
        }
    }

    fn write_word(&mut self, offset: u64, value: u32) {
        if offset == GPIO8 {
            self.pads
                .write()
                .expect("ESP32-C61 IO_MUX pad controls poisoned")[GPIO8_PIN] = value;
        }
    }
}

impl Peripheral for Esp32c61IoMux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let word = self.read_word(offset & !3);
        let shift = (offset & 3) * 8;
        Ok(((word >> shift) & 0xFF) as u8)
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let word_offset = offset & !3;
        if word_offset == GPIO8 {
            let shift = (offset & 3) * 8;
            let mut pads = self
                .pads
                .write()
                .expect("ESP32-C61 IO_MUX pad controls poisoned");
            let word = &mut pads[GPIO8_PIN];
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
        match offset & !3 {
            GPIO8 => MmioAccessClass::SideEffectFree,
            _ => MmioAccessClass::SideEffecting,
        }
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "gpio8": self.read_word(GPIO8) })
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        if let Some(value) = state.get("gpio8").and_then(serde_json::Value::as_u64) {
            self.write_word(GPIO8, value as u32);
        }
        Ok(())
    }

    fn peripheral_descriptor(&self) -> Option<PeripheralDescriptor> {
        Some(io_mux_descriptor().clone())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(&self.read_word(GPIO8)).expect("serialize C61 IO_MUX GPIO8")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if !bytes.is_empty() {
            let value: u32 = bincode::deserialize(bytes).map_err(|error| {
                crate::SimulationError::NotImplemented(format!(
                    "ESP32-C61 IO_MUX snapshot decode: {error}"
                ))
            })?;
            self.write_word(GPIO8, value);
        }
        Ok(())
    }
}

pub(crate) fn gpio_function_selected(word: u32) -> bool {
    word & MCU_SEL_MASK == 1 << 12
}
