// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Pieces the three ESP32-family GPIO models (classic, C3/C6, S3) share for
//! GPIO interrupts and for taking part in a world `gpio_net`.
//!
//! The register files differ per chip (bank split, offsets, which INT_ENA bit
//! is "this CPU"), but three things are the same IP on every member:
//!
//! * `GPIO_PINn_REG.INT_TYPE` (bits [9:7]): 1 rising, 2 falling, 3 any edge,
//!   4 low level, 5 high level (ESP32 / ESP32-C3 / ESP32-S3 TRM, "GPIO
//!   interrupt");
//! * `GPIO_PINn_REG.PAD_DRIVER` (bit 2): 1 = open drain, the pad drives only a
//!   0 and releases for a 1;
//! * a pad's own output stage: ENABLE drives the pad, unless open drain holds
//!   a 1, in which case nothing does.
//!
//! The scheduler chain ([`IrqLevelWatch`]) is how a GPIO interrupt reaches
//! the interrupt matrix on a walk-free bus: the GPIO models are
//! `uses_scheduler()` peripherals, the matrix fabrics re-derive their sources
//! from `matrix_irq_sources_into` when an event of a scheduler peripheral
//! fires, and nothing else would re-derive them after an edge applied from
//! outside (a board button, a `gpio_net`) or after firmware acknowledges the
//! interrupt in `STATUS_W1TC`.

/// `GPIO_PINn_REG.PAD_DRIVER`: open drain.
pub(crate) const PIN_PAD_DRIVER: u32 = 1 << 2;

/// `INT_TYPE` field of a `GPIO_PINn_REG` word.
#[inline]
pub(crate) fn int_type(pin_word: u32) -> u32 {
    (pin_word >> 7) & 7
}

/// Whether an input change `before -> after` latches the pin's status bit
/// for `INT_TYPE` `kind` (edge types 1..3 only).
#[inline]
pub(crate) fn edge_hits(kind: u32, before: bool, after: bool) -> bool {
    match kind {
        1 => !before && after,
        2 => before && !after,
        3 => before != after,
        _ => false,
    }
}

/// Whether level `level` holds the status bit of a level-type pin
/// (`INT_TYPE` 4 = low, 5 = high).
#[inline]
pub(crate) fn level_hits(kind: u32, level: bool) -> bool {
    matches!((kind, level), (4, false) | (5, true))
}

/// Which pads of a 32-bit bank drive their own output stage: ENABLE set,
/// except an open-drain pad holding a 1 (released).
#[inline]
pub(crate) fn driving_mask(enable: u32, out: u32, open_drain: u32) -> u32 {
    enable & !(open_drain & out)
}

/// Edge detector for a GPIO model's matrix interrupt line on a walk-free bus.
///
/// `take` answers whether the line moved since it was last reported, so the
/// model's `take_scheduled_events` arms exactly one event per change; the
/// event's delivery is what makes the fabric re-derive its sources
/// (`SystemBus::deliver_scheduled_irq_levels`).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IrqLevelWatch {
    reported: bool,
    seq: u32,
}

impl IrqLevelWatch {
    /// `Some(token)` when `level` differs from the last reported level.
    pub(crate) fn take(&mut self, level: bool) -> Option<u32> {
        if level == self.reported {
            return None;
        }
        self.reported = level;
        self.seq = self.seq.wrapping_add(1);
        Some(self.seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_and_level_types_follow_the_trm_encoding() {
        assert!(edge_hits(1, false, true) && !edge_hits(1, true, false));
        assert!(edge_hits(2, true, false) && !edge_hits(2, false, true));
        assert!(edge_hits(3, true, false) && edge_hits(3, false, true));
        assert!(!edge_hits(0, false, true) && !edge_hits(4, true, false));
        assert!(level_hits(4, false) && !level_hits(4, true));
        assert!(level_hits(5, true) && !level_hits(5, false));
    }

    #[test]
    fn an_open_drain_one_is_released() {
        // pad 0: push-pull 1; pad 1: open-drain 1 (released); pad 2:
        // open-drain 0 (drives); pad 3: output disabled.
        let m = driving_mask(0b0111, 0b0011, 0b0110);
        assert_eq!(m, 0b0101);
    }

    #[test]
    fn the_level_watch_reports_each_change_once() {
        let mut w = IrqLevelWatch::default();
        assert_eq!(w.take(false), None);
        assert!(w.take(true).is_some());
        assert_eq!(w.take(true), None);
        assert!(w.take(false).is_some());
    }
}
