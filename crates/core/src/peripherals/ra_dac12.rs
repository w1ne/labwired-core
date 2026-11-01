// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Renesas RA 12-bit D/A converter, DAC12 (two channels, one register block).
//!
//! Register layout from FSP `R7FA6M5BH.h` / `R7FA8M1AH.h` (`R_DAC_Type`,
//! 0x10C4 bytes, identical on both parts): `DADR0`/`DADR1` (u16) +0x00/+0x02,
//! `DACR` +0x04 (`DAE` bit 5, `DAOE0` bit 6, `DAOE1` bit 7), `DADPR` +0x05
//! (`DPSEL` bit 7), `DAADSCR` +0x06 (`DAADST` bit 7), `DAVREFCR` +0x07
//! (`REF` bits 2:0), `DAAMPCR` +0x08 (`DAAMP0` bit 6, `DAAMP1` bit 7), `DAPC`
//! +0x09 (`PUMPEN` bit 0), `DAASWCR` +0x1C (`DAASW0` bit 6, `DAASW1` bit 7),
//! `DAADUSR` +0x10C0. The block sits at 0x4017_1000 on the RA6M5 and at
//! 0x4033_3000 on the RA8M1. Behaviour from FSP `r_dac.c`, which Zephyr's
//! `dac_renesas_ra.c` drives: `R_DAC_Open` stores `DADPR`, `DAADSCR`,
//! `DAVREFCR` and `DAPC` with the channel's `DAOEn` clear, `R_DAC_Start`
//! sets `DAAMPn`/`DAASWn` around `DAOEn` = 1, and `R_DAC_Write` stores
//! `DADRn`.
//!
//! Modelled:
//! - every register above reads back what was stored (`DACR` bits 4:0 read
//!   1, its reset value 0x1F);
//! - `DADRn` keeps only its twelve data bits: bits 11:0 when `DADPR.DPSEL`
//!   = 0 (right-justified), bits 15:4 when `DPSEL` = 1 (left-justified); the
//!   other four bits read 0;
//! - channel n's output, [`RaDac12::output_mv`], is `code * vref_mv / 4096`
//!   while `DACR.DAOEn` = 1 and undriven (`None`) otherwise. The new code
//!   reaches the output as soon as `DADRn` is written;
//! - [`crate::Peripheral::analog_output_probe`] hands that output to an ADC
//!   channel the board wires to the DAn pin (an `analog_links` entry on the
//!   ADC), so the ADC converts what the DAC drives.
//!
//! Not modelled: the conversion time and amplifier stabilisation wait (the
//! output steps at once), the reference selection (`DAVREFCR.REF`) and charge
//! pump (`DAPC`), which are stored only (the output always scales to the
//! configured `vref_mv`), and D/A-A/D synchronous start (`DAADSCR`,
//! `DAADUSR` are stored only; the ADC is never held off).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::SimResult;

/// Register window, `sizeof(R_DAC_Type)`.
pub const WINDOW: usize = 0x10C4;
/// Reference used for [`RaDac12::output_mv`] when the chip YAML sets none
/// (the EK-RA6M5 / EK-RA8M1 AVCC0 = VREFH = 3.3 V).
pub const DEFAULT_VREF_MV: u16 = 3300;

const DADR0: usize = 0x00;
const DACR: usize = 0x04;
const DADPR: usize = 0x05;
const DACR_RESET: u8 = 0x1F;
const DPSEL: u8 = 1 << 7;
const DAOE: [u8; 2] = [1 << 6, 1 << 7];
/// [`RaDac12::live`] value for an undriven output.
const UNDRIVEN: u32 = u32::MAX;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RaDac12 {
    regs: Vec<u8>,
    vref_mv: u16,
    /// Each channel's [`RaDac12::output_mv`], kept current on every write
    /// and shared with the probes handed out. Not snapshotted: it is derived
    /// from `regs`, and a restore keeps the same cells so bound probes stay
    /// live.
    #[serde(skip, default = "undriven_cells")]
    live: Arc<[AtomicU32; 2]>,
}

fn undriven_cells() -> Arc<[AtomicU32; 2]> {
    Arc::new([AtomicU32::new(UNDRIVEN), AtomicU32::new(UNDRIVEN)])
}

impl RaDac12 {
    pub fn new(vref_mv: u16) -> Self {
        let mut regs = vec![0u8; WINDOW];
        regs[DACR] = DACR_RESET;
        Self {
            regs,
            vref_mv,
            live: undriven_cells(),
        }
    }

    fn publish(&self) {
        for (ch, cell) in self.live.iter().enumerate() {
            let v = self.output_mv(ch).map_or(UNDRIVEN, u32::from);
            cell.store(v, Ordering::Relaxed);
        }
    }

    fn dadr(&self, ch: usize) -> u16 {
        let off = DADR0 + 2 * ch;
        u16::from_le_bytes([self.regs[off], self.regs[off + 1]])
    }

    fn data_mask(&self) -> u16 {
        if self.regs[DADPR] & DPSEL != 0 {
            0xFFF0
        } else {
            0x0FFF
        }
    }

    /// The 12-bit conversion code channel `ch` holds, whatever the format.
    pub fn code(&self, ch: usize) -> u16 {
        let raw = self.dadr(ch);
        if self.regs[DADPR] & DPSEL != 0 {
            raw >> 4
        } else {
            raw & 0x0FFF
        }
    }

    /// The voltage on the DAn pin in millivolts, or `None` while `DAOEn` = 0.
    pub fn output_mv(&self, ch: usize) -> Option<u16> {
        if ch >= DAOE.len() || self.regs[DACR] & DAOE[ch] == 0 {
            return None;
        }
        Some((u32::from(self.code(ch)) * u32::from(self.vref_mv) / 4096) as u16)
    }
}

impl Default for RaDac12 {
    fn default() -> Self {
        Self::new(DEFAULT_VREF_MV)
    }
}

impl crate::Peripheral for RaDac12 {
    // Nothing here is timed: the output follows each write at once, so the
    // DAC never needs the per-cycle tick walk (and must not hold off idle
    // fast-forward by asking for it).
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn legacy_tick_dynamic(&self) -> bool {
        false
    }

    // `tick` is the trait's no-op and nothing leaves the walk (no IRQ, DMA or
    // event), so deleting the walk cannot change what firmware sees.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(self.regs.get(offset as usize).copied().unwrap_or(0))
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off >= WINDOW {
            return Ok(());
        }
        if off < DACR {
            // DADRn: only the data bits of the current format are kept.
            let mask = self.data_mask().to_le_bytes()[off % 2];
            self.regs[off] = value & mask;
        } else if off == DACR {
            self.regs[off] = value | DACR_RESET;
        } else {
            self.regs[off] = value;
        }
        self.publish();
        Ok(())
    }

    fn analog_output_probe(&self, channel: u8) -> Option<crate::AnalogProbe> {
        let ch = usize::from(channel);
        if ch >= DAOE.len() {
            return None;
        }
        let live = Arc::clone(&self.live);
        Some(Arc::new(move || {
            let v = live[ch].load(Ordering::Relaxed);
            (v != UNDRIVEN).then_some(v as u16)
        }))
    }

    fn on_node_reset(&mut self) {
        let live = Arc::clone(&self.live);
        *self = Self::new(self.vref_mv);
        self.live = live;
        self.publish();
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn restore(&mut self, state: serde_json::Value) -> SimResult<()> {
        let mut restored: RaDac12 = serde_json::from_value(state).map_err(|error| {
            crate::SimulationError::NotImplemented(format!("RaDac12 snapshot decode: {error}"))
        })?;
        restored.live = Arc::clone(&self.live);
        *self = restored;
        self.publish();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Peripheral;

    fn w16(d: &mut RaDac12, off: u64, v: u16) {
        let [lo, hi] = v.to_le_bytes();
        d.write(off, lo).unwrap();
        d.write(off + 1, hi).unwrap();
    }

    fn r16(d: &RaDac12, off: u64) -> u16 {
        u16::from_le_bytes([d.read(off).unwrap(), d.read(off + 1).unwrap()])
    }

    #[test]
    fn reset_values_and_dacr_low_bits_read_one() {
        let mut d = RaDac12::default();
        assert_eq!(d.read(DACR as u64).unwrap(), 0x1F);
        d.write(DACR as u64, 0x40).unwrap();
        assert_eq!(d.read(DACR as u64).unwrap(), 0x5F);
        assert_eq!(r16(&d, 0), 0);
        assert_eq!(d.read(0x1C).unwrap(), 0);
    }

    #[test]
    fn dadr_keeps_twelve_bits_in_either_format() {
        let mut d = RaDac12::default();
        w16(&mut d, 0, 0xFFFF);
        assert_eq!(r16(&d, 0), 0x0FFF);
        assert_eq!(d.code(0), 0x0FFF);
        d.write(DADPR as u64, DPSEL).unwrap();
        w16(&mut d, 2, 0xABCD);
        assert_eq!(r16(&d, 2), 0xABC0);
        assert_eq!(d.code(1), 0x0ABC);
    }

    #[test]
    fn output_follows_daoe_and_the_code() {
        let mut d = RaDac12::new(3300);
        w16(&mut d, 0, 0x0800);
        assert_eq!(d.output_mv(0), None, "DAOE0 = 0: the pin is not driven");
        d.write(DACR as u64, 0x40).unwrap();
        assert_eq!(d.output_mv(0), Some(1650));
        assert_eq!(d.output_mv(1), None);
        w16(&mut d, 0, 0x0FFF);
        assert_eq!(d.output_mv(0), Some(3299));
        d.write(DACR as u64, 0x00).unwrap();
        assert_eq!(d.output_mv(0), None);
    }

    #[test]
    fn probe_follows_later_writes_reset_and_restore() {
        let mut d = RaDac12::new(3300);
        let probe = d.analog_output_probe(0).unwrap();
        assert!(d.analog_output_probe(2).is_none());
        assert_eq!(probe(), None);
        let snap = d.snapshot();
        w16(&mut d, 0, 0x0800);
        d.write(DACR as u64, 0x40).unwrap();
        assert_eq!(probe(), Some(1650));
        d.restore(snap).unwrap();
        assert_eq!(probe(), None, "the restored state drives nothing");
        d.write(DACR as u64, 0x40).unwrap();
        w16(&mut d, 0, 0x0400);
        assert_eq!(probe(), Some(825));
        d.on_node_reset();
        assert_eq!(probe(), None);
    }

    #[test]
    fn snapshot_round_trips() {
        let mut d = RaDac12::default();
        w16(&mut d, 0, 0x0123);
        d.write(DACR as u64, 0x40).unwrap();
        let snap = d.snapshot();
        let mut e = RaDac12::default();
        e.restore(snap).unwrap();
        assert_eq!(e.output_mv(0), d.output_mv(0));
        assert_eq!(r16(&e, 0), 0x0123);
    }
}
