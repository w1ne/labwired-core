// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The ONE way a GPIO port decides that a pad is being driven by a peripheral
//! rather than by its own output latch.
//!
//! # The question every family asks
//!
//! "Firmware is reading pad N — who is actually driving it?" Every chip answers
//! it the same way in principle and differently in registers:
//!
//! | family | selector | where it lives |
//! |---|---|---|
//! | STM32 V2 | AF nibble | `MODER` selects AF, `AFRL`/`AFRH` pick which |
//! | STM32 F1 | fixed map | `CRL`/`CRH` mode+CNF say "AF output" |
//! | ESP32 / -S3 / -C3 | matrix index | `GPIO_FUNCn_OUT_SEL_CFG.out_sel` |
//! | RP2040 | function number | `IO_BANK0.GPIOn_CTRL.FUNCSEL` |
//!
//! The register decode differs; the *shape* does not. A pad currently selects
//! some peripheral signal, and if that signal is one a live peripheral
//! publishes a wire for, the pad reads that wire instead of the GPIO latch.
//!
//! [`PadRoutes`] is that shape, once. A family binds `(pad, selector) → (wire,
//! line)` at config-build time and supplies a closure that decodes its own
//! registers into "the selector pad N currently has". Everything downstream —
//! `read_gpio_pad`, the logic analyzer sampling through it, push-capture
//! registration, and the signal name in `gpio_routing` — is handled here.
//!
//! Before this, three families had grown three parallel copies of it (the STM32
//! AF table, and bespoke `set_i2c_lines` paths on the C3 and S3), each with its
//! own tap-registration bookkeeping. Adding UART or SPI visibility to a family
//! meant a fourth. Now it means one more binding.
//!
//! # What a family owes this type
//!
//! * Call [`PadRoutes::bind`] once per (pad, signal) the datasheet allows.
//! * Pass a selector decode to [`PadRoutes::level`] and friends that reads the
//!   live registers — so re-routing a pad at runtime follows immediately, and a
//!   pad taken back for plain GPIO stops reading the wire.
//! * Call [`PadRoutes::sync_taps`] after any register write, so push capture
//!   follows a pad that changed hands.

use std::sync::Arc;

use crate::logic_capture::LogicTap;
use crate::peripherals::pad_lines::PadLines;

/// One pad that a peripheral signal can drive.
#[derive(Debug, Clone)]
struct Route {
    pin: u8,
    /// The selector value this pad must currently hold for the route to be
    /// live: an AF nibble, a GPIO-matrix signal index, a FUNCSEL number.
    /// `None` means "this family has a fixed mapping" — the decode closure
    /// answers whether the pad is handed to a peripheral at all.
    selector: Option<u32>,
    /// Index into the bound cell's lines.
    line: usize,
    /// Signal name surfaced through `gpio_routing().func`, e.g. `"I2C1_SDA"`.
    func: &'static str,
    /// Index into [`PadRoutes::cells`].
    cell: usize,
}

/// Every pad-to-peripheral-wire binding on one GPIO port.
///
/// Empty on a port with no peripheral routed to it, where every operation
/// costs one `is_empty` check.
#[derive(Debug, Default)]
pub struct PadRoutes {
    /// Deduplicated wires, so several pads of one controller share an entry.
    cells: Vec<Arc<PadLines>>,
    routes: Vec<Route>,
    /// Per cell, per line: the watch channels last registered with that wire.
    /// Cached so an unrelated register write costs no mutex traffic.
    registered: Vec<Vec<Vec<u32>>>,
}

impl PadRoutes {
    pub fn new() -> Self {
        Self::default()
    }

    /// `true` when no peripheral is routed to this port at all.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Every distinct signal name BOUND to this port, regardless of which one a
    /// pad currently selects — `["I2C1_SCL", "I2C1_SDA", …]`, in binding order.
    ///
    /// Deliberately *not* [`Self::func`]: that answers "who drives pin N right
    /// now", which depends on live register state and is therefore zero on a
    /// freshly built bus. This answers the static question "what could ever be
    /// seen here", which is what the bus-visibility scoreboard
    /// (`crates/core/tests/bus_visibility.rs`) measures — a bus with no binding
    /// can never reach a pad, so the logic analyzer can never show it, no matter
    /// what the firmware does.
    ///
    /// Read-only and allocating; it is a reporting seam, not a hot path.
    pub fn bound_functions(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for route in &self.routes {
            if !out.contains(&route.func) {
                out.push(route.func);
            }
        }
        out
    }

    /// Bind `pin` to `line` of `cell`, live whenever the pad's selector reads
    /// `selector`. Call once per datasheet-allowed (pad, signal) pair.
    pub fn bind(
        &mut self,
        cell: &Arc<PadLines>,
        pin: u8,
        selector: Option<u32>,
        line: usize,
        func: &'static str,
    ) {
        let cell_idx = match self.cells.iter().position(|c| Arc::ptr_eq(c, cell)) {
            Some(i) => i,
            None => {
                self.cells.push(cell.clone());
                self.registered.push(vec![Vec::new(); cell.names().len()]);
                self.cells.len() - 1
            }
        };
        self.routes.push(Route {
            pin,
            selector,
            line,
            func,
            cell: cell_idx,
        });
    }

    /// The route currently driving `pin`, if any.
    ///
    /// `selector_of` decodes the family's own registers: `Some(value)` when the
    /// pad is handed to a peripheral (the AF nibble / matrix index / FUNCSEL),
    /// `None` when it is a plain GPIO. A route with `selector: None` matches
    /// any `Some`, which is the fixed-mapping case (STM32 F1).
    fn active<F>(&self, pin: u8, selector_of: F) -> Option<&Route>
    where
        F: Fn(u8) -> Option<u32>,
    {
        if self.routes.is_empty() {
            return None;
        }
        let current = selector_of(pin)?;
        self.routes
            .iter()
            .find(|r| r.pin == pin && r.selector.is_none_or(|want| want == current))
    }

    /// The live wire level for `pin`, or `None` when no peripheral drives it
    /// and the caller should fall back to its own register truth.
    pub fn level<F>(&self, pin: u8, selector_of: F) -> Option<bool>
    where
        F: Fn(u8) -> Option<u32>,
    {
        let route = self.active(pin, selector_of)?;
        Some(self.cells[route.cell].level(route.line))
    }

    /// The datasheet name of the signal currently driving `pin`, for
    /// `gpio_routing().func`.
    pub fn func<F>(&self, pin: u8, selector_of: F) -> Option<&'static str>
    where
        F: Fn(u8) -> Option<u32>,
    {
        self.active(pin, selector_of).map(|route| route.func)
    }

    /// Register `watched` pads' channels with the wires that drive them, so
    /// each wire reports its own transitions at the cycles they occurred
    /// (push capture). Only wires whose channel set actually changed are
    /// touched, so an unrelated register write is nearly free.
    ///
    /// Call after every register write: a pad that changed hands must stop
    /// reporting from its old source and start reporting from its new one.
    pub fn sync_taps<F>(&mut self, tap: &LogicTap, watched: &[(u8, u32)], selector_of: F)
    where
        F: Fn(u8) -> Option<u32>,
    {
        if self.routes.is_empty() {
            return;
        }
        let mut per_cell: Vec<Vec<Vec<u32>>> = self
            .cells
            .iter()
            .map(|cell| vec![Vec::new(); cell.names().len()])
            .collect();
        for &(pin, channel) in watched {
            if let Some(route) = self.active(pin, &selector_of) {
                if let Some(slot) = per_cell[route.cell].get_mut(route.line) {
                    slot.push(channel);
                }
            }
        }
        for (idx, channels) in per_cell.into_iter().enumerate() {
            if self.registered[idx] != channels {
                self.cells[idx].install_tap(Some(tap.clone()), channels.clone());
                self.registered[idx] = channels;
            }
        }
    }

    /// Drop the push registrations THIS routing table installed — the disarm
    /// path.
    ///
    /// Only the ones it installed, which is the whole subtlety. A controller's
    /// wire reaches pads on SEVERAL ports — an STM32 USART3 can come out on
    /// PB10, PC10 or PD8 — so one [`PadLines`] cell is held by several
    /// `PadRoutes`, one per port. `Machine::logic_watch` then offers every
    /// peripheral its slice of the watch set, and a port with no watched pins
    /// lands here. Clearing the cell wholesale at that point wipes the
    /// registration a DIFFERENT port installed moments earlier, and since ports
    /// are visited in bus-index order, watching PB10 and then reaching gpioc
    /// silently disarmed the tap: the pad still read correctly, so the levels
    /// looked right and the trace was simply empty.
    ///
    /// A cell this table never registered channels on is therefore left alone.
    pub fn clear_taps(&mut self) {
        for (idx, cell) in self.cells.iter().enumerate() {
            if self.registered[idx]
                .iter()
                .all(|channels| channels.is_empty())
            {
                continue;
            }
            cell.clear_tap();
            self.registered[idx] = vec![Vec::new(); cell.names().len()];
        }
    }

    /// Force the next [`sync_taps`](Self::sync_taps) to install unconditionally,
    /// for use when arming a fresh watch.
    pub fn invalidate_registrations(&mut self) {
        for slot in &mut self.registered {
            for line in slot.iter_mut() {
                *line = vec![u32::MAX];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const I2C: &[&str] = &["SCL", "SDA"];
    const SCL: usize = 0;
    const SDA: usize = 1;

    fn wire() -> Arc<PadLines> {
        Arc::new(PadLines::new(I2C, &[true, true]))
    }

    #[test]
    fn a_pad_reads_the_wire_only_while_it_selects_that_signal() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        lines.set(&[false, true]);

        // Pad 6 currently selects AF4 → it reads the wire.
        assert_eq!(routes.level(6, |_| Some(4)), Some(false));
        // Same pad handed back to plain GPIO → the caller falls back.
        assert_eq!(routes.level(6, |_| None), None);
        // Selecting a DIFFERENT function must not read this wire.
        assert_eq!(routes.level(6, |_| Some(7)), None);
        // An unbound pad is never routed.
        assert_eq!(routes.level(9, |_| Some(4)), None);
    }

    #[test]
    fn a_fixed_mapping_route_matches_any_selector() {
        // STM32 F1: the registers say "this pad is an AF output" but not which.
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, None, SDA, "I2C1_SDA");
        lines.set(&[true, false]);
        assert_eq!(routes.level(6, |_| Some(0)), Some(false));
        assert_eq!(routes.level(6, |_| None), None, "still not a plain GPIO");
    }

    #[test]
    fn one_wire_serves_several_pads_without_duplicating_the_cell() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        routes.bind(&lines, 7, Some(4), SDA, "I2C1_SDA");
        routes.bind(&lines, 8, Some(4), SCL, "I2C1_SCL");
        assert_eq!(routes.cells.len(), 1, "deduplicated by identity");
        lines.set(&[false, true]);
        assert_eq!(routes.level(6, |_| Some(4)), Some(false));
        assert_eq!(routes.level(7, |_| Some(4)), Some(true));
    }

    #[test]
    fn the_signal_name_follows_the_live_routing() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 7, Some(4), SDA, "I2C1_SDA");
        assert_eq!(routes.func(7, |_| Some(4)), Some("I2C1_SDA"));
        assert_eq!(routes.func(7, |_| Some(5)), None);
    }

    #[test]
    fn watched_pads_register_with_the_wire_that_drives_them() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        routes.bind(&lines, 7, Some(4), SDA, "I2C1_SDA");

        let tap = LogicTap::new();
        routes.sync_taps(&tap, &[(6, 0), (7, 1)], |_| Some(4));

        lines.set(&[false, false]);
        assert_eq!(
            tap.take_events()
                .iter()
                .map(|e| (e.ch, e.value))
                .collect::<Vec<_>>(),
            vec![(0, false), (1, false)],
            "each watched pad reports from its own line",
        );
    }

    #[test]
    fn a_pad_taken_back_for_gpio_stops_reporting_from_the_wire() {
        // The regression this guards: re-routing must follow immediately, or a
        // plain GPIO pin silently keeps showing bus traffic.
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        let tap = LogicTap::new();
        routes.sync_taps(&tap, &[(6, 0)], |_| Some(4));
        lines.set(&[false, true]);
        assert_eq!(tap.take_events().len(), 1);

        // Firmware hands the pad back to GPIO.
        routes.sync_taps(&tap, &[(6, 0)], |_| None);
        lines.set(&[true, true]);
        assert!(
            tap.take_events().is_empty(),
            "the wire still moves, but this pad is no longer listening",
        );
    }

    #[test]
    fn re_syncing_an_unchanged_routing_does_not_reinstall() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        let tap = LogicTap::new();
        routes.sync_taps(&tap, &[(6, 0)], |_| Some(4));
        let before = routes.registered.clone();
        routes.sync_taps(&tap, &[(6, 0)], |_| Some(4));
        assert_eq!(routes.registered, before, "no churn on a quiet write");
    }

    #[test]
    fn clearing_taps_stops_every_wire() {
        let lines = wire();
        let mut routes = PadRoutes::new();
        routes.bind(&lines, 6, Some(4), SCL, "I2C1_SCL");
        let tap = LogicTap::new();
        routes.sync_taps(&tap, &[(6, 0)], |_| Some(4));
        routes.clear_taps();
        lines.set(&[false, false]);
        assert!(tap.take_events().is_empty());
    }

    #[test]
    fn an_empty_routing_table_answers_nothing_and_costs_nothing() {
        let routes = PadRoutes::new();
        assert!(routes.is_empty());
        assert_eq!(routes.level(0, |_| Some(1)), None);
        assert_eq!(routes.func(0, |_| Some(1)), None);
    }
}
