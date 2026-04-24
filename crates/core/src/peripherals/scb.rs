// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// System Control Block (SCB)
#[derive(Debug, serde::Serialize)]
pub struct Scb {
    pub cpuid: u32,
    pub icsr: u32,
    #[serde(skip)]
    pub vtor: Arc<AtomicU32>, // Shared with CPU
    pub aircr: u32,
    pub scr: u32,
    pub ccr: u32,
    pub shpr1: u32,
    pub shpr2: u32,
    pub shpr3: u32,
}

impl Scb {
    pub fn new(vtor: Arc<AtomicU32>) -> Self {
        Self {
            cpuid: 0x410F_C241, // Cortex-M4 r0p1
            icsr: 0,
            vtor,
            aircr: 0,
            scr: 0,
            ccr: 0,
            shpr1: 0,
            shpr2: 0,
            shpr3: 0,
        }
    }

    fn read_reg(&self, offset: u32) -> u32 {
        match offset {
            0x00 => self.cpuid,
            0x04 => self.icsr,
            0x08 => self.vtor.load(Ordering::Relaxed),
            0x0C => self.aircr,
            0x10 => self.scr,
            0x14 => self.ccr,
            0x18 => self.shpr1,
            0x1C => self.shpr2,
            0x20 => self.shpr3,
            _ => 0,
        }
    }

    fn write_reg(&mut self, offset: u32, value: u32) {
        match offset {
            0x04 => self.icsr = value, // Simplified
            0x08 => self.vtor.store(value, Ordering::Relaxed),
            0x0C => self.aircr = value,
            0x10 => self.scr = value,
            0x14 => self.ccr = value,
            0x18 => self.shpr1 = value,
            0x1C => self.shpr2 = value,
            0x20 => self.shpr3 = value,
            _ => {}
        }
    }
}

impl crate::Peripheral for Scb {
    fn read(&self, offset: u32) -> SimResult<u8> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let reg_val = self.read_reg(reg_offset);
        Ok(((reg_val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u32, value: u8) -> SimResult<()> {
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let mut reg_val = self.read_reg(reg_offset);

        let mask = 0xFF << (byte_offset * 8);
        reg_val &= !mask;
        reg_val |= (value as u32) << (byte_offset * 8);

        self.write_reg(reg_offset, reg_val);
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        // Inject VTOR value manually since we skip the Arc
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "vtor".to_string(),
                serde_json::Value::Number(self.vtor.load(Ordering::Relaxed).into()),
            );
        }
        value
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        // VTOR is an Arc<AtomicU32> shared with the CPU; we must not
        // replace the pointer, only its atomic contents. Other fields
        // are owned by this SCB — update them by name-lookup.
        let Some(obj) = state.as_object() else {
            return Ok(());
        };
        let u32_of = |k: &str| obj.get(k).and_then(|v| v.as_u64()).map(|v| v as u32);
        if let Some(v) = u32_of("cpuid") { self.cpuid = v; }
        if let Some(v) = u32_of("icsr") { self.icsr = v; }
        if let Some(v) = u32_of("vtor") { self.vtor.store(v, Ordering::Relaxed); }
        if let Some(v) = u32_of("aircr") { self.aircr = v; }
        if let Some(v) = u32_of("scr") { self.scr = v; }
        if let Some(v) = u32_of("ccr") { self.ccr = v; }
        if let Some(v) = u32_of("shpr1") { self.shpr1 = v; }
        if let Some(v) = u32_of("shpr2") { self.shpr2 = v; }
        if let Some(v) = u32_of("shpr3") { self.shpr3 = v; }
        Ok(())
    }
}
