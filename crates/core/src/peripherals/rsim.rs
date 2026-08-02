// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NXP Kinetis RSIM (Radio System Integration Module) — minimal behavioural
//! model for the radio reference oscillator hand-off.
//!
//! The KW41Z has **no classic OSC peripheral**: the 32 MHz reference is the
//! radio crystal oscillator, enabled and observed through RSIM. The NXP
//! `BOARD_RfOscInit` board routine turns it on and then **spins until it reports
//! ready**:
//!
//! ```text
//!   RSIM->CONTROL = (RSIM->CONTROL & ~RF_OSC_EN_MASK) | RSIM_CONTROL_RF_OSC_EN(1);
//!   RSIM->RF_OSC_CTRL |= RADIO_EXT_OSC_OVRD_EN;
//!   while ((RSIM->CONTROL & RSIM_CONTROL_RF_OSC_READY_MASK) == 0) {}   // bit 24
//! ```
//!
//! A passive register bank never sets `RF_OSC_READY`, so boot hangs before the
//! MCG is ever touched. This model raises `RF_OSC_READY` as soon as the firmware
//! enables the oscillator (`RF_OSC_EN != 0`), modelling instantaneous crystal
//! start-up. Every other RSIM register is plain backing storage so the
//! surrounding writes (`RF_OSC_CTRL` override-enable, `ANA_TRIM`) read back and
//! never fault.
//!
//! Reset values are the public CMSIS-SVD values (`data/NXP/MKW41Z4.svd`).

use crate::{Peripheral, SimResult};
use std::any::Any;

/// Backing-store span: covers CONTROL (0x000) through ANA_TRIM (0x12C).
const WINDOW: usize = 0x200;

const CONTROL: usize = 0x000;
const RF_OSC_CTRL: usize = 0x124;
const ANA_TRIM: usize = 0x12C;

// RSIM_CONTROL fields.
const RF_OSC_EN_MASK: u32 = 0xF << 8; // CONTROL[11:8] — enable nibble
const RF_OSC_READY: u32 = 1 << 24; // CONTROL[24] — oscillator ready status
const RF_OSC_READY_OVRD_EN: u32 = 1 << 25; // CONTROL[25] — force-ready enable
const RF_OSC_READY_OVRD: u32 = 1 << 26; // CONTROL[26] — forced ready value

/// Behavioural RSIM. Only the RF-oscillator ready hand-off is modelled; the rest
/// is faithful storage.
#[derive(Debug)]
pub struct Rsim {
    data: Vec<u8>,
}

impl Default for Rsim {
    fn default() -> Self {
        Self::new()
    }
}

impl Rsim {
    pub fn new() -> Self {
        let mut s = Self {
            data: vec![0u8; WINDOW],
        };
        // SVD reset values.
        s.set32(CONTROL, 0x00C0_0002);
        s.set32(RF_OSC_CTRL, 0x0020_3806);
        s.set32(ANA_TRIM, 0x784B_0000);
        s
    }

    fn get32(&self, off: usize) -> u32 {
        let b = &self.data;
        (b[off] as u32)
            | ((b[off + 1] as u32) << 8)
            | ((b[off + 2] as u32) << 16)
            | ((b[off + 3] as u32) << 24)
    }

    fn set32(&mut self, off: usize, val: u32) {
        self.data[off] = val as u8;
        self.data[off + 1] = (val >> 8) as u8;
        self.data[off + 2] = (val >> 16) as u8;
        self.data[off + 3] = (val >> 24) as u8;
    }

    /// Reflect the enable/override bits into RF_OSC_READY, the way the silicon
    /// reports the crystal as stable. Modelled as instantaneous.
    fn refresh_control(&mut self) {
        let mut ctrl = self.get32(CONTROL);
        let forced_ready = ctrl & RF_OSC_READY_OVRD_EN != 0 && ctrl & RF_OSC_READY_OVRD != 0;
        if ctrl & RF_OSC_EN_MASK != 0 || forced_ready {
            ctrl |= RF_OSC_READY;
        } else {
            ctrl &= !RF_OSC_READY;
        }
        self.set32(CONTROL, ctrl);
    }
}

impl Peripheral for Rsim {
    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.data.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off >= WINDOW {
            return Ok(());
        }
        self.data[off] = value;
        if off < CONTROL + 4 {
            self.refresh_control();
        }
        Ok(())
    }

    fn read_u32(&self, offset: u64) -> SimResult<u32> {
        let off = offset as usize;
        if off + 3 < WINDOW {
            return Ok(self.get32(off));
        }
        Ok(0)
    }

    fn write_u32(&mut self, offset: u64, value: u32) -> SimResult<()> {
        let off = offset as usize;
        if off + 3 >= WINDOW {
            return Ok(());
        }
        self.set32(off, value);
        if off == CONTROL {
            self.refresh_control();
        }
        Ok(())
    }

    fn peek(&self, offset: u64) -> Option<u8> {
        self.data.get(offset as usize).copied()
    }

    /// Walk-free (kw41z): the RSIM is purely combinational — `RF_OSC_READY` is
    /// recomputed from the enable/override bits on every CONTROL write
    /// (`refresh_control`), every other register is plain storage, and there is
    /// NO `tick()`/`tick_elapsed()` override. The crystal start-up is modelled
    /// as instantaneous (no timed handshake), so the model has zero
    /// time-dependent state and the per-cycle walk can never change any
    /// observable output. Proven inert (`needs_legacy_walk() == false`) by the
    /// differential gate `crates/core/tests/kw41z_walk_free_differential.rs`.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({ "peripheral": "RSIM", "control": self.get32(CONTROL) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_control_reports_not_ready() {
        let rsim = Rsim::new();
        assert_eq!(rsim.read_u32(CONTROL as u64).unwrap(), 0x00C0_0002);
        assert_eq!(rsim.read_u32(CONTROL as u64).unwrap() & RF_OSC_READY, 0);
    }

    #[test]
    fn enabling_rf_osc_raises_ready_word_write() {
        // BOARD_RfOscInit does a 32-bit read-modify-write setting RF_OSC_EN.
        let mut rsim = Rsim::new();
        let ctrl = rsim.read_u32(CONTROL as u64).unwrap();
        rsim.write_u32(CONTROL as u64, (ctrl & !RF_OSC_EN_MASK) | (1 << 8))
            .unwrap();
        assert_ne!(
            rsim.read_u32(CONTROL as u64).unwrap() & RF_OSC_READY,
            0,
            "RF_OSC_READY must set once the oscillator is enabled"
        );
    }

    #[test]
    fn enabling_via_byte_store_raises_ready() {
        // A byte store into the enable nibble (CONTROL[11:8], byte 1) must also
        // settle the ready bit.
        let mut rsim = Rsim::new();
        rsim.write((CONTROL + 1) as u64, 0x01).unwrap(); // bit 8 = RF_OSC_EN[0]
        assert_ne!(rsim.read_u32(CONTROL as u64).unwrap() & RF_OSC_READY, 0);
    }

    #[test]
    fn override_ready_without_enable() {
        // Force-ready override path (RF_OSC_READY_OVRD_EN + RF_OSC_READY_OVRD).
        let mut rsim = Rsim::new();
        rsim.write_u32(
            CONTROL as u64,
            0x00C0_0002 | RF_OSC_READY_OVRD_EN | RF_OSC_READY_OVRD,
        )
        .unwrap();
        assert_ne!(rsim.read_u32(CONTROL as u64).unwrap() & RF_OSC_READY, 0);
    }

    #[test]
    fn other_registers_are_storage() {
        let mut rsim = Rsim::new();
        assert_eq!(rsim.read_u32(RF_OSC_CTRL as u64).unwrap(), 0x0020_3806);
        rsim.write_u32(RF_OSC_CTRL as u64, 0x0020_3806 | (1 << 13))
            .unwrap();
        assert_eq!(
            rsim.read_u32(RF_OSC_CTRL as u64).unwrap(),
            0x0020_3806 | (1 << 13)
        );
    }
}
