// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Renesas RA SYSTEM clock (HOCO / OSCSF) — behavioural model.
//!
//! RA4M1 `R_SYSTEM` (R01UH0887 / FSP `R7FA4M1AB.h`): HOCOCR @ 0x36 (HCSTP bit0;
//! 0 = operating, 1 = stopped), OSCSF @ 0x3C (HOCOSF bit0). Firmware enables
//! HOCO then spins on OSCSF.HOCOSF; this model settles instantly like
//! [`crate::peripherals::mcg::Mcg`].
//!
//! RA8M1 (`ra8_sysc`, FSP `R7FA8M1AH.h` `R_SYSTEM_Type`, 0xD80 bytes): see
//! [`Ra8Sysc`] — a full SYSTEM register file with the oscillator stable
//! flags and the peripheral-clock switch handshake derived from it.

use crate::{Peripheral, SimResult};
use std::any::Any;

const HOCOCR: u64 = 0x36;
const OSCSF: u64 = 0x3C;

const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

/// Renesas RA SYSTEM block — HOCO enable + OSCSF ready flag.
#[derive(Debug)]
pub struct RaSysc {
    /// HOCOCR: bit0 HCSTP (0 = HOCO operating).
    hococr: u8,
    /// OSCSF: bit0 HOCOSF (1 = HOCO stable). Read-only to firmware.
    oscsf: u8,
}

impl Default for RaSysc {
    fn default() -> Self {
        Self::new()
    }
}

impl RaSysc {
    pub fn new() -> Self {
        // SVD/CMSIS: HOCOCR reset 0 (operating). HOCOSF is typically already
        // set when OFS1 leaves HOCO enabled out of reset.
        let mut s = Self {
            hococr: 0,
            oscsf: HOCOSF,
        };
        s.recompute_oscsf();
        s
    }

    fn recompute_oscsf(&mut self) {
        if self.hococr & HCSTP == 0 {
            self.oscsf |= HOCOSF;
        } else {
            self.oscsf &= !HOCOSF;
        }
    }
}

impl Peripheral for RaSysc {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        Ok(match offset {
            HOCOCR => self.hococr,
            OSCSF => self.oscsf,
            _ => 0,
        })
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        match offset {
            HOCOCR => {
                self.hococr = value;
                self.recompute_oscsf();
            }
            OSCSF => {} // read-only status
            _ => {}
        }
        Ok(())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "peripheral": "ra_sysc",
            "hococr": self.hococr,
            "oscsf": self.oscsf,
        })
    }
}

/// Renesas RA8 SYSTEM block (`ra8_sysc`): every register in
/// `R_SYSTEM_Type` (FSP `R7FA8M1AH.h`, 0xD80 bytes) is stored and reads
/// back, with two derived behaviours FSP's `bsp_clock_init` /
/// `bsp_peripheral_clock_set` poll on:
///
/// - **OSCSF** (+0x3C, read-only) is computed from the oscillator stop
///   bits: HOCOSF (bit0) = !HOCOCR.HCSTP (+0x36), MOSCSF (bit3) =
///   !MOSCCR.MOSTP (+0x32), PLLSF (bit5) = !PLLCR.PLLSTP (+0x2A), PLL2SF
///   (bit6) = !PLL2CR.PLL2STP (+0x4A). Every oscillator settles at once:
///   no stabilisation time and no MOSCWTCR/HOCOWTCR wait is modelled.
/// - **xxxCKCR** peripheral-clock switches (SCI +0x55, SPI +0x57, ADC +0x5B,
///   GPT +0x5D, LCD +0x5F, USB +0x74, OCTA +0x75, CANFD +0x76, USB60 +0x77,
///   I3C +0x78): CKSRDY (bit7) reads back CKSREQ (bit6) — FSP sets CKSREQ,
///   waits for CKSRDY = 1, writes source + divider, clears CKSREQ, waits
///   for CKSRDY = 0. The selected sources are stored only; no peripheral
///   rate is derived from them.
///
/// Reset values: the oscillator/PLL stop bits reset to "stopped" (MOSCCR,
/// SOSCCR, PLLCR, PLL2CR = 0x01) and SCKSCR to MOCO (0x01), the RA-family
/// pattern FSP relies on. HOCOCR resets to 0 (operating) as for `ra_sysc`:
/// the EK-RA8M1's OFS1 decides that on silicon. Everything else resets to
/// 0. These values are not cross-checked against the RA8M1 hardware manual
/// (R01UH0994); FSP's start-up code does not read them back beyond the
/// stop bits.
#[derive(Debug)]
pub struct Ra8Sysc {
    regs: Box<[u8; RA8_SYSTEM_SIZE]>,
}

const RA8_SYSTEM_SIZE: usize = 0xD80;
const RA8_SCKSCR: usize = 0x26;
const RA8_PLLCR: usize = 0x2A;
const RA8_MOSCCR: usize = 0x32;
const RA8_HOCOCR: usize = 0x36;
const RA8_OSCSF: usize = 0x3C;
const RA8_PLL2CR: usize = 0x4A;
const RA8_SOSCCR: usize = 0xC00;
const RA8_MOSCSF: u8 = 1 << 3;
const RA8_PLLSF: u8 = 1 << 5;
const RA8_PLL2SF: u8 = 1 << 6;
/// Peripheral clock control registers with the CKSREQ/CKSRDY handshake.
const RA8_CKCR: [usize; 10] = [0x55, 0x57, 0x5B, 0x5D, 0x5F, 0x74, 0x75, 0x76, 0x77, 0x78];
const CKSREQ: u8 = 1 << 6;
const CKSRDY: u8 = 1 << 7;

impl Default for Ra8Sysc {
    fn default() -> Self {
        Self::new()
    }
}

impl Ra8Sysc {
    pub fn new() -> Self {
        let mut regs = Box::new([0u8; RA8_SYSTEM_SIZE]);
        for stopped in [RA8_MOSCCR, RA8_PLLCR, RA8_PLL2CR, RA8_SOSCCR] {
            regs[stopped] = 0x01;
        }
        regs[RA8_SCKSCR] = 0x01; // MOCO
        Self { regs }
    }

    fn oscsf(&self) -> u8 {
        let running = |off: usize| self.regs[off] & 1 == 0;
        let mut sf = 0;
        if running(RA8_HOCOCR) {
            sf |= HOCOSF;
        }
        if running(RA8_MOSCCR) {
            sf |= RA8_MOSCSF;
        }
        if running(RA8_PLLCR) {
            sf |= RA8_PLLSF;
        }
        if running(RA8_PLL2CR) {
            sf |= RA8_PLL2SF;
        }
        sf
    }
}

impl Peripheral for Ra8Sysc {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let off = offset as usize;
        if off >= RA8_SYSTEM_SIZE {
            return Ok(0);
        }
        if off == RA8_OSCSF {
            return Ok(self.oscsf());
        }
        let v = self.regs[off];
        if RA8_CKCR.contains(&off) {
            return Ok((v & !CKSRDY) | if v & CKSREQ != 0 { CKSRDY } else { 0 });
        }
        Ok(v)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off < RA8_SYSTEM_SIZE && off != RA8_OSCSF {
            self.regs[off] = value;
        }
        Ok(())
    }

    fn snapshot(&self) -> serde_json::Value {
        let nonzero: serde_json::Map<String, serde_json::Value> = self
            .regs
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(i, v)| (format!("{i:#05x}"), serde_json::json!(v)))
            .collect();
        serde_json::json!({
            "peripheral": "ra8_sysc",
            "oscsf": self.oscsf(),
            "regs": nonzero,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hoco_enable_sets_oscsf_hocosf() {
        let mut sys = RaSysc::new();
        // Stop HOCO — HOCOSF must clear.
        sys.write(HOCOCR, HCSTP).unwrap();
        assert_eq!(sys.read(HOCOCR).unwrap() & HCSTP, HCSTP);
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, 0);
        // Enable (HCSTP=0) — HOCOSF settles immediately (no spin).
        sys.write(HOCOCR, 0).unwrap();
        assert_eq!(sys.read(HOCOCR).unwrap() & HCSTP, 0);
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, HOCOSF);
    }

    #[test]
    fn ra8_sci_clock_switch_completes_the_fsp_handshake() {
        const SCICKDIVCR: u64 = 0x54;
        const SCICKCR: u64 = 0x55;
        let mut sys = Ra8Sysc::new();
        // bsp_peripheral_clock_set: request the stop, wait for CKSRDY.
        sys.write(SCICKCR, CKSREQ).unwrap();
        assert_eq!(sys.read(SCICKCR).unwrap() & CKSRDY, CKSRDY);
        // Divider /4 (2), source PLL1P (5), REQ|RDY written back.
        sys.write(SCICKDIVCR, 2).unwrap();
        sys.write(SCICKCR, 5 | CKSREQ | CKSRDY).unwrap();
        // Release the request; CKSRDY drops and the source reads back.
        sys.write(SCICKCR, 5).unwrap();
        assert_eq!(sys.read(SCICKCR).unwrap(), 5);
        assert_eq!(sys.read(SCICKDIVCR).unwrap(), 2);
    }

    #[test]
    fn ra8_every_peripheral_clock_switch_has_the_handshake() {
        let mut sys = Ra8Sysc::new();
        for off in RA8_CKCR {
            sys.write(off as u64, CKSREQ | 3).unwrap();
            assert_eq!(
                sys.read(off as u64).unwrap(),
                CKSREQ | CKSRDY | 3,
                "{off:#x}"
            );
            sys.write(off as u64, 3).unwrap();
            assert_eq!(sys.read(off as u64).unwrap(), 3, "{off:#x}");
        }
        // A non-CKCR byte (SCICKDIVCR) never grows a CKSRDY bit.
        sys.write(0x54, CKSREQ).unwrap();
        assert_eq!(sys.read(0x54).unwrap(), CKSREQ);
    }

    #[test]
    fn ra8_oscsf_follows_the_oscillator_stop_bits() {
        let mut sys = Ra8Sysc::new();
        // Reset: HOCO running, MOSC/PLL/PLL2 stopped, MOCO selected.
        assert_eq!(sys.read(RA8_OSCSF as u64).unwrap(), HOCOSF);
        assert_eq!(sys.read(RA8_SCKSCR as u64).unwrap(), 1);
        // bsp_clock_init: MOSCCR = 0, wait MOSCSF; PLLCR = 0, wait PLLSF.
        sys.write(RA8_MOSCCR as u64, 0).unwrap();
        assert_eq!(sys.read(RA8_OSCSF as u64).unwrap() & RA8_MOSCSF, RA8_MOSCSF);
        sys.write(RA8_PLLCR as u64, 0).unwrap();
        sys.write(RA8_PLL2CR as u64, 0).unwrap();
        assert_eq!(
            sys.read(RA8_OSCSF as u64).unwrap(),
            HOCOSF | RA8_MOSCSF | RA8_PLLSF | RA8_PLL2SF
        );
        // Stopping HOCO and the PLL clears their flags; OSCSF is read-only.
        sys.write(RA8_HOCOCR as u64, 1).unwrap();
        sys.write(RA8_PLLCR as u64, 1).unwrap();
        sys.write(RA8_OSCSF as u64, 0xFF).unwrap();
        assert_eq!(sys.read(RA8_OSCSF as u64).unwrap(), RA8_MOSCSF | RA8_PLL2SF);
    }

    #[test]
    fn ra8_system_registers_read_back() {
        let mut sys = Ra8Sysc::new();
        // SCKDIVCR (32-bit at +0x20), PLLCCR (16-bit at +0x28), PRCR (+0x3FA).
        for (off, v) in [
            (0x20u64, 0x12u8),
            (0x23, 0x34),
            (0x28, 0x5F),
            (0x29, 0x01),
            (0x3FA, 0x0B),
        ] {
            sys.write(off, v).unwrap();
            assert_eq!(sys.read(off).unwrap(), v, "{off:#x}");
        }
        // Past the end of R_SYSTEM_Type: reads 0, writes dropped.
        sys.write(0xD80, 0xAA).unwrap();
        assert_eq!(sys.read(0xD80).unwrap(), 0);
    }

    #[test]
    fn ra4_and_ra6_sysc_have_no_sci_clock() {
        let mut sys = RaSysc::new();
        sys.write(0x55, CKSREQ).unwrap();
        assert_eq!(sys.read(0x55).unwrap(), 0);
    }

    #[test]
    fn oscsf_is_read_only() {
        let mut sys = RaSysc::new();
        sys.write(HOCOCR, HCSTP).unwrap();
        sys.write(OSCSF, 0xFF).unwrap();
        assert_eq!(sys.read(OSCSF).unwrap() & HOCOSF, 0);
    }
}
