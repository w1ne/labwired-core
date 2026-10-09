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
//! - phase-correct PWM: high while `TCNT < OCR` (duty `OCR / TOP`). The
//!   counters here count up to TOP and wrap rather than back down, so the
//!   duty is right and the period is half the silicon's.
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
pub(super) fn oc_level(wave: Waveform, com: u8, tcnt: u16, ocr: u16, top: u16) -> Option<bool> {
    let high = match (wave, com & 0x03) {
        (Waveform::NonPwm, _) | (_, 0) | (_, 1) => return None,
        (Waveform::Fast, _) => tcnt <= ocr,
        (Waveform::PhaseCorrect, _) => ocr >= top || tcnt < ocr,
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

impl Avr {
    /// Pads the output-compare units own right now, as `(mask, level)` per
    /// port (B, D). OC0A = PD6, OC0B = PD5, OC2A = PB3, OC2B = PD3.
    fn oc_override(&self) -> ([u8; 2], [u8; 2]) {
        let mut mask = [0u8; 2];
        let mut level = [0u8; 2];
        let mut put = |port: usize, bit: u8, l: Option<bool>| {
            if let Some(l) = l {
                mask[port] |= 1 << bit;
                if l {
                    level[port] |= 1 << bit;
                }
            }
        };
        let w0 = wave_8bit(self.tccr0a, self.tccr0b);
        let t0 = u16::from(self.tcnt0);
        put(
            PORT_D,
            6,
            oc_level(w0, self.tccr0a >> 6, t0, u16::from(self.ocr0a), 0xFF),
        );
        put(
            PORT_D,
            5,
            oc_level(w0, self.tccr0a >> 4, t0, u16::from(self.ocr0b), 0xFF),
        );
        let w2 = wave_8bit(self.tccr2a, self.tccr2b);
        let t2 = u16::from(self.tcnt2);
        put(
            PORT_B,
            3,
            oc_level(w2, self.tccr2a >> 6, t2, u16::from(self.ocr2a), 0xFF),
        );
        put(
            PORT_D,
            3,
            oc_level(w2, self.tccr2a >> 4, t2, u16::from(self.ocr2b), 0xFF),
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
    pub(super) fn sync_oc_pins(&mut self, bus: &mut dyn Bus) {
        if (self.tccr0a | self.tccr2a) & 0xF0 == 0 && self.oc_mask == [0, 0] {
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
            .filter(|t| oc_level(Waveform::Fast, 2, *t, 200, 0xFF) == Some(true))
            .count();
        assert_eq!(high, 201);
        // OCR = TOP holds the pad high; inverting mode is the complement.
        assert!((0..=255u16).all(|t| oc_level(Waveform::Fast, 2, t, 0xFF, 0xFF) == Some(true)));
        assert_eq!(oc_level(Waveform::Fast, 3, 10, 200, 0xFF), Some(false));
    }

    #[test]
    fn phase_correct_duty_is_ocr_over_top() {
        let high = (0..=255u16)
            .filter(|t| oc_level(Waveform::PhaseCorrect, 2, *t, 200, 0xFF) == Some(true))
            .count();
        assert_eq!(high, 200);
        assert!(
            (0..=255u16).all(|t| oc_level(Waveform::PhaseCorrect, 2, t, 0, 0xFF) == Some(false))
        );
    }

    #[test]
    fn disconnected_toggle_and_non_pwm_leave_the_pad_to_port() {
        assert_eq!(oc_level(Waveform::Fast, 0, 0, 10, 0xFF), None);
        assert_eq!(oc_level(Waveform::Fast, 1, 0, 10, 0xFF), None);
        assert_eq!(oc_level(Waveform::NonPwm, 2, 0, 10, 0xFF), None);
    }
}
