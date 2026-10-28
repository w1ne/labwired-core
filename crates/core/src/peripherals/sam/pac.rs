// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//! Opt-in SAMD51 PAC slice (DS60001507F 27): the peripheral access error flags,
//! which drivers clear before using a peripheral (codal's SAMDDAC clears
//! INTFLAGD.DAC). No access errors are modelled, so every flag reads 0 and a
//! write-1-to-clear is accepted; write protection (WRCTRL), the event output
//! and the PAC interrupt are refused.
use crate::{Peripheral, SimResult, SimulationError};
use std::any::Any;

#[derive(Debug, Default)]
pub struct Samd51Pac;

impl Samd51Pac {
    pub fn new() -> Self {
        Self
    }
    fn refuse<T>(what: &str) -> SimResult<T> {
        Err(SimulationError::NotImplemented(format!(
            "samd51 PAC slice: {what}"
        )))
    }
}

impl Peripheral for Samd51Pac {
    /// No `tick()` work: state moves on MMIO and the bus tick only, so the
    /// legacy walk can be deleted on a scheduler build.
    fn needs_legacy_walk(&self) -> bool {
        false
    }
    fn read(&self, offset: u64) -> SimResult<u8> {
        match offset {
            // WRCTRL, EVCTRL, INTENCLR/SET, INTFLAGAHB, INTFLAGA..D, STATUSA..D
            0x00..=0x0b | 0x10..=0x23 | 0x34..=0x43 => Ok(0),
            _ => Self::refuse(&format!("register {offset:#x}")),
        }
    }
    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            // INTFLAGAHB, INTFLAGA..D: write-1-to-clear of flags that never set.
            0x10..=0x23 => Ok(()),
            0x00..=0x0b if value == 0 => Ok(()),
            _ => Self::refuse("write protection, events or interrupts"),
        }
    }
    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            self.write(offset + i as u64, byte)?;
        }
        Ok(())
    }
    fn peek(&self, offset: u64) -> Option<u8> {
        self.read(offset).ok()
    }
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
