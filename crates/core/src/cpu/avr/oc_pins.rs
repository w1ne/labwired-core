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

use super::{Avr, AVR_IO_MIRROR_BASE, AVR_PINB, AVR_PIND};
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

/// Port index into `oc_mask`/`oc_level`: 0 = port B, 1 = port D.
const PORT_B: usize = 0;
const PORT_D: usize = 1;

/// `(port, bit)` of OC0A, OC0B, OC1A, OC1B, OC2A, OC2B (`None` = not bonded).
type OcPins = [Option<(usize, u8)>; 6];

/// ATmega328P (and 328PB): OC0A = PD6, OC0B = PD5, OC1A = PB1, OC1B = PB2,
/// OC2A = PB3, OC2B = PD3 (DS40002061B, section 5.1).
const PINS_328P: OcPins = [
    Some((PORT_D, 6)),
    Some((PORT_D, 5)),
    Some((PORT_B, 1)),
    Some((PORT_B, 2)),
    Some((PORT_B, 3)),
    Some((PORT_D, 3)),
];

/// ATmega32U4 (DS7766, section 10.3 "Alternate Port Functions"): OC0A = PB7,
/// OC0B = PD0, OC1A = PB5, OC1B = PB6; there is no Timer2. On the Arduino
/// Leonardo these are D11, D3, D9, D10. OC1C (PB7) and the Timer3/Timer4
/// outputs are not modelled.
const PINS_M32U4: OcPins = [
    Some((PORT_B, 7)),
    Some((PORT_D, 0)),
    Some((PORT_B, 5)),
    Some((PORT_B, 6)),
    None,
    None,
];

impl Avr {
    /// Pads the output-compare units own right now, as `(mask, level)` per
    /// port (B, D), on this part's pins.
    fn oc_override(&self) -> ([u8; 2], [u8; 2]) {
        let pins = if self.model == super::AvrModel::Atmega32u4 {
            &PINS_M32U4
        } else {
            &PINS_328P
        };
        let mut mask = [0u8; 2];
        let mut level = [0u8; 2];
        let mut put = |pin: Option<(usize, u8)>, l: Option<bool>| {
            if let (Some((port, bit)), Some(l)) = (pin, l) {
                mask[port] |= 1 << bit;
                if l {
                    level[port] |= 1 << bit;
                }
            }
        };
        let w0 = wave_8bit(self.tccr0a, self.tccr0b);
        let t0 = u16::from(self.tcnt0);
        put(
            pins[0],
            oc_level(w0, self.tccr0a >> 6, t0, false, u16::from(self.ocr0a), 0xFF),
        );
        put(
            pins[1],
            oc_level(w0, self.tccr0a >> 4, t0, false, u16::from(self.ocr0b), 0xFF),
        );
        let (w1, top1, t1) = (self.t1.waveform(), self.t1.top(), self.t1.tcnt);
        put(
            pins[2],
            oc_level(w1, self.t1.tccra >> 6, t1, self.t1.down, self.t1.ocra, top1),
        );
        put(
            pins[3],
            oc_level(w1, self.t1.tccra >> 4, t1, self.t1.down, self.t1.ocrb, top1),
        );
        let w2 = wave_8bit(self.tccr2a, self.tccr2b);
        let t2 = u16::from(self.tcnt2);
        put(
            pins[4],
            oc_level(
                w2,
                self.tccr2a >> 6,
                t2,
                self.t2_down,
                u16::from(self.ocr2a),
                0xFF,
            ),
        );
        put(
            pins[5],
            oc_level(
                w2,
                self.tccr2a >> 4,
                t2,
                self.t2_down,
                u16::from(self.ocr2b),
                0xFF,
            ),
        );
        (mask, level)
    }

    /// The pad byte the bus-side port model gets for a `PORTx` latch value.
    pub(super) fn port_pad_value(&self, port_addr: u16, latch: u8) -> u8 {
        let i = match port_addr {
            a if a == AVR_PINB + 2 => PORT_B,
            a if a == AVR_PIND + 2 => PORT_D,
            _ => return latch,
        };
        (latch & !self.oc_mask[i]) | (self.oc_level[i] & self.oc_mask[i])
    }

    /// Push the waveform levels onto the pads after the timers moved. One
    /// bus write per pad change; nothing while no `COMnx` bit is set.
    #[inline]
    pub(super) fn sync_oc_pins(&mut self, bus: &mut dyn Bus) {
        // The ATmega2560's OC pads are on other pins (PB4..PB7, PE3..5, PG5,
        // PH3..6, PL3..5); its pads follow `PORTx` (see `AvrModel`).
        if self.model == super::AvrModel::Atmega2560 {
            return;
        }
        if (self.tccr0a | self.t1.tccra | self.tccr2a) & 0xF0 == 0 && self.oc_mask == [0, 0] {
            return;
        }
        let (mask, level) = self.oc_override();
        for (i, port_addr) in [(PORT_B, AVR_PINB + 2), (PORT_D, AVR_PIND + 2)] {
            if mask[i] == self.oc_mask[i] && level[i] == self.oc_level[i] {
                continue;
            }
            self.oc_mask[i] = mask[i];
            self.oc_level[i] = level[i];
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
