// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bus wiring of the analog multiplexers (the declarative `analog_mux`
//! primitive, [`crate::peripherals::components::declarative_analog_mux`]).
//!
//! Three events move the level on a mux's Z pin, and each has one home here:
//!
//! 1. **A select or enable pad moves.** Firmware stores to a GPIO output
//!    register. [`SystemBus::route_analog_muxes_on_write`] runs inside that
//!    MMIO write (the same choke as the edge-driven GPIO devices), so the next
//!    instruction, typically the ADC conversion start, already sees the new
//!    channel. A per-tick pass would be too late: the stock firmware of a
//!    real board sets the select lines and starts the conversion two
//!    instructions later.
//! 2. **An input level moves.** An analog source whose `connection:` is the
//!    mux seeds a Y input through [`SystemBus::seed_adc_channel`], which
//!    checks muxes before ADC peripherals.
//! 3. **Attach.** The first routing is pushed when the mux attaches.
//!
//! The mux drives its downstream channel through `seed_adc_channel` too, so a
//! mux behind a mux works without a special case.

use super::SystemBus;
use crate::peripherals::components::declarative_analog_mux::{decode, AnalogMux, MuxPad};

impl SystemBus {
    /// Attach a mux and push its first routing onto the downstream channel.
    pub(crate) fn attach_analog_mux(&mut self, mux: AnalogMux) -> anyhow::Result<()> {
        if self.analog_muxes.iter().any(|m| m.id == mux.id) {
            anyhow::bail!("analog mux '{}' is declared twice", mux.id);
        }
        let connection = mux.connection.clone();
        let channel = mux.channel;
        let id = mux.id.clone();
        self.analog_muxes.push(mux);
        let idx = self.analog_muxes.len() - 1;
        if !self.route_analog_mux(idx, true) {
            self.analog_muxes.pop();
            anyhow::bail!(
                "analog mux '{id}': connection '{connection}' has no ADC channel {channel} \
                 (name an ADC peripheral, or a mux declared before this one)"
            );
        }
        Ok(())
    }

    /// The mux declared as `id`, for readback.
    pub fn analog_mux(&self, id: &str) -> Option<&AnalogMux> {
        self.analog_muxes.iter().find(|m| m.id == id)
    }

    /// Re-read the pads of mux `idx` and drive Z onto the downstream channel
    /// when the routing changed, or always when `force`. Returns whether the
    /// downstream channel accepted the level (true when nothing was pushed).
    pub(crate) fn route_analog_mux(&mut self, idx: usize, force: bool) -> bool {
        let pad = |bus: &SystemBus, p: &MuxPad| {
            // A pad the model cannot report reads low; attach refuses such a
            // pad, so this default is never reached for a wired mux.
            bus.peripherals
                .get(p.peripheral)
                .and_then(|per| per.dev.read_gpio_pad(p.bit))
                .unwrap_or(false)
        };
        let mux = &self.analog_muxes[idx];
        let select: Vec<bool> = mux.select.iter().map(|p| pad(self, p)).collect();
        let enabled = mux
            .enable
            .as_ref()
            .is_none_or(|(p, active)| active.asserted(pad(self, p)));
        let routed = decode(&select, enabled);
        let (mv, changed) = self.analog_muxes[idx].route(routed);
        if !(force || changed) {
            return true;
        }
        let (connection, channel) = {
            let m = &self.analog_muxes[idx];
            (m.connection.clone(), m.channel)
        };
        self.seed_adc_channel(&connection, channel, mv)
    }

    /// MMIO write hook: re-route every mux with a select or enable pad on
    /// peripheral `idx`. Outlined so a bus without muxes pays one length check.
    #[inline]
    pub(crate) fn route_analog_muxes_on_write(&mut self, idx: usize) {
        if self.analog_muxes.is_empty() {
            return;
        }
        self.route_analog_muxes_on_write_cold(idx);
    }

    #[inline(never)]
    fn route_analog_muxes_on_write_cold(&mut self, idx: usize) {
        for m in 0..self.analog_muxes.len() {
            if self.analog_muxes[m].watches(idx) {
                self.route_analog_mux(m, false);
            }
        }
    }

    /// `seed_adc_channel` arm for a mux: set Y`channel` of the mux named
    /// `connection` and push Z downstream. `None` when no mux has that id.
    pub(crate) fn seed_analog_mux_input(
        &mut self,
        connection: &str,
        channel: u8,
        millivolts: u16,
    ) -> Option<bool> {
        let idx = self.analog_muxes.iter().position(|m| m.id == connection)?;
        if !self.analog_muxes[idx].set_input_mv(channel, millivolts) {
            return Some(false);
        }
        // Push only when this input is the routed one; the others do not
        // reach Z.
        if self.analog_muxes[idx].selected() == Some(channel as usize) {
            return Some(self.route_analog_mux(idx, true));
        }
        Some(true)
    }
}
