// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! LPC21xx/LPC22xx Timer/Counter block used by the imported LPC2294 profile.
//!
//! The model implements the timer-clocked counter, prescaler, four match
//! registers, match interrupt flags, reset/stop actions and the level IRQ
//! output. Capture inputs, counter mode and external match pins are outside
//! this profile.

use crate::{Peripheral, PeripheralTickResult, SimResult};

const IR: u64 = 0x00;
const TCR: u64 = 0x04;
const TC: u64 = 0x08;
const PR: u64 = 0x0C;
const PC: u64 = 0x10;
const MCR: u64 = 0x14;
const MR0: u64 = 0x18;
const REGISTER_SIZE: u64 = 0x28;

const TCR_ENABLE: u32 = 1 << 0;
const TCR_RESET: u32 = 1 << 1;

/// LPC22xx timer/counter register bank for Timer0 or Timer1.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LpcTimer {
    ir: u32,
    tcr: u32,
    tc: u32,
    pr: u32,
    pc: u32,
    mcr: u32,
    mr: [u32; 4],
}

impl LpcTimer {
    pub const SIZE: u64 = REGISTER_SIZE;

    pub fn new() -> Self {
        Self {
            ir: 0,
            tcr: 0,
            tc: 0,
            pr: 0,
            pc: 0,
            mcr: 0,
            mr: [0; 4],
        }
    }

    fn read_register(&self, offset: u64) -> u32 {
        match offset {
            IR => self.ir,
            TCR => self.tcr,
            TC => self.tc,
            PR => self.pr,
            PC => self.pc,
            MCR => self.mcr,
            MR0..=0x24 if offset & 3 == 0 => self.mr[((offset - MR0) / 4) as usize],
            _ => 0,
        }
    }

    fn write_register(&mut self, offset: u64, value: u32) {
        match offset {
            IR => self.ir &= !(value & 0xF),
            TCR => {
                self.tcr = value & (TCR_ENABLE | TCR_RESET);
                if self.tcr & TCR_RESET != 0 {
                    self.tc = 0;
                    self.pc = 0;
                }
            }
            TC => self.tc = value,
            PR => self.pr = value,
            PC => self.pc = value,
            MCR => self.mcr = value & 0xFFF,
            MR0..=0x24 if offset & 3 == 0 => {
                self.mr[((offset - MR0) / 4) as usize] = value;
            }
            _ => {}
        }
    }

    fn write_byte(&mut self, offset: u64, value: u8) {
        let register = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        if register == IR {
            self.ir &= !(u32::from(value) << shift);
            return;
        }
        let mask = 0xFFu32 << shift;
        let merged = (self.read_register(register) & !mask) | (u32::from(value) << shift);
        self.write_register(register, merged);
    }

    fn advance(&mut self, cycles: u64) {
        if self.tcr & TCR_ENABLE == 0 || self.tcr & TCR_RESET != 0 {
            return;
        }

        // PR is the maximum PC value. The next PCLK after PC reaches PR
        // increments TC and clears PC, so PR=0 increments TC on every PCLK.
        for _ in 0..cycles {
            if self.pc >= self.pr {
                self.pc = 0;
                self.tc = self.tc.wrapping_add(1);
                self.on_timer_count();
                if self.tcr & TCR_ENABLE == 0 || self.tcr & TCR_RESET != 0 {
                    break;
                }
            } else {
                self.pc += 1;
            }
        }
    }

    fn on_timer_count(&mut self) {
        let mut reset = false;
        let mut stop = false;
        for channel in 0..4 {
            if self.tc != self.mr[channel] {
                continue;
            }
            let actions = (self.mcr >> (channel * 3)) & 0x7;
            if actions & 1 != 0 {
                self.ir |= 1 << channel;
            }
            reset |= actions & 2 != 0;
            stop |= actions & 4 != 0;
        }

        if reset {
            self.tc = 0;
            self.pc = 0;
        }
        if stop {
            self.tcr &= !TCR_ENABLE;
        }
    }

    fn irq_level(&self) -> bool {
        self.ir != 0
    }
}

impl Default for LpcTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Peripheral for LpcTimer {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let register = offset & !3;
        let shift = ((offset & 3) * 8) as u32;
        Ok((self.read_register(register) >> shift) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        self.write_byte(offset, value);
        Ok(())
    }

    fn read_u16(&self, offset: u64) -> SimResult<u16> {
        if offset & 1 == 0 {
            let register = offset & !3;
            let shift = ((offset & 3) * 8) as u32;
            return Ok((self.read_register(register) >> shift) as u16);
        }
        let lo = u16::from(self.read(offset)?);
        let hi = u16::from(self.read(offset + 1)?);
        Ok(lo | (hi << 8))
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        if offset & 3 == 0 {
            Ok(self.read_register(offset))
        } else {
            let mut value = 0;
            for byte in 0..4 {
                value |= u32::from(self.read(offset + byte)?) << (byte * 8);
            }
            Ok(value)
        }
    }

    fn write_u16(&mut self, offset: u64, value: u16) -> SimResult<()> {
        if offset & 1 == 0 && offset & 3 != 3 {
            let register = offset & !3;
            let shift = ((offset & 3) * 8) as u32;
            if register == IR {
                self.ir &= !(u32::from(value) << shift);
                return Ok(());
            }
            let mask = 0xFFFFu32 << shift;
            let merged = (self.read_register(register) & !mask) | (u32::from(value) << shift);
            self.write_register(register, merged);
            return Ok(());
        }
        self.write(offset, value as u8)?;
        self.write(offset + 1, (value >> 8) as u8)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        if offset & 3 == 0 {
            self.write_register(offset, value);
            return Ok(());
        }
        for (index, byte) in value.to_le_bytes().iter().enumerate() {
            self.write(offset + index as u64, *byte)?;
        }
        Ok(())
    }

    fn needs_legacy_walk(&self) -> bool {
        true
    }

    fn legacy_tick_active(&self) -> bool {
        self.tcr & TCR_ENABLE != 0 && self.tcr & TCR_RESET == 0
    }

    fn legacy_tick_dynamic(&self) -> bool {
        true
    }

    fn tick_elapsed(&mut self, cycles: u64) -> PeripheralTickResult {
        self.advance(cycles);
        // ARM7 samples the peripheral's live line through its registered
        // PL190 input. Do not also pend `irq` as an NVIC exception number.
        PeripheralTickResult::default()
    }

    fn irq_line_level(&self) -> Option<bool> {
        Some(self.irq_level())
    }

    fn on_node_reset(&mut self) {
        *self = Self::new();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let restored: Self = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("LPC timer snapshot decode: {error}"))
        })?;
        *self = restored;
        Ok(())
    }

    fn runtime_snapshot(&self) -> Vec<u8> {
        bincode::serialize(self).expect("bincode serialize LPC timer")
    }

    fn restore_runtime_snapshot(&mut self, bytes: &[u8]) -> SimResult<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        let restored: Self = bincode::deserialize(bytes).map_err(|error| {
            crate::SimulationError::NotImplemented(format!(
                "LPC timer runtime snapshot decode: {error}"
            ))
        })?;
        *self = restored;
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
