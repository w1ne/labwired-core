// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! megaAVR 0-series PORTMUX (ATmega4808/4809 data sheet DS40002173 §15;
//! offsets and field widths from the Microchip ATmega4809 ATDF): EVSYSROUTEA,
//! CCLROUTEA, USARTROUTEA, TWISPIROUTEA, TCAROUTEA and TCBROUTEA, every one
//! read/write and reset to 0.
//!
//! The registers are stored, and only TCAROUTEA moves a pad: it is published
//! as the remap cell ([`crate::Peripheral::remap_register`]) the TCA0
//! waveform-output routes read, so WO0..WO2 reach pins 0..2 of the port it
//! selects (0 = PORTA .. 5 = PORTF). The other routes are not applied: the
//! USART, TWI and SPI models are not pad-routed, and TCB outputs are refused.

use crate::{Peripheral, SimResult};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

const TCAROUTEA: usize = 4;
/// Writable bits per register (ATDF field masks).
const MASKS: [u8; 6] = [0x3F, 0x0F, 0xFF, 0x33, 0x07, 0x0F];

#[derive(Debug, Default)]
pub struct MegaAvrPortmux {
    regs: [u8; 6],
    /// TCAROUTEA, shared with the TCA0 pad routes.
    tca_route: Arc<AtomicU32>,
}

impl MegaAvrPortmux {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Peripheral for MegaAvrPortmux {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let i = offset as usize;
        if let Some(r) = self.regs.get_mut(i) {
            *r = value & MASKS[i];
            if i == TCAROUTEA {
                self.tca_route.store(u32::from(*r), Ordering::Relaxed);
            }
        }
        Ok(())
    }

    fn remap_register(&self) -> Option<Arc<AtomicU32>> {
        Some(self.tca_route.clone())
    }

    fn on_node_reset(&mut self) {
        self.regs = [0; 6];
        self.tca_route.store(0, Ordering::Relaxed);
    }

    /// A register bank: nothing moves on the walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tcaroutea_is_the_remap_cell() {
        let mut p = MegaAvrPortmux::new();
        let cell = p.remap_register().unwrap();
        p.write(4, 0xF9).unwrap();
        assert_eq!(p.read(4).unwrap(), 0x01, "TCA0 field is 3 bits");
        assert_eq!(cell.load(Ordering::Relaxed), 1, "PORTB");
        p.write(2, 0xC0).unwrap();
        assert_eq!(cell.load(Ordering::Relaxed), 1, "USARTROUTEA moves nothing");
        p.on_node_reset();
        assert_eq!(cell.load(Ordering::Relaxed), 0);
    }
}
