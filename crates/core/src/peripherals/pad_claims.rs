// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Which peripheral signal currently drives each pad, for families that mux at
//! the PERIPHERAL rather than at the port.
//!
//! An STM32 pad names its own function in `AFRL`/`AFRH`, so the port can answer
//! "what am I?" by itself. Nordic and Silicon Labs invert that: the peripheral
//! names the pad (`PSEL.TXD` on nRF52, `GPIO_TIMERROUTE[n].CC0ROUTE` on EFR32
//! Series 2), and the port knows nothing until something tells it. This table
//! is that something — one shared instance per bus, written by the muxing
//! peripherals and read by the GPIO ports.
//!
//! It was `NrfPinClaims`, private to the Nordic module, until the EFR32 route
//! registers needed exactly the same structure. The mechanism is not Nordic;
//! only the encoding of "which pad" is, and that stays with each family.
//!
//! Reads are lock-free (`Relaxed`) because a pad read runs on the CPU walk and
//! must not contend with an MMIO write on another peripheral.

use std::sync::atomic::{AtomicU32, Ordering};

/// Slot value for a pad no peripheral has claimed.
pub(crate) const UNCLAIMED: u32 = u32::MAX;

#[derive(Debug)]
pub struct PadClaims {
    /// One slot per `port * pins_per_port + pin`, holding the claim token of
    /// the signal driving that pad or [`UNCLAIMED`].
    slots: Vec<AtomicU32>,
    pins_per_port: usize,
}

impl PadClaims {
    /// A table for `ports` ports of `pins_per_port` pads each.
    ///
    /// ⚠️ Both numbers are a property of the SILICON, not a convenience: nRF52
    /// has 2 x 32 and EFR32 Series 2 has 4 x 16, and `pad_index` has to agree
    /// with whatever the family's own encoding produces or a claim lands on the
    /// wrong pad — silently, because every index is in range.
    pub fn new(ports: usize, pins_per_port: usize) -> Self {
        Self {
            slots: (0..ports * pins_per_port)
                .map(|_| AtomicU32::new(UNCLAIMED))
                .collect(),
            pins_per_port,
        }
    }

    /// Flat slot index for `(port, pin)`.
    pub fn pad_index(&self, port: u8, pin: u8) -> usize {
        usize::from(port) * self.pins_per_port + usize::from(pin)
    }

    /// The claim token currently driving `(port, pin)`, or `None` for a pad no
    /// peripheral has selected — the `selector_of` closure
    /// [`PadRoutes::level`](super::pad_routing::PadRoutes::level) wants.
    ///
    /// `None` for an out-of-range pad rather than a panic, for the same reason
    /// [`PadLines::level`](super::pad_lines::PadLines::level) reads low: this
    /// runs on the CPU walk, where stale bookkeeping must not take the engine
    /// down.
    pub fn selector(&self, port: u8, pin: u8) -> Option<u32> {
        let idx = self.pad_index(port, pin);
        let token = self.slots.get(idx)?.load(Ordering::Relaxed);
        (token != UNCLAIMED).then_some(token)
    }

    /// Take `pad` for `token`, displacing whatever held it.
    ///
    /// Last writer wins, which is what neither silicon promises: "Only one
    /// peripheral can be assigned to drive a particular GPIO pin at a time.
    /// Failing to do so may result in unpredictable behavior" (nRF52840 PS
    /// v1.11 6.31.6, p790). Picking the most recent claim is one legal reading
    /// of unpredictable and the only one that stays deterministic.
    pub(crate) fn take(&self, pad: usize, token: u32) {
        if let Some(slot) = self.slots.get(pad) {
            slot.store(token, Ordering::Relaxed);
        }
    }

    /// Give up `pad` — but ONLY if `token` still holds it.
    ///
    /// The compare is load-bearing. Two peripherals can name the same pad, and
    /// the second one's claim must survive the first one's release; a blind
    /// store would hand the pad back to the GPIO latch while a live peripheral
    /// was still driving it.
    pub(crate) fn release(&self, pad: usize, token: u32) {
        if let Some(slot) = self.slots.get(pad) {
            let _ = slot.compare_exchange(token, UNCLAIMED, Ordering::Relaxed, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_is_visible_to_the_port_that_reads_it() {
        let claims = PadClaims::new(4, 16);
        assert_eq!(claims.selector(2, 8), None, "unclaimed out of the box");
        claims.take(claims.pad_index(2, 8), 7);
        assert_eq!(claims.selector(2, 8), Some(7));
        assert_eq!(claims.selector(2, 9), None, "neighbouring pad untouched");
    }

    /// The compare-exchange in `release`, stated as a test: a displaced signal
    /// releasing later must not steal the pad back from whoever holds it.
    #[test]
    fn a_stale_release_cannot_take_a_pad_from_its_live_owner() {
        let claims = PadClaims::new(4, 16);
        let pad = claims.pad_index(1, 3);
        claims.take(pad, 1);
        claims.take(pad, 2); // a second peripheral names the same pad
        claims.release(pad, 1); // the first one lets go, late
        assert_eq!(
            claims.selector(1, 3),
            Some(2),
            "the live claim survives a displaced signal's release",
        );
    }

    /// ⚠️ The two geometries in the tree do not share an index, and a table
    /// built with the wrong one puts claims on real pads that are not the ones
    /// the firmware named.
    #[test]
    fn the_index_follows_the_declared_geometry() {
        assert_eq!(PadClaims::new(2, 32).pad_index(1, 0), 32, "nRF52: 2 x 32");
        assert_eq!(PadClaims::new(4, 16).pad_index(1, 0), 16, "EFR32: 4 x 16");
    }
}
