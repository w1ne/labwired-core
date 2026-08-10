// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use std::collections::HashMap;

/// A simple stub peripheral that returns fixed values on read.
#[derive(Debug, serde::Serialize)]
pub struct StubPeripheral {
    pub values: HashMap<u64, u32>, // mapping offset to value
    pub default_val: u32,
}

impl StubPeripheral {
    pub fn new(default_val: u32) -> Self {
        Self {
            values: HashMap::new(),
            default_val,
        }
    }
}

impl crate::Peripheral for StubPeripheral {
    fn read(&self, offset: u64) -> SimResult<u8> {
        // Simple byte mapping
        let reg_offset = offset & !3;
        let byte_offset = (offset % 4) as u32;
        let val = self
            .values
            .get(&reg_offset)
            .cloned()
            .unwrap_or(self.default_val);
        Ok(((val >> (byte_offset * 8)) & 0xFF) as u8)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        // Ignores writes for now
        Ok(())
    }

    /// A stub never overrides `tick()` — it is a literal no-op on the legacy
    /// walk for every state, so it can never be the reason a bus keeps the walk.
    fn needs_legacy_walk(&self) -> bool {
        false
    }

    /// ...and for the same reason it does not belong in the per-cycle walk SET
    /// either. `needs_legacy_walk()` above only says a stub cannot *block*
    /// walk-deletion; this says it should not be *visited*.
    ///
    /// Byte-identical by construction: `StubPeripheral` does not override
    /// `tick()`/`tick_elapsed()`, so every visit returned a default
    /// `PeripheralTickResult` and changed nothing. Skipping it removes only the
    /// dispatch, never an effect. `ticks_remaining` is reset on MMIO access
    /// anyway, and a stub has no state that evolves between accesses.
    ///
    /// This matters because chip profiles legitimately carry many stub windows
    /// (trim/config blocks that firmware pokes but never polls): the nRF54L15
    /// has 9, and visiting them dominated its per-cycle cost — the walk was
    /// slower for peripherals that do nothing than for the ones that do.
    fn legacy_tick_active(&self) -> bool {
        false
    }

    fn snapshot(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    /// Census hook — **measurement only**, and only when the `silent-census`
    /// feature is compiled in (off by default, see `crate::census`).
    ///
    /// The post-construction stub sweep must answer "is this `dyn Peripheral`
    /// a stub?" by *type*, not by the entry's `name`: the post-factory
    /// replacement passes (`system::cortex_m::configure_cortex_m`,
    /// `SystemBus::replace_or_add_peripheral`) rewrite names, so a name proves
    /// nothing about what the machine is actually holding. Exposing the
    /// concrete type here makes `dyn Any::is::<StubPeripheral>()` an exact
    /// `TypeId` test.
    ///
    /// Feature-gated because it is the census's only production-type change.
    /// With the feature off this override does not exist and `StubPeripheral`
    /// is unchanged. With it on, every existing `Peripheral::as_any` consumer
    /// immediately `downcast_ref`s to a concrete model type that a stub is not,
    /// so `Some(stub)` is indistinguishable from the previous `None`.
    #[cfg(feature = "silent-census")]
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
