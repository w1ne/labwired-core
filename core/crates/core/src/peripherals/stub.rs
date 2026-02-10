// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::collections::HashMap;

/// A simple stub peripheral that returns fixed values on read.
#[derive(Debug, serde::Serialize)]
pub struct StubPeripheral {
    pub values: HashMap<u64, u32>, // mapping offset to value
    pub default_val: u32,
}

impl StubPeripheral {
    pub fn new(default_val: u32) -> Self {
        Self {
            values: HashMap::new(),
            default_val,
        }
    }
}

impl crate::Peripheral for StubPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Simple byte mapping
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let val = self
            .values
            .get(&reg_offset)
            .cloned()
            .unwrap_or(self.default_val);
        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Ignores writes for now
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}
