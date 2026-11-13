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
//! RA8M1 (`ra8_sysc`, FSP `R7FA8M1AH.h` `R_SYSTEM_Type`, 0xD80 bytes) and
//! RA6M5 (`ra6_sysc`, FSP `R7FA6M5BH.h`, 0x700 bytes): see [`RaSyscRegs`] —
//! a full SYSTEM register file with the oscillator stable flags and the
//! peripheral-clock switch handshake derived from it.

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

/// Renesas RA SYSTEM block as a full register file (`ra8_sysc`,
/// `ra6_sysc`): every register in the part's `R_SYSTEM_Type` is stored and
/// reads back, with two derived behaviours FSP's `bsp_clock_init` /
/// `bsp_peripheral_clock_set` poll on:
///
/// - **OSCSF** (+0x3C, read-only) is computed from the oscillator stop
///   bits: HOCOSF (bit0) = !HOCOCR.HCSTP (+0x36), MOSCSF (bit3) =
///   !MOSCCR.MOSTP (+0x32), PLLSF (bit5) = !PLLCR.PLLSTP (+0x2A), PLL2SF
///   (bit6) = !PLL2CR.PLL2STP (+0x4A). These offsets are common to the RA6
///   and RA8 headers. Every oscillator settles at once: no stabilisation
///   time and no MOSCWTCR/HOCOWTCR wait is modelled.
/// - **xxxCKCR** peripheral-clock switches listed by the layout: CKSRDY
///   (bit7) reads back CKSREQ (bit6) — FSP sets CKSREQ, waits for
///   CKSRDY = 1, writes source + divider, clears CKSREQ, waits for
///   CKSRDY = 0. The selected sources are stored only; no peripheral rate is
///   derived from them.
///
/// Reset values: the oscillator/PLL stop bits listed by the layout reset to
/// "stopped" (0x01) and SCKSCR to MOCO (0x01), the RA-family pattern FSP
/// relies on. HOCOCR resets to 0 (operating) as for `ra_sysc`: the board's
/// OFS1 decides that on silicon. Everything else resets to 0. These values
/// are not cross-checked against the hardware manuals; FSP's start-up code
/// does not read them back beyond the stop bits.
#[derive(Debug)]
pub struct RaSyscRegs {
    layout: &'static RaSyscLayout,
    regs: Box<[u8]>,
}

/// Per-family `R_SYSTEM_Type` facts for [`RaSyscRegs`].
#[derive(Debug)]
pub struct RaSyscLayout {
    /// Factory type name, also the snapshot tag.
    pub name: &'static str,
    /// `sizeof(R_SYSTEM_Type)`; offsets past it read 0, writes dropped.
    pub size: usize,
    /// Registers whose bit0 is a stop bit that resets to "stopped".
    pub stopped_at_reset: &'static [usize],
    /// Peripheral clock control registers with the CKSREQ/CKSRDY handshake.
    pub ckcr: &'static [usize],
}

/// RA8M1 (FSP `R7FA8M1AH.h`, 0xD80 bytes): MOSCCR/PLLCR/PLL2CR and SOSCCR
/// (+0xC00); CKCR for SCI +0x55, SPI +0x57, ADC +0x5B, GPT +0x5D, LCD
/// +0x5F, USB +0x74, OCTA +0x75, CANFD +0x76, USB60 +0x77, I3C +0x78.
pub const RA8_SYSC: RaSyscLayout = RaSyscLayout {
    name: "ra8_sysc",
    size: 0xD80,
    stopped_at_reset: &[RA_MOSCCR, RA_PLLCR, RA_PLL2CR, 0xC00],
    ckcr: &[0x55, 0x57, 0x5B, 0x5D, 0x5F, 0x74, 0x75, 0x76, 0x77, 0x78],
};

/// RA6M5 (FSP `R7FA6M5BH.h`, 0x700 bytes: VBTBKR[512] at +0x500 is the
/// last member): MOSCCR/PLLCR/PLL2CR; CKCR for the clocks the RA6M5 has a
/// switch handshake on, USB +0x74, OCTA +0x75 and CANFD +0x76. SOSCCR
/// (+0x480) is stored with no reset override: the sub-clock is in the VBATT
/// domain and its reset state is not modelled.
pub const RA6_SYSC: RaSyscLayout = RaSyscLayout {
    name: "ra6_sysc",
    size: 0x700,
    stopped_at_reset: &[RA_MOSCCR, RA_PLLCR, RA_PLL2CR],
    ckcr: &[0x74, 0x75, 0x76],
};

const RA_SCKSCR: usize = 0x26;
const RA_PLLCR: usize = 0x2A;
const RA_MOSCCR: usize = 0x32;
const RA_HOCOCR: usize = 0x36;
const RA_OSCSF: usize = 0x3C;
const RA_PLL2CR: usize = 0x4A;
const RA_MOSCSF: u8 = 1 << 3;
const RA_PLLSF: u8 = 1 << 5;
const RA_PLL2SF: u8 = 1 << 6;
const CKSREQ: u8 = 1 << 6;
const CKSRDY: u8 = 1 << 7;

impl RaSyscRegs {
    pub fn new(layout: &'static RaSyscLayout) -> Self {
        let mut regs = vec![0u8; layout.size].into_boxed_slice();
        for &stopped in layout.stopped_at_reset {
            regs[stopped] = 0x01;
        }
        regs[RA_SCKSCR] = 0x01; // MOCO
        Self { layout, regs }
    }

    pub fn ra8() -> Self {
        Self::new(&RA8_SYSC)
    }

    pub fn ra6() -> Self {
        Self::new(&RA6_SYSC)
    }

    fn oscsf(&self) -> u8 {
        let running = |off: usize| self.regs[off] & 1 == 0;
        let mut sf = 0;
        if running(RA_HOCOCR) {
            sf |= HOCOSF;
        }
        if running(RA_MOSCCR) {
            sf |= RA_MOSCSF;
        }
        if running(RA_PLLCR) {
            sf |= RA_PLLSF;
        }
        if running(RA_PLL2CR) {
            sf |= RA_PLL2SF;
        }
        sf
    }
}

impl Peripheral for RaSyscRegs {
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn read(&self, offset: u64) -> SimResult<u8> {
        let off = offset as usize;
        if off >= self.layout.size {
            return Ok(0);
        }
        if off == RA_OSCSF {
            return Ok(self.oscsf());
        }
        let v = self.regs[off];
        if self.layout.ckcr.contains(&off) {
            return Ok((v & !CKSRDY) | if v & CKSREQ != 0 { CKSRDY } else { 0 });
        }
        Ok(v)
    }

    fn write(&mut self, offset: u64, value: u8) -> SimResult<()> {
        let off = offset as usize;
        if off < self.layout.size && off != RA_OSCSF {
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
            "peripheral": self.layout.name,
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
        let mut sys = RaSyscRegs::ra8();
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
        let mut sys = RaSyscRegs::ra8();
        for &off in RA8_SYSC.ckcr {
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
        let mut sys = RaSyscRegs::ra8();
        // Reset: HOCO running, MOSC/PLL/PLL2 stopped, MOCO selected.
        assert_eq!(sys.read(RA_OSCSF as u64).unwrap(), HOCOSF);
        assert_eq!(sys.read(RA_SCKSCR as u64).unwrap(), 1);
        // bsp_clock_init: MOSCCR = 0, wait MOSCSF; PLLCR = 0, wait PLLSF.
        sys.write(RA_MOSCCR as u64, 0).unwrap();
        assert_eq!(sys.read(RA_OSCSF as u64).unwrap() & RA_MOSCSF, RA_MOSCSF);
        sys.write(RA_PLLCR as u64, 0).unwrap();
        sys.write(RA_PLL2CR as u64, 0).unwrap();
        assert_eq!(
            sys.read(RA_OSCSF as u64).unwrap(),
            HOCOSF | RA_MOSCSF | RA_PLLSF | RA_PLL2SF
        );
        // Stopping HOCO and the PLL clears their flags; OSCSF is read-only.
        sys.write(RA_HOCOCR as u64, 1).unwrap();
        sys.write(RA_PLLCR as u64, 1).unwrap();
        sys.write(RA_OSCSF as u64, 0xFF).unwrap();
        assert_eq!(sys.read(RA_OSCSF as u64).unwrap(), RA_MOSCSF | RA_PLL2SF);
    }

    #[test]
    fn ra8_system_registers_read_back() {
        let mut sys = RaSyscRegs::ra8();
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
    fn ra6_sysc_runs_the_fsp_clock_bring_up() {
        let mut sys = RaSyscRegs::ra6();
        // Reset: HOCO running, MOSC/PLL/PLL2 stopped, MOCO selected.
        assert_eq!(sys.read(RA_OSCSF as u64).unwrap(), HOCOSF);
        assert_eq!(sys.read(RA_SCKSCR as u64).unwrap(), 1);
        sys.write(RA_MOSCCR as u64, 0).unwrap();
        sys.write(RA_PLLCR as u64, 0).unwrap();
        assert_eq!(
            sys.read(RA_OSCSF as u64).unwrap(),
            HOCOSF | RA_MOSCSF | RA_PLLSF
        );
        // SCKSCR to PLL (5), SOSCCR/SOMCR (+0x480/+0x481) and the last
        // VBTBKR byte (+0x6FF) read back; past 0x700 reads 0.
        for (off, v) in [(0x26u64, 5u8), (0x480, 1), (0x481, 2), (0x6FF, 0xA5)] {
            sys.write(off, v).unwrap();
            assert_eq!(sys.read(off).unwrap(), v, "{off:#x}");
        }
        sys.write(0x700, 0xAA).unwrap();
        assert_eq!(sys.read(0x700).unwrap(), 0);
    }

    #[test]
    fn ra6_sysc_handshake_is_only_on_its_own_ckcr() {
        let mut sys = RaSyscRegs::ra6();
        // USBCKCR / OCTACKCR / CANFDCKCR have CKSREQ/CKSRDY on RA6M5.
        for off in [0x74u64, 0x75, 0x76] {
            sys.write(off, CKSREQ | 1).unwrap();
            assert_eq!(sys.read(off).unwrap(), CKSREQ | CKSRDY | 1, "{off:#x}");
        }
        // The RA8 SCICKCR offset is a plain byte here.
        sys.write(0x55, CKSREQ).unwrap();
        assert_eq!(sys.read(0x55).unwrap(), CKSREQ);
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
