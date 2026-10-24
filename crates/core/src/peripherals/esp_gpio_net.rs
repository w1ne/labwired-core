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

use std::sync::Arc;

use crate::peripherals::pad_lines::{PadLines, WireInputEdge};

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

/// How a chip encodes `GPIO_FUNCn_IN_SEL_CFG_REG`: the pad number field and
/// the `SEL` bit that routes the signal through the matrix (clear: the
/// signal takes its IO_MUX pad, which these models do not follow).
///
/// | part | pad field | `SEL` | source |
/// |---|---|---|---|
/// | ESP32-C3 | [4:0] | bit 6 | esp32c3 PAC `gpio::func_in_sel_cfg` |
/// | ESP32-C6 | [5:0] | bit 7 | esp32c6 PAC `gpio::func_in_sel_cfg` |
/// | ESP32-S3 | [5:0] | bit 7 | esp32s3 PAC `gpio::func_in_sel_cfg` |
/// | ESP32 (classic) | [5:0] | bit 7 (`SIG_IN_SEL`) | esp-idf `soc/esp32/include/soc/gpio_reg.h` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InSelLayout {
    pub(crate) pad_mask: u32,
    pub(crate) sel_bit: u32,
}

impl InSelLayout {
    /// The pad a `FUNCn_IN_SEL_CFG` word routes its signal from, if the
    /// matrix routes it at all.
    #[inline]
    pub(crate) fn pad(self, word: u32) -> Option<u8> {
        (word & self.sel_bit != 0).then_some((word & self.pad_mask) as u8)
    }
}

/// One peripheral INPUT signal of the GPIO matrix, bound to the line of the
/// peripheral wire it feeds (an SPI slave's SCK, a master's MISO, an I²C
/// controller's SCL/SDA, which read the wire they also drive).
#[derive(Debug, Clone)]
struct InputRoute {
    signal: u32,
    cell: Arc<PadLines>,
    line: usize,
}

/// The input half of the ESP32-family GPIO matrix for peripheral wires.
///
/// The output half is [`crate::peripherals::pad_routing::PadRoutes`]: a pad
/// selects the signal that drives it (`FUNCn_OUT_SEL_CFG`, pad-indexed). The
/// input matrix is the other way round: each peripheral input signal selects
/// the pad it reads (`FUNCn_IN_SEL_CFG`, signal-indexed), so one pad can feed
/// several inputs and an input follows firmware's re-routing at once.
///
/// What a level presented to a pad from outside (a world `gpio_net`) does:
/// every signal whose `IN_SEL` names that pad gets it on its line
/// ([`PadLines::set_input`]), and a [`WireInputEdge`] is queued for the bus to
/// hand to the peripheral owning the line
/// ([`crate::Peripheral::wire_input_edge`]). Shared by every ESP GPIO model
/// with a matrix; the signal numbers and the `IN_SEL` encoding are the chip's.
#[derive(Debug, Default)]
pub(crate) struct MatrixInputs {
    routes: Vec<InputRoute>,
}

impl MatrixInputs {
    /// Bind input `signal` to `line` of `cell`.
    pub(crate) fn bind(&mut self, signal: u32, cell: &Arc<PadLines>, line: usize) {
        if self
            .routes
            .iter()
            .any(|r| r.signal == signal && Arc::ptr_eq(&r.cell, cell) && r.line == line)
        {
            return;
        }
        self.routes.push(InputRoute {
            signal,
            cell: cell.clone(),
            line,
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// The level `level` reached pad `pin` from outside: hand it to every
    /// bound input whose `IN_SEL` names the pad.
    pub(crate) fn deliver(
        &self,
        pin: u8,
        level: bool,
        in_sel: impl Fn(u32) -> u32,
        layout: InSelLayout,
        out: &mut Vec<WireInputEdge>,
    ) {
        for r in &self.routes {
            if layout.pad(in_sel(r.signal)) == Some(pin) && r.cell.set_input(r.line, level) {
                out.push(WireInputEdge {
                    cell: r.cell.clone(),
                    line: r.line,
                    level,
                });
            }
        }
    }

    /// After a routing write: every bound input takes the level the outside
    /// holds on the pad it now selects (`external(pin)`), if any.
    pub(crate) fn sync(
        &self,
        in_sel: impl Fn(u32) -> u32,
        layout: InSelLayout,
        external: impl Fn(u8) -> Option<bool>,
        out: &mut Vec<WireInputEdge>,
    ) {
        for r in &self.routes {
            let Some(pin) = layout.pad(in_sel(r.signal)) else {
                continue;
            };
            if let Some(level) = external(pin) {
                if r.cell.set_input(r.line, level) {
                    out.push(WireInputEdge {
                        cell: r.cell.clone(),
                        line: r.line,
                        level,
                    });
                }
            }
        }
    }

    /// Mark every bound input that reads a pad on a world `gpio_net` as a
    /// net line ([`PadLines::mark_on_net`]).
    pub(crate) fn mark_on_net(
        &self,
        in_sel: impl Fn(u32) -> u32,
        layout: InSelLayout,
        on_net: impl Fn(u8) -> bool,
    ) {
        for r in &self.routes {
            if layout.pad(in_sel(r.signal)).is_some_and(&on_net) && !r.cell.on_net(r.line) {
                r.cell.mark_on_net(r.line);
            }
        }
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
