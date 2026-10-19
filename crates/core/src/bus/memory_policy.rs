// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use crate::SimResult;
use serde::{Deserialize, Serialize};

pub use labwired_config::CpuWritePolicy;

/// One recorded policy violation (the first per region is kept in full).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionViolation {
    pub addr: u64,
    pub cycle: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pc: Option<u32>,
}

/// Per-region stats for `result.json`'s `memory_policy` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionPolicyStats {
    pub mode: CpuWritePolicy,
    pub violations: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first: Option<RegionViolation>,
}

/// What `labwired test` writes under `result.json.memory_policy`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryPolicyReport {
    pub flash: RegionPolicyStats,
    pub ram: RegionPolicyStats,
}

/// The memory layout the gate checks a store against (built once per store
/// by `SystemBus::memory_policy_gate_store`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryPolicyLayout {
    pub ram_base: u64,
    pub ram_len: u64,
    pub flash_base: u64,
    pub flash_len: u64,
    pub flash_alias_active: bool,
}

/// Live policy state carried by [`crate::bus::SystemBus`].
#[derive(Debug, Default)]
pub struct MemoryPolicyState {
    pub flash: CpuWritePolicy,
    pub ram: CpuWritePolicy,
    pub violations_flash: u64,
    pub violations_ram: u64,
    pub first_flash: Option<RegionViolation>,
    pub first_ram: Option<RegionViolation>,
    pub flash_warning_emitted: bool,
    pub ram_warning_emitted: bool,
    pub last_cpu_pc: u32,
    pub cycle: u64,
}

impl MemoryPolicyState {
    /// The policies a chip declares — used by `from_config` and unit tests.
    pub fn for_chip(chip: &labwired_config::ChipDescriptor) -> Self {
        Self {
            flash: chip.flash.cpu_writes,
            ram: chip.ram.cpu_writes,
            ..Default::default()
        }
    }

    #[inline]
    pub fn active(&self) -> bool {
        !self.flash.is_allow() || !self.ram.is_allow()
    }

    pub fn note_pc(&mut self, pc: u32) {
        self.last_cpu_pc = pc;
    }

    /// True when a CPU store at `addr` must skip the direct-to-RAM fast path
    /// (the policy gate in `accessors.rs` then handles it).
    #[inline]
    pub fn ram_blocked(&self, addr: u64, ram_base: u64, ram_len: u64) -> bool {
        self.ram != CpuWritePolicy::Allow
            && addr >= ram_base
            && addr < ram_base.saturating_add(ram_len)
    }

    fn record(
        region_first: &mut Option<RegionViolation>,
        count: &mut u64,
        addr: u64,
        pc: u32,
        cycle: u64,
    ) {
        *count += 1;
        if region_first.is_none() {
            *region_first = Some(RegionViolation {
                addr,
                cycle,
                pc: Some(pc),
            });
        }
    }

    /// The gate `bus/accessors.rs` calls before any store that would land in
    /// the flash region, its boot alias, or the ram region.
    /// `layout.flash_alias_active` is `bus.flash_boot_alias && flash_base != 0`.
    /// Returns `None` when the store may proceed, or `Some(Err(..))` when it
    /// must raise a precise data fault.
    pub fn gate(&mut self, addr: u64, layout: MemoryPolicyLayout) -> Option<SimResult<()>> {
        let in_flash = (addr >= layout.flash_base
            && addr < layout.flash_base.saturating_add(layout.flash_len))
            || (layout.flash_alias_active && addr < layout.flash_len);
        let in_ram =
            addr >= layout.ram_base && addr < layout.ram_base.saturating_add(layout.ram_len);
        if !in_flash && !in_ram {
            return None;
        }
        let pc = self.last_cpu_pc;
        let cycle = self.cycle;
        if in_flash {
            match self.flash {
                CpuWritePolicy::Allow => {}
                CpuWritePolicy::Warn => {
                    Self::record(
                        &mut self.first_flash,
                        &mut self.violations_flash,
                        addr,
                        pc,
                        cycle,
                    );
                    if !self.flash_warning_emitted {
                        self.flash_warning_emitted = true;
                        tracing::warn!(
                            addr = format_args!("{addr:#010X}"),
                            pc = format_args!("{pc:#010X}"),
                            "CPU store into flash with cpu_writes: warn — store committed"
                        );
                    }
                }
                CpuWritePolicy::Fault => {
                    Self::record(
                        &mut self.first_flash,
                        &mut self.violations_flash,
                        addr,
                        pc,
                        cycle,
                    );
                    return Some(Err(crate::SimulationError::MemoryViolation(addr)));
                }
            }
        }
        if in_ram {
            match self.ram {
                CpuWritePolicy::Allow => {}
                CpuWritePolicy::Warn => {
                    Self::record(
                        &mut self.first_ram,
                        &mut self.violations_ram,
                        addr,
                        pc,
                        cycle,
                    );
                    if !self.ram_warning_emitted {
                        self.ram_warning_emitted = true;
                        tracing::warn!(
                            addr = format_args!("{addr:#010X}"),
                            pc = format_args!("{pc:#010X}"),
                            "CPU store into ram with cpu_writes: warn — store committed"
                        );
                    }
                }
                CpuWritePolicy::Fault => {
                    Self::record(
                        &mut self.first_ram,
                        &mut self.violations_ram,
                        addr,
                        pc,
                        cycle,
                    );
                    return Some(Err(crate::SimulationError::MemoryViolation(addr)));
                }
            }
        }
        None
    }

    pub fn report(&self) -> Option<MemoryPolicyReport> {
        if !self.active() {
            return None;
        }
        Some(MemoryPolicyReport {
            flash: RegionPolicyStats {
                mode: self.flash,
                violations: self.violations_flash,
                first: self.first_flash.clone(),
            },
            ram: RegionPolicyStats {
                mode: self.ram,
                violations: self.violations_ram,
                first: self.first_ram.clone(),
            },
        })
    }
}

impl crate::bus::SystemBus {
    /// Policy gate for one CPU store. See [`MemoryPolicyState::gate`].
    #[inline]
    pub(crate) fn memory_policy_gate_store(&mut self, addr: u64) -> Option<SimResult<()>> {
        if self.memory_policy_bypass || !self.memory_policy.active() {
            return None;
        }
        self.memory_policy.gate(
            addr,
            MemoryPolicyLayout {
                ram_base: self.ram.base_addr,
                ram_len: self.ram.data.len() as u64,
                flash_base: self.flash.base_addr,
                flash_len: self.flash.data.len() as u64,
                flash_alias_active: self.flash_boot_alias && self.flash.base_addr != 0,
            },
        )
    }

    /// Runs `f` with the CPU-write policy gate bypassed (non-CPU writes: DMA
    /// engines, the loader's peripheral-backed byte walk, and boot seeds).
    /// Nesting-safe; restores the previous state. Peripheral-internal DMA
    /// models writing through `&mut dyn Bus` are a recorded deferral.
    pub(crate) fn with_memory_policy_bypass<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let saved = self.memory_policy_bypass;
        self.memory_policy_bypass = true;
        let r = f(self);
        self.memory_policy_bypass = saved;
        r
    }

    /// True when a non-default RAM `cpu_writes` policy is in force — the
    /// interpreter's store gate owns every RAM store; consumers that write
    /// `bus.ram` directly (JIT lanes) must refuse instead.
    #[inline]
    pub(crate) fn ram_cpu_writes_gated(&self) -> bool {
        self.memory_policy.ram != crate::bus::CpuWritePolicy::Allow
    }

    /// True when a CPU store at `addr` must skip the direct-RAM fast paths in
    /// the Cortex-M core (policy is non-default for RAM).
    #[inline(always)]
    pub(crate) fn ram_cpu_writes_blocked(&self, addr: u64) -> bool {
        self.ram_cpu_writes_gated()
            && self
                .memory_policy
                .ram_blocked(addr, self.ram.base_addr, self.ram.data.len() as u64)
    }

    /// The `result.json` block; `None` on default-allow buses.
    pub fn memory_policy_report(&self) -> Option<MemoryPolicyReport> {
        self.memory_policy.report()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_flash(mode: CpuWritePolicy) -> MemoryPolicyState {
        MemoryPolicyState {
            flash: mode,
            ..Default::default()
        }
    }

    fn layout() -> MemoryPolicyLayout {
        MemoryPolicyLayout {
            ram_base: 0x2000_0000,
            ram_len: 0x1000,
            flash_base: 0x0800_0000,
            flash_len: 0x1_0000,
            flash_alias_active: false,
        }
    }

    #[test]
    fn allow_does_not_gate() {
        let mut s = state_flash(CpuWritePolicy::Allow);
        assert!(s.gate(0x0800_1000, layout()).is_none());
        assert_eq!(s.violations_flash, 0);
    }

    #[test]
    fn fault_gates_and_records_first() {
        let mut s = state_flash(CpuWritePolicy::Fault);
        s.note_pc(0x0800_006C);
        let r = s.gate(0x0800_1000, layout());
        assert!(
            matches!(
                r,
                Some(Err(crate::SimulationError::MemoryViolation(0x0800_1000)))
            ),
            "{r:?}"
        );
        assert!(s.gate(0x0800_1004, layout()).is_some());
        assert!(s.gate(0x0801_0000, layout()).is_none());
        assert_eq!(s.violations_flash, 2);
        let first = s.first_flash.clone().unwrap();
        assert_eq!(first.addr, 0x0800_1000);
        assert_eq!(first.pc, Some(0x0800_006C));
    }

    #[test]
    fn warn_commits_and_dedups_the_log() {
        let mut s = state_flash(CpuWritePolicy::Warn);
        assert!(s.gate(0x0800_1000, layout()).is_none());
        assert!(s.gate(0x0800_1004, layout()).is_none());
        assert_eq!(s.violations_flash, 2);
        assert!(s.flash_warning_emitted);
    }

    #[test]
    fn ram_fault_gates_ram_only() {
        let mut s = MemoryPolicyState {
            ram: CpuWritePolicy::Fault,
            ..Default::default()
        };
        assert!(s.gate(0x2000_0010, layout()).is_some());
        assert!(s.gate(0x0800_0010, layout()).is_none());
        assert_eq!(s.violations_ram, 1);
    }

    #[test]
    fn boot_alias_writes_are_gated_as_flash() {
        let mut s = state_flash(CpuWritePolicy::Fault);
        let alias_layout = MemoryPolicyLayout {
            flash_alias_active: true,
            ..layout()
        };
        assert!(s.gate(0x40, alias_layout).is_some());
        assert_eq!(s.violations_flash, 1);
    }

    #[test]
    fn report_is_none_until_a_policy_is_set() {
        let s = MemoryPolicyState::default();
        assert!(s.report().is_none());
        let s = state_flash(CpuWritePolicy::Fault);
        assert!(s.report().is_some());
    }

    #[test]
    fn from_config_copies_policies() {
        use crate::tests::bus_trace_one_home::dummy_manifest;

        let chip_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../configs/chips/stm32f103.yaml");
        let mut chip = labwired_config::ChipDescriptor::from_file(&chip_path).unwrap();
        chip.flash.cpu_writes = CpuWritePolicy::Fault;
        chip.ram.cpu_writes = CpuWritePolicy::Warn;
        let path = chip_path.to_string_lossy();
        let bus = crate::bus::SystemBus::from_config(&chip, &dummy_manifest(&path)).unwrap();
        assert_eq!(bus.memory_policy.flash, CpuWritePolicy::Fault);
        assert_eq!(bus.memory_policy.ram, CpuWritePolicy::Warn);
        assert!(bus.memory_policy.active());
    }
}
