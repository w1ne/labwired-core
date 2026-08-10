// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Bus wiring for the ESP32-C3 permission-control unit (PMS).
//!
//! The model itself lives in [`crate::peripherals::esp32c3::pms`]; this file is
//! the glue that makes it observable the way silicon is:
//!
//! 1. **Configuration** comes from the declarative `SENSITIVE` peripheral —
//!    the ONE place PMS registers are stored. Every write into its PMS span
//!    re-derives the cached permission map, so there is no second source of
//!    truth to drift.
//! 2. **Detection** hangs off the existing store path in `accessors.rs` and the
//!    instruction-fetch path in the RISC-V interpreter.
//! 3. **Surfacing** goes back through the same `SENSITIVE` registers (so
//!    `esp_memprot_get_violate_addr/world/operation` and the inspect wall read
//!    real values) and through the existing ESP32-C3 interrupt matrix, which
//!    routes `ETS_CORE0_{I,D}RAM0_PMS_INTR_SOURCE` to whatever CPU line
//!    firmware mapped it to — `ETS_MEMPROT_ERR_INUM` (26) under IDF, whose
//!    vector slot is `_panic_handler`.
//! 4. **Fail-loud fallback.** If a violation cannot be delivered as an
//!    interrupt (the matrix does not route it, or firmware masked the line),
//!    the access still returns `SimulationError::MemoryViolation` rather than
//!    being silently permitted — a violation is never swallowed.

use super::SystemBus;
use crate::peripherals::esp32c3::pms::{reg, Esp32C3Pms, PmsOp, PmsPort, PmsViolation};

/// Base address of the C3 `SENSITIVE` block. Matching on the base (not just the
/// name) is what keeps this inert on every other SoC.
const SENSITIVE_BASE: u64 = 0x600C_1000;

/// What happened to an access the PMS examined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PmsOutcome {
    /// Permitted — proceed exactly as before.
    Allowed,
    /// Blocked, and the violation was raised as an interrupt-matrix source
    /// that the matrix routes to an enabled CPU line. Firmware's own handler
    /// (IDF's panic handler) will run.
    BlockedFaultRaised,
    /// Blocked, but nothing can deliver it — surface an engine fault instead
    /// of pretending the access happened.
    BlockedUndeliverable,
}

impl SystemBus {
    /// Locate the C3 `SENSITIVE` block and (re)build the PMS model from its
    /// current register contents. Called from `rebuild_peripheral_ranges`
    /// alongside the interrupt-controller cache rebuild.
    pub(crate) fn rebuild_esp32c3_pms(&mut self) {
        self.esp32c3_sensitive_idx = self
            .peripherals
            .iter()
            .position(|p| p.name == "sensitive" && p.base == SENSITIVE_BASE);
        if self.esp32c3_sensitive_idx.is_none() {
            self.esp32c3_pms = None;
            self.esp32c3_pms_armed = false;
            return;
        }
        // Preserve any latched violation across a range rebuild (attaching a
        // device must not clear a fault firmware has not acknowledged yet).
        let mut pms = self.esp32c3_pms.take().unwrap_or_default();
        self.refresh_pms_from_registers(&mut pms);
        self.esp32c3_pms_armed = pms.armed();
        self.esp32c3_pms = Some(pms);
    }

    fn refresh_pms_from_registers(&self, pms: &mut Esp32C3Pms) {
        let Some(idx) = self.esp32c3_sensitive_idx else {
            return;
        };
        pms.refresh(|off| self.read_cached_declarative_u32(idx, off));
    }

    /// Write choke: a CPU write landed on peripheral `idx` at `offset`. When
    /// that is the `SENSITIVE` PMS span, re-derive the permission map — and
    /// honour a `VIOLATE_CLR` pulse by dropping the latch, its status words and
    /// its interrupt-matrix source.
    pub(crate) fn sync_esp32c3_pms_write(&mut self, idx: usize, offset: u64) {
        if Some(idx) != self.esp32c3_sensitive_idx {
            return;
        }
        let aligned = offset & !3;
        if !reg::PMS_SPAN.contains(&aligned) {
            return;
        }
        let Some(mut pms) = self.esp32c3_pms.take() else {
            return;
        };
        // Write locks first. Production firmware runs with
        // CONFIG_ESP_SYSTEM_MEMPROT_FEATURE_LOCK=y, so once startup has locked
        // the split lines / permissions / monitors, silicon IGNORES every later
        // write to them — protection cannot be turned back off. Undo the store
        // instead of letting the twin quietly go blind again.
        let written = self.read_cached_declarative_u32(idx, aligned).unwrap_or(0);
        if let (false, crate::peripherals::esp32c3::pms::PmsWriteVerdict::Reject(restore)) =
            (self.pms_write_bypass, pms.write_verdict(aligned, written))
        {
            self.poke_sensitive(idx, aligned, restore);
            self.esp32c3_pms = Some(pms);
            return;
        }
        // MONITOR_1 carries VIOLATE_CLR / VIOLATE_EN and is the only register
        // whose write has an effect beyond reconfiguration.
        let mut cleared = None;
        for (off, port) in [
            (reg::IRAM0_PMS_MONITOR_1, PmsPort::Iram0),
            (reg::DRAM0_PMS_MONITOR_1, PmsPort::Dram0),
        ] {
            if aligned == off {
                let value = self.read_cached_declarative_u32(idx, off).unwrap_or(0);
                if pms.apply_monitor_ctrl(port, value) {
                    cleared = Some(port);
                }
            }
        }
        // Publish the cleared status BEFORE re-deriving, so the refresh folds
        // the new status words into the shadow rather than leaving it stale.
        if let Some(port) = cleared {
            for (o, v) in pms.status_words(port) {
                self.poke_sensitive(idx, o, v);
            }
        }
        self.refresh_pms_from_registers(&mut pms);
        self.esp32c3_pms_armed = pms.armed();
        self.esp32c3_pms = Some(pms);
        if self.esp32c3_irq_routing {
            self.recompute_esp32c3_irq_lines();
        }
    }

    /// Store gate for the three `write_u*` entry points.
    ///
    /// `None` means "not our business, carry on" — the case for every bus
    /// without a C3 PMS and for every C3 whose firmware has not narrowed a
    /// region, so the cost is a single predictable branch. `Some(Ok(()))` means
    /// the store was BLOCKED and a fault was raised the firmware will take;
    /// silicon drops the write, so the twin must too. `Some(Err(..))` is the
    /// fail-loud path for a violation nothing can deliver.
    ///
    /// Placed at the head of `write_u16`/`write_u32` as well as `write_u8` so a
    /// wide store that would otherwise decompose into four `write_u8` calls
    /// (the C3's IRAM lives in `extra_mem`, which the wide paths miss) raises
    /// exactly ONE violation, at the address the instruction named.
    #[inline]
    pub(crate) fn esp32c3_pms_gate_store(&mut self, addr: u64) -> Option<crate::SimResult<()>> {
        if !self.esp32c3_pms_armed {
            return None;
        }
        match self.esp32c3_pms_check_store(addr) {
            PmsOutcome::Allowed => None,
            PmsOutcome::BlockedFaultRaised => Some(Ok(())),
            PmsOutcome::BlockedUndeliverable => {
                Some(Err(crate::SimulationError::MemoryViolation(addr)))
            }
        }
    }

    /// Check a CPU **store** against the PMS. Only ever reached when
    /// `esp32c3_pms_armed` is set, so unprotected firmware pays one bool test.
    #[cold]
    pub(crate) fn esp32c3_pms_check_store(&mut self, addr: u64) -> PmsOutcome {
        self.esp32c3_pms_check(addr, PmsOp::Store)
    }

    /// Check an instruction **fetch** against the PMS.
    #[cold]
    pub(crate) fn esp32c3_pms_check_fetch(&mut self, addr: u64) -> PmsOutcome {
        self.esp32c3_pms_check(addr, PmsOp::Fetch)
    }

    fn esp32c3_pms_check(&mut self, addr: u64, op: PmsOp) -> PmsOutcome {
        if addr > u32::MAX as u64 {
            return PmsOutcome::Allowed;
        }
        let Some(pms) = self.esp32c3_pms.as_ref() else {
            return PmsOutcome::Allowed;
        };
        let Some(violation) = pms.check(addr as u32, op) else {
            return PmsOutcome::Allowed;
        };
        self.raise_esp32c3_pms_violation(violation)
    }

    /// Latch a violation, publish its status registers, assert its
    /// interrupt-matrix source and re-route the CPU lines.
    fn raise_esp32c3_pms_violation(&mut self, violation: PmsViolation) -> PmsOutcome {
        let Some(mut pms) = self.esp32c3_pms.take() else {
            return PmsOutcome::Allowed;
        };
        let newly_latched = pms.latch(violation);
        if newly_latched {
            if let Some(idx) = self.esp32c3_sensitive_idx {
                for (o, v) in pms.status_words(violation.port) {
                    self.poke_sensitive(idx, o, v);
                    pms.set_shadow(o, v);
                }
            }
        }
        self.esp32c3_pms = Some(pms);

        if self.esp32c3_irq_routing {
            self.recompute_esp32c3_irq_lines();
            let src = violation.port.intr_source();
            // Did the matrix actually route this source to an ENABLED line?
            // `recompute_esp32c3_irq_lines` already folded the PMS sources in,
            // so a set bit here means firmware will take the trap.
            if self.esp32c3_pms_line_for_source(src).is_some() {
                return PmsOutcome::BlockedFaultRaised;
            }
        }
        PmsOutcome::BlockedUndeliverable
    }

    /// The CPU line `source` is routed to, if the interrupt matrix maps it to
    /// an enabled line that currently passes the priority threshold.
    fn esp32c3_pms_line_for_source(&self, source: u32) -> Option<u8> {
        let cache = self.esp32c3_irq_cache.as_ref()?;
        let line = *cache.source_line.get(source as usize)?;
        if line == 0 || (cache.int_enable & (1u32 << line)) == 0 {
            return None;
        }
        let pri = cache.line_pri.get(line as usize).copied().unwrap_or(0);
        (pri >= cache.int_thresh).then_some(line)
    }

    /// Interrupt-matrix sources currently asserted by latched PMS violations.
    /// Folded into `recompute_esp32c3_irq_lines` next to the walk-emitted and
    /// scheduler-emitted source bitmaps. Level semantics: a source stays
    /// asserted until firmware pulses `VIOLATE_CLR`.
    pub(crate) fn esp32c3_pms_sources(&self) -> [u64; 2] {
        self.esp32c3_pms
            .as_ref()
            .map(|p| p.asserted_sources())
            .unwrap_or([0; 2])
    }

    fn poke_sensitive(&self, idx: usize, offset: u64, value: u32) {
        if let Some(generic) = self.peripherals.get(idx).and_then(|p| {
            p.dev.as_any().and_then(|a| {
                a.downcast_ref::<crate::peripherals::declarative::GenericPeripheral>()
            })
        }) {
            generic.poke_u32_raw(offset, value);
        }
    }

    /// Number of ESP32-C3 PMS violations detected on this bus since reset.
    /// `0` on every bus without a C3 `SENSITIVE` block. This is the
    /// engine-visible counter tests and the inspect wall read; the *content*
    /// of the fault lives in the `SENSITIVE` status registers, exactly where
    /// firmware reads it.
    pub fn esp32c3_pms_violations(&self) -> u64 {
        self.esp32c3_pms.as_ref().map(|p| p.violations).unwrap_or(0)
    }

    /// The violation currently latched on `port` — the twin's equivalent of
    /// `esp_mprot_get_violate_addr/world/operation`.
    pub fn esp32c3_pms_latched(&self, port: PmsPort) -> Option<PmsViolation> {
        self.esp32c3_pms.as_ref().and_then(|p| p.latched(port))
    }

    /// True while the PMS is configured such that some access could be blocked.
    pub fn esp32c3_pms_armed(&self) -> bool {
        self.esp32c3_pms_armed
    }
}
