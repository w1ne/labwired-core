// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Output-compare pins in the PWM waveform modes: what `analogWrite` drives.
//!
//! With `COMnx` set to 2 (non-inverting) or 3 (inverting) in a PWM mode, the
//! waveform generator owns the pad and the `PORTx` latch no longer reaches it
//! (ATmega328P datasheet, sections 15.7, 18.7). The core keeps its own latch
//! in `io`, so firmware reads back what it wrote, and the bus-side port model
//! (which holds the pad level every observer reads) gets the latch with the
//! owned bits replaced by the waveform level.
//!
//! The level is a function of the counter, so it is exact at every
//! instruction boundary however the counter got there:
//! - fast PWM: set at BOTTOM, cleared after the compare match, so high for
//!   `TCNT <= OCR` (duty `(OCR + 1) / (TOP + 1)`; `OCR = TOP` holds it high);
//! - phase-correct PWM: cleared on the match going up, set on the match
//!   going down, so high while `TCNT < OCR`, and at `TCNT == OCR` on the
//!   down slope (duty `OCR / TOP`, period `2 * TOP`). Timer1 and Timer2 (in
//!   WGM 1) count down as on silicon. Timer0 still counts up and wraps in
//!   phase-correct mode, so its period there is half the silicon's (Arduino
//!   runs Timer0 in fast PWM).
//!
//! Not modelled: `COMnx = 1` (toggle on match), which is stateful, and the
//! non-PWM modes, where the pad keeps following `PORTx`.
//!
//! On the mega parts the 16-bit Timer1/3/4/5 drive their A, B and C units
//! the same way (DS2549 17.9.3, 17.9.4, Tables 17-4 and 17-5), and OC0A and
//! OC1C share PB7 through the output compare modulator (DS2549 section 19).

use super::timer1::Timer1;
use super::{Avr, AvrModel, AVR_IO_MIRROR_BASE, AVR_PINB, AVR_PIND};
use crate::Bus;

/// Waveform family of a timer's current WGM setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Waveform {
    /// Normal or CTC: the pads follow `PORTx` here.
    NonPwm,
    Fast,
    PhaseCorrect,
}

/// The pad level an output-compare unit drives, or `None` when the unit
/// does not own the pad.
pub(super) fn oc_level(
    wave: Waveform,
    com: u8,
    tcnt: u16,
    down: bool,
    ocr: u16,
    top: u16,
) -> Option<bool> {
    let high = match (wave, com & 0x03) {
        (Waveform::NonPwm, _) | (_, 0) | (_, 1) => return None,
        (Waveform::Fast, _) => tcnt <= ocr,
        (Waveform::PhaseCorrect, _) => ocr >= top || tcnt < ocr || (down && tcnt == ocr),
    };
    Some(if com & 0x03 == 3 { !high } else { high })
}

/// Timer0/Timer2 waveform. Only the TOP = 0xFF modes count the way the
/// counters here do; the TOP = OCRnA modes (5, 7) are left to `PORTx`.
fn wave_8bit(tccra: u8, tccrb: u8) -> Waveform {
    match (tccra & 0x03) | ((tccrb & 0x08) >> 1) {
        1 => Waveform::PhaseCorrect,
        3 => Waveform::Fast,
        _ => Waveform::NonPwm,
    }
}

/// The output-compare units, in the order of [`OcMap::pins`].
const UNITS: usize = 16;

/// Where a part bonds its output-compare pins: the `PORTx` data address of
/// each port slot (0 = no slot) and the `(slot, bit)` of OC0A, OC0B, OC1A,
/// OC1B, OC1C, OC2A, OC2B, OC3A..C, OC4A..C, OC5A..C (`None` = not bonded or
/// no such unit).
pub(super) struct OcMap {
    ports: [u16; PORT_SLOTS],
    pins: [Option<(usize, u8)>; UNITS],
}

/// Port slots of an [`OcMap`].
pub(super) const PORT_SLOTS: usize = 5;

/// PORTx data addresses (PINx + 2).
const PORTB: u16 = AVR_PINB + 2;
const PORTD: u16 = AVR_PIND + 2;
const PORTE: u16 = 0x2E;
const PORTG: u16 = 0x34;
const PORTH: u16 = 0x102;
const PORTL: u16 = 0x10B;

/// Index of OC0A and OC1C in [`OcMap::pins`]: the output compare modulator
/// pair.
const OC0A: usize = 0;
const OC1C: usize = 4;

/// ATmega328P (and 328PB) pins: OC0A = PD6, OC0B = PD5, OC1A = PB1,
/// OC1B = PB2, OC2A = PB3, OC2B = PD3 (DS40002061B, section 5.1).
const MAP_328P: OcMap = OcMap {
    ports: [PORTB, PORTD, 0, 0, 0],
    pins: [
        Some((1, 6)),
        Some((1, 5)),
        Some((0, 1)),
        Some((0, 2)),
        None,
        Some((0, 3)),
        Some((1, 3)),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ],
};

/// ATmega2560 pins (DS2549, section 13.3 "Alternate Port Functions"):
/// OC0A = PB7, OC0B = PG5, OC1A/B/C = PB5/PB6/PB7, OC2A = PB4, OC2B = PH6,
/// OC3A/B/C = PE3/PE4/PE5, OC4A/B/C = PH3/PH4/PH5, OC5A/B/C = PL3/PL4/PL5.
/// On the Arduino Mega: D13, D4, D11, D12, D13, D10, D9, D5, D2, D3, D6, D7,
/// D8, D46, D45, D44.
const MAP_MEGA2560: OcMap = OcMap {
    ports: [PORTB, PORTG, PORTH, PORTE, PORTL],
    pins: [
        Some((0, 7)),
        Some((1, 5)),
        Some((0, 5)),
        Some((0, 6)),
        Some((0, 7)),
        Some((0, 4)),
        Some((2, 6)),
        Some((3, 3)),
        Some((3, 4)),
        Some((3, 5)),
        Some((2, 3)),
        Some((2, 4)),
        Some((2, 5)),
        Some((4, 3)),
        Some((4, 4)),
        Some((4, 5)),
    ],
};

/// ATmega1281: the 2560's pins, but its 64-pin package has no port H or L,
/// so OC2B and the Timer4/Timer5 outputs are not bonded.
const MAP_MEGA1281: OcMap = OcMap {
    ports: [PORTB, PORTG, 0, PORTE, 0],
    pins: [
        Some((0, 7)),
        Some((1, 5)),
        Some((0, 5)),
        Some((0, 6)),
        Some((0, 7)),
        Some((0, 4)),
        None,
        Some((3, 3)),
        Some((3, 4)),
        Some((3, 5)),
        None,
        None,
        None,
        None,
        None,
        None,
    ],
};

/// ATmega32U4 pins (DS7766, section 10.3 "Alternate Port Functions"):
/// OC0A = PB7, OC0B = PD0, OC1A = PB5, OC1B = PB6; there is no Timer2. On the
/// Arduino Leonardo these are D11, D3, D9, D10. OC1C (PB7) and the Timer3 /
/// Timer4 outputs are not modelled.
const MAP_M32U4: OcMap = OcMap {
    ports: [PORTB, PORTD, 0, 0, 0],
    pins: [
        Some((0, 7)),
        Some((1, 0)),
        Some((0, 5)),
        Some((0, 6)),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ],
};

/// What the output-compare units drive, as last pushed to the bus-side port
/// model: per port slot the pads they own and the level there, and the
/// OC0A/OC1C levels while the output compare modulator combines them on one
/// pad (whose level then also depends on its `PORTx` latch bit).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct OcState {
    mask: [u8; PORT_SLOTS],
    level: [u8; PORT_SLOTS],
    modulated: Option<(bool, bool)>,
}

/// The three units of a 16-bit timer: `COMnA` (7:6), `COMnB` (5:4) and, with
/// unit C, `COMnC` (3:2).
fn oc16_levels(t: &Timer1) -> [Option<bool>; 3] {
    let (wave, top) = (t.waveform(), t.top());
    let unit = |shift: u8, ocr: u16| oc_level(wave, t.tccra >> shift, t.tcnt, t.down, ocr, top);
    [
        unit(6, t.ocra),
        unit(4, t.ocrb),
        if t.has_c { unit(2, t.ocrc) } else { None },
    ]
}

impl Avr {
    fn oc_map(&self) -> &'static OcMap {
        match self.model {
            AvrModel::Atmega2560 => &MAP_MEGA2560,
            AvrModel::Atmega1281 => &MAP_MEGA1281,
            AvrModel::Atmega32u4 => &MAP_M32U4,
            _ => &MAP_328P,
        }
    }

    /// Any `COMnx` bit set on a timer this part has.
    fn any_com(&self) -> bool {
        (self.tccr0a | self.tccr2a) & 0xF0 != 0
            || self.t1.tccra & 0xFC != 0
            || self.t16.t.iter().any(|t| t.tccra & 0xFC != 0)
    }

    /// What the output-compare units own and drive right now.
    fn oc_override(&self) -> OcState {
        let w0 = wave_8bit(self.tccr0a, self.tccr0b);
        let t0 = u16::from(self.tcnt0);
        let w2 = wave_8bit(self.tccr2a, self.tccr2b);
        let t2 = u16::from(self.tcnt2);
        let [oc1a, oc1b, oc1c] = oc16_levels(&self.t1);
        let mut levels = [None; UNITS];
        levels[..7].copy_from_slice(&[
            oc_level(w0, self.tccr0a >> 6, t0, false, u16::from(self.ocr0a), 0xFF),
            oc_level(w0, self.tccr0a >> 4, t0, false, u16::from(self.ocr0b), 0xFF),
            oc1a,
            oc1b,
            oc1c,
            oc_level(
                w2,
                self.tccr2a >> 6,
                t2,
                self.t2_down,
                u16::from(self.ocr2a),
                0xFF,
            ),
            oc_level(
                w2,
                self.tccr2a >> 4,
                t2,
                self.t2_down,
                u16::from(self.ocr2b),
                0xFF,
            ),
        ]);
        if self.is_mega() {
            for (k, t) in self.t16.t.iter().enumerate() {
                levels[7 + 3 * k..10 + 3 * k].copy_from_slice(&oc16_levels(t));
            }
        }
        let map = self.oc_map();
        let mut st = OcState::default();
        for (pin, l) in map.pins.iter().zip(levels) {
            if let (Some((port, bit)), Some(l)) = (*pin, l) {
                st.mask[port] |= 1 << bit;
                if l {
                    st.level[port] |= 1 << bit;
                }
            }
        }
        // DS2549 section 19: with both OC1C and OC0A enabled on the shared
        // pad, the modulator combines them; the pad level is then resolved
        // against its PORTx latch bit in `port_pad_value`.
        if let (Some(a), Some(c), Some(pa), Some(pc)) =
            (levels[OC0A], levels[OC1C], map.pins[OC0A], map.pins[OC1C])
        {
            if pa == pc {
                st.modulated = Some((a, c));
            }
        }
        st
    }

    /// The pad byte the bus-side port model gets for a `PORTx` latch value.
    pub(super) fn port_pad_value(&self, port_addr: u16, latch: u8) -> u8 {
        let map = self.oc_map();
        let Some(i) = map.ports.iter().position(|&p| p == port_addr) else {
            return latch;
        };
        let (mask, mut level) = (self.oc.mask[i], self.oc.level[i]);
        if let (Some((a, c)), Some((port, bit))) = (self.oc.modulated, map.pins[OC1C]) {
            if port == i {
                // PORTB7 = 0: OC1C AND OC0A; PORTB7 = 1: OC1C OR OC0A
                // (DS2549 19.2, Figure 19-3).
                let on = if latch & (1 << bit) != 0 {
                    a || c
                } else {
                    a && c
                };
                level = (level & !(1 << bit)) | (u8::from(on) << bit);
            }
        }
        (latch & !mask) | (level & mask)
    }

    /// Push the waveform levels onto the pads after the timers moved. One
    /// bus write per pad change; nothing while no `COMnx` bit is set.
    #[inline]
    pub(super) fn sync_oc_pins(&mut self, bus: &mut dyn Bus) {
        if self.oc == OcState::default() && !self.any_com() {
            return;
        }
        let next = self.oc_override();
        if next == self.oc {
            return;
        }
        let prev = std::mem::replace(&mut self.oc, next);
        let modulator_moved = prev.modulated != next.modulated;
        for (i, &port_addr) in self.oc_map().ports.iter().enumerate() {
            let same = next.mask[i] == prev.mask[i] && next.level[i] == prev.level[i];
            let mod_here = modulator_moved && self.oc_map().pins[OC1C].is_some_and(|(p, _)| p == i);
            if port_addr == 0 || (same && !mod_here) {
                continue;
            }
            let latch = self.io[usize::from(port_addr - 0x20)];
            let pad = self.port_pad_value(port_addr, latch);
            // Best-effort like every port mirror write: a chip yaml without
            // the window has no pad to drive.
            let _mirror = bus.write_u8(AVR_IO_MIRROR_BASE + u64::from(port_addr), pad);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_pwm_duty_is_ocr_plus_one_over_256() {
        let high = (0..=255u16)
            .filter(|t| oc_level(Waveform::Fast, 2, *t, false, 200, 0xFF) == Some(true))
            .count();
        assert_eq!(high, 201);
        // OCR = TOP holds the pad high; inverting mode is the complement.
        assert!(
            (0..=255u16).all(|t| oc_level(Waveform::Fast, 2, t, false, 0xFF, 0xFF) == Some(true))
        );
        assert_eq!(
            oc_level(Waveform::Fast, 3, 10, false, 200, 0xFF),
            Some(false)
        );
    }

    #[test]
    fn phase_correct_duty_is_ocr_over_top() {
        // One dual-slope cycle: 0..=255 up, then 254..=1 down (510 clocks).
        let cycle = || {
            (0..=255u16)
                .map(|t| (t, false))
                .chain((1..=254u16).rev().map(|t| (t, true)))
        };
        let high = cycle()
            .filter(|(t, d)| oc_level(Waveform::PhaseCorrect, 2, *t, *d, 200, 0xFF) == Some(true))
            .count();
        assert_eq!(high, 400, "2 * OCR of 2 * TOP clocks");
        let mid = cycle()
            .filter(|(t, d)| oc_level(Waveform::PhaseCorrect, 2, *t, *d, 128, 0xFF) == Some(true))
            .count();
        assert_eq!(mid, 256, "analogWrite(128): 256 of 510");
        assert!(
            cycle().all(|(t, d)| oc_level(Waveform::PhaseCorrect, 2, t, d, 0, 0xFF) == Some(false))
        );
        assert!(cycle()
            .all(|(t, d)| oc_level(Waveform::PhaseCorrect, 2, t, d, 0xFF, 0xFF) == Some(true)));
    }

    #[test]
    fn disconnected_toggle_and_non_pwm_leave_the_pad_to_port() {
        assert_eq!(oc_level(Waveform::Fast, 0, 0, false, 10, 0xFF), None);
        assert_eq!(oc_level(Waveform::Fast, 1, 0, false, 10, 0xFF), None);
        assert_eq!(oc_level(Waveform::NonPwm, 2, 0, false, 10, 0xFF), None);
    }
}
