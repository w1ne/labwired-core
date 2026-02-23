// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Shared state for NVIC registers.
#[derive(Debug)]
pub struct NvicState {
    pub iser: [AtomicU32; 8],
    pub ispr: [AtomicU32; 8],
    pub iabr: [AtomicU32; 8],
    pub ipr: [AtomicU32; 240], // Priority registers (simplified)
}

impl Default for NvicState {
    fn default() -> Self {
        Self {
            iser: Default::default(),
            ispr: Default::default(),
            iabr: Default::default(),
            ipr: [0; 240].map(|_| AtomicU32::new(0)),
        }
    }
}

/// Nested Vectored Interrupt Controller (NVIC) mock.
#[derive(Debug, Clone)]
pub struct Nvic {
    pub state: Arc<NvicState>,
}

impl Nvic {
    pub fn new(state: Arc<NvicState>) -> Self {
        Self { state }
    }

    pub fn is_enabled(&self, irq: u32) -> bool {
        if irq < 16 {
            return true;
        }
        let idx = ((irq - 16) / 32) as usize;
        let bit = (irq - 16) % 32;
        if idx < 8 {
            (self.state.iser[idx].load(Ordering::SeqCst) & (1 << bit)) != 0
        } else {
            false
        }
    }

    pub fn acknowledge_interrupt(&self, irq: u32) {
        if irq >= 16 {
            let idx = ((irq - 16) / 32) as usize;
            let bit = (irq - 16) % 32;
            if idx < 8 {
                // Clear pending, set active
                self.state.ispr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
                self.state.iabr[idx].fetch_or(1 << bit, Ordering::SeqCst);
            }
        }
    }

    pub fn complete_interrupt(&self, irq: u32) {
        if irq >= 16 {
            let idx = ((irq - 16) / 32) as usize;
            let bit = (irq - 16) % 32;
            if idx < 8 {
                // Clear active
                self.state.iabr[idx].fetch_and(!(1 << bit), Ordering::SeqCst);
            }
        }
    }
}

impl Peripheral for Nvic {
    fn read(&self, offset: u64) -> SimResult<u8> {
        let reg_idx = (offset / 4) as usize;
        let byte_offset = (offset % 4) as usize;

        let val = if offset < 0x20 {
            // ISER0-7
            self.state.iser[reg_idx].load(Ordering::SeqCst)
        } else if (0x100..0x120).contains(&offset) {
            // ISPR0-7
            let real_idx = (offset - 0x100) / 4;
            self.state.ispr[real_idx as usize].load(Ordering::SeqCst)
        } else if (0x200..0x220).contains(&offset) {
            // IABR0-7
            let real_idx = (offset - 0x200) / 4;
            self.state.iabr[real_idx as usize].load(Ordering::SeqCst)
        } else if (0x300..0x6BC).contains(&offset) {
            // IPR0-239
            let real_idx = (offset - 0x300) / 4;
            self.state.ipr[real_idx as usize].load(Ordering::SeqCst)
        } else {
            0
        };

        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let reg_idx = (offset / 4) as usize;
        let byte_offset = (offset % 4) as usize;
        let mask = (value as u32) << (byte_offset * 8);

        if offset < 0x20 {
            // ISER: Writing 1 sets the enable bit
            self.state.iser[reg_idx].fetch_or(mask, Ordering::SeqCst);
        } else if (0x80..0xA0).contains(&offset) {
            // ICER: Writing 1 clears the enable bit
            let real_idx = reg_idx - 0x80 / 4;
            self.state.iser[real_idx].fetch_and(!mask, Ordering::SeqCst);
        } else if (0x100..0x120).contains(&offset) {
            // ISPR: Writing 1 sets the pending bit
            let real_idx = reg_idx - 0x100 / 4;
            self.state.ispr[real_idx].fetch_or(mask, Ordering::SeqCst);
        } else if (0x180..0x1A0).contains(&offset) {
            // ICPR: Writing 1 clears the pending bit
            let real_idx = reg_idx - 0x180 / 4;
            self.state.ispr[real_idx].fetch_and(!mask, Ordering::SeqCst);
        } else if (0x300..0x6BC).contains(&offset) {
            // IPR: Priority registers
            let real_idx = (offset - 0x300) / 4;
            let mut old_val = self.state.ipr[real_idx as usize].load(Ordering::SeqCst);
            loop {
                let mut new_val = old_val;
                let m = 0xFF << (byte_offset * 8);
                new_val &= !m;
                new_val |= mask;
                match self.state.ipr[real_idx as usize].compare_exchange_weak(
                    old_val,
                    new_val,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => old_val = actual,
                }
            }
        }

        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        let iser: Vec<u32> = self
            .state
            .iser
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        let ispr: Vec<u32> = self
            .state
            .ispr
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        serde_json::json!({
            "iser": iser,
            "ispr": ispr,
        })
    }
}
