// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The resolved peripheral memory map, handed to peripheral models at build
//! time so a model never has to hardcode a sibling peripheral's base address.
//!
//! # Why this exists
//!
//! A peripheral's **base address** is a property of the chip's memory map,
//! which the chip YAML owns. A **register offset within** that peripheral
//! (`+0x510`) is a silicon fact of the peripheral itself and legitimately
//! belongs with the model. The line is drawn there, deliberately: mass-moving
//! offsets into YAML would be worse than the disease.
//!
//! Most models only ever need their *own* base, and the peripheral factories
//! already read that straight off `p_cfg.base_address`. The gap this type
//! closes is the model that must address a **different** peripheral — nRF52
//! GPIOTE drives pads that live in the GPIO ports' windows, so it emits MMIO
//! writes at `gpio0`/`gpio1` bases it does not own.
//!
//! # The bug that motivated it
//!
//! `Nrf52Gpiote` hardcoded `GPIO1_BASE = 0x5000_0300`, the real-silicon P1
//! base. The nRF52840 chip YAML deliberately remaps `gpio1` to `0x5000_1000`,
//! because Nordic's real P1 base sits *inside* GPIO0's 4 KB window and the two
//! would collide on a flat bus. So every port-1 GPIOTE task wrote
//! `0x5000_0810` — a perfectly valid address, inside **gpio0's** window, where
//! it was swallowed. No error, no warning, no fault: a wrong address is still
//! a valid address. That is what makes this class silent.
//!
//! Reading the base from here instead means the model cannot disagree with the
//! YAML, because it has no second copy of the fact to disagree with.

use labwired_config::PeripheralConfig;

/// Read-only view of the peripheral memory map a bus is being built from —
/// the chip descriptor's peripherals with the system manifest's overrides
/// already merged in, i.e. exactly the addresses the bus will route.
#[derive(Copy, Clone, Debug)]
pub struct ChipMap<'a> {
    entries: &'a [PeripheralConfig],
}

impl<'a> ChipMap<'a> {
    /// Wrap the merged peripheral list. Built once in
    /// [`crate::bus::SystemBus::from_config`] and passed down to the family
    /// factories.
    pub fn new(entries: &'a [PeripheralConfig]) -> Self {
        Self { entries }
    }

    /// An empty map, for unit tests and callers that construct a model outside
    /// a bus. Every lookup misses, so callers must supply their own fallback
    /// and cannot silently get a wrong address from a stub.
    pub fn empty() -> Self {
        Self { entries: &[] }
    }

    /// Base address of the peripheral with this exact `id`, as declared.
    /// `None` when the chip does not declare it — callers decide whether that
    /// is a fallback or a hard error, since a chip may legitimately lack a
    /// peripheral (nRF52832 has no P1 at all).
    pub fn base_of(&self, id: &str) -> Option<u64> {
        self.entries
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.base_address)
    }

    /// Every declared peripheral id, in map order. Used by the sibling-lookup
    /// diagnostics and by tests.
    pub fn ids(&self) -> impl Iterator<Item = &'a str> {
        self.entries.iter().map(|p| p.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(id: &str, base: u64) -> PeripheralConfig {
        PeripheralConfig {
            id: id.to_string(),
            r#type: "gpio".to_string(),
            base_address: base,
            size: None,
            irq: None,
            clock: None,
            config: Default::default(),
        }
    }

    #[test]
    fn base_of_returns_the_declared_address() {
        let entries = vec![cfg("gpio0", 0x5000_0000), cfg("gpio1", 0x5000_1000)];
        let map = ChipMap::new(&entries);
        assert_eq!(map.base_of("gpio0"), Some(0x5000_0000));
        // The remapped port-1 base, NOT Nordic's raw-silicon 0x5000_0300.
        assert_eq!(map.base_of("gpio1"), Some(0x5000_1000));
    }

    #[test]
    fn base_of_misses_for_an_undeclared_peripheral() {
        let entries = vec![cfg("gpio0", 0x5000_0000)];
        assert_eq!(ChipMap::new(&entries).base_of("gpio1"), None);
    }

    #[test]
    fn empty_map_misses_everything() {
        assert_eq!(ChipMap::empty().base_of("gpio0"), None);
        assert_eq!(ChipMap::empty().ids().count(), 0);
    }
}
