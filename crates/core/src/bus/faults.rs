// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Fault-injection helpers on [`SystemBus`] (test / diagnostics only).

use super::SystemBus;

impl SystemBus {
    /// Disable RCC clock-gating for measurement/diagnostic tooling: while set,
    /// `is_peripheral_clocked` returns `true` for every peripheral, so a gated
    /// peripheral's registers stay accessible regardless of the RCC enable bits.
    ///
    /// This is a measurement hook, NOT something the runtime ever calls. The
    /// runtime requires firmware to clock a peripheral before use, and a gated-
    /// but-unclocked peripheral must read 0 / ignore writes (silicon fidelity).
    /// But tooling that measures *register modeling* (the SVD coverage probe)
    /// needs to ask "is this register modelled" — a property of the device —
    /// independent of whether its clock happens to be on. A flag is used rather
    /// than pre-setting the RCC enable bits because the coverage probe itself
    /// writes 0 to every register, including the RCC enable registers, which
    /// would re-gate any peripheral probed after the RCC.
    pub fn set_clock_gating_bypass(&mut self, bypass: bool) {
        self.clock_gating_bypass = bypass;
    }

    /// Inject a `missing_clock` fault: force `peripheral` to behave as if its
    /// clock is never enabled, so every CPU access to it is suppressed (reads
    /// return 0, writes are dropped) exactly like an unclocked peripheral on
    /// silicon. Returns an error if the peripheral is absent. Whether the fault
    /// actually fired (an access was suppressed) is read back with
    /// [`Self::missing_clock_suppressed`] after the run.
    pub fn inject_missing_clock(&mut self, peripheral: &str) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        self.fault_unclocked
            .entry(idx)
            .or_insert_with(|| std::sync::atomic::AtomicU64::new(0));
        Ok(())
    }

    /// Number of accesses suppressed by a `missing_clock` fault on `peripheral`
    /// (0 if not faulted or never accessed). `> 0` means the fault fired.
    pub fn missing_clock_suppressed(&self, peripheral: &str) -> u64 {
        let Some(idx) = self.find_peripheral_index_by_name(peripheral) else {
            return 0;
        };
        self.fault_unclocked
            .get(&idx)
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            .unwrap_or(0)
    }

    /// Inject a `stuck_at_bit` fault: hold `bit` of `register` on the
    /// declarative peripheral `peripheral` at `level` (0/1) — the CPU always
    /// reads that level regardless of writes. Returns an error if the peripheral
    /// or register is absent, the bit is out of range, or the peripheral is not
    /// a declarative `GenericPeripheral`.
    pub fn inject_stuck_bit(
        &mut self,
        peripheral: &str,
        register: &str,
        bit: u8,
        level: u8,
    ) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not introspectable for faults"))?;
        let generic = any
            .downcast_mut::<crate::peripherals::declarative::GenericPeripheral>()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not a declarative peripheral"))?;
        if !generic.force_stuck_bit(register, bit, level) {
            return Err(format!(
                "register '{register}' bit {bit} invalid on peripheral '{peripheral}'"
            ));
        }
        Ok(())
    }

    /// Inject a `wrong_reset_value` fault: force `register` on the declarative
    /// peripheral `peripheral` to `value`. Returns an error (never a silent
    /// no-op) if the peripheral or register is absent, or the peripheral is not
    /// a declarative `GenericPeripheral`.
    pub fn inject_wrong_reset_value(
        &mut self,
        peripheral: &str,
        register: &str,
        value: u32,
    ) -> Result<(), String> {
        let idx = self
            .find_peripheral_index_by_name(peripheral)
            .ok_or_else(|| format!("fault target peripheral '{peripheral}' not found"))?;
        let any = self.peripherals[idx]
            .dev
            .as_any_mut()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not introspectable for faults"))?;
        let generic = any
            .downcast_mut::<crate::peripherals::declarative::GenericPeripheral>()
            .ok_or_else(|| format!("peripheral '{peripheral}' is not a declarative peripheral"))?;
        if !generic.force_register_value(register, value) {
            return Err(format!(
                "register '{register}' not found on peripheral '{peripheral}'"
            ));
        }
        Ok(())
    }
}
