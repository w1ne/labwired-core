// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SVD register-coverage driver: parse the SVD, build the wired model,
//! probe every peripheral's registers, and emit a coverage matrix.

use std::collections::BTreeMap;
use std::path::PathBuf;

use labwired_core::coverage::{probe_peripheral, Access, ProbeReg, ProbeTarget, RegStatus};
use labwired_core::Bus; // BusTarget probes via the single Bus-trait accessor
use serde::{Deserialize, Serialize};
use svd_parser::svd::{Peripheral, Register, RegisterCluster};

struct SvdPeripheral {
    name: String,
    base: u64,
    registers: Vec<ProbeReg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeripheralCoverage {
    pub modelled: usize,
    pub indeterminate: usize,
    pub unmodelled: usize,
    pub total: usize,
    pub unmodelled_regs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoverageMatrix(pub BTreeMap<String, PeripheralCoverage>);

/// Discover the ESP32-S3 SVD: `LABWIRED_ESP32S3_SVD` override, else PlatformIO,
/// else the vendored copy under `tests/fixtures/svd/` in the workspace root.
pub fn discover_svd() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_ESP32S3_SVD") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let pio = PathBuf::from(format!(
        "{home}/.platformio/platforms/espressif32/misc/svd/esp32s3.svd"
    ));
    if pio.is_file() {
        return Some(pio);
    }
    // Fallback: vendored copy committed under tests/fixtures/svd/ so CI runs
    // the ratchet without requiring PlatformIO or the Xtensa toolchain.
    let vendored = crate::tier1::workspace_root().join("tests/fixtures/svd/esp32s3.svd");
    vendored.is_file().then_some(vendored)
}

/// Flatten a `RegisterCluster` into `ProbeReg` entries, mirroring svd-ingestor's
/// `process_register_cluster` pattern which handles clusters and register arrays.
fn collect_probe_regs(cluster: &RegisterCluster, parent_offset: u64, out: &mut Vec<ProbeReg>) {
    match cluster {
        RegisterCluster::Register(reg) => {
            expand_probe_register(reg, parent_offset, out);
        }
        RegisterCluster::Cluster(c) => match c {
            svd_parser::svd::Cluster::Single(info) => {
                let cluster_offset = parent_offset + info.address_offset as u64;
                for child in &info.children {
                    collect_probe_regs(child, cluster_offset, out);
                }
            }
            svd_parser::svd::Cluster::Array(info, dim) => {
                for i in 0..dim.dim {
                    let instance_offset = parent_offset
                        + info.address_offset as u64
                        + i as u64 * dim.dim_increment as u64;
                    for child in &info.children {
                        collect_probe_regs(child, instance_offset, out);
                    }
                }
            }
        },
    }
}

/// Effective access of an SVD register.
///
/// Register-level `<access>` is optional in the ESP32-S3 SVD — for most
/// registers it lives on the FIELDS instead. A register whose every field is
/// read-only cannot retain probe writes on real silicon either, so defaulting
/// it to ReadWrite would misclassify it as an accept-and-ignore stub
/// (Unmodelled) when the honest verdict is Indeterminate (probe can't tell;
/// the per-peripheral FSM tests confirm it). Derive from the fields when the
/// register itself doesn't declare an access.
fn register_access(info: &svd_parser::svd::RegisterInfo) -> Access {
    use svd_parser::svd::Access as SvdAccess;
    match info.properties.access {
        Some(SvdAccess::ReadOnly) => return Access::ReadOnly,
        Some(SvdAccess::WriteOnly) | Some(SvdAccess::WriteOnce) => return Access::WriteOnly,
        Some(_) => return Access::ReadWrite,
        None => {}
    }
    let mut saw_field = false;
    let mut all_ro = true;
    let mut all_wo = true;
    for f in info.fields() {
        saw_field = true;
        match f.access {
            Some(SvdAccess::ReadOnly) => all_wo = false,
            Some(SvdAccess::WriteOnly) | Some(SvdAccess::WriteOnce) => all_ro = false,
            _ => {
                all_ro = false;
                all_wo = false;
            }
        }
    }
    match (saw_field, all_ro, all_wo) {
        (true, true, _) => Access::ReadOnly,
        (true, _, true) => Access::WriteOnly,
        _ => Access::ReadWrite,
    }
}

fn expand_probe_register(reg: &Register, parent_offset: u64, out: &mut Vec<ProbeReg>) {
    match reg {
        Register::Single(info) => {
            let access = register_access(info);
            let offset = parent_offset + info.address_offset as u64;
            let reset_value = info.properties.reset_value.unwrap_or(0) as u32;
            out.push(ProbeReg {
                name: info.name.clone(),
                offset,
                access,
                reset_value,
            });
        }
        Register::Array(info, dim) => {
            for i in 0..dim.dim {
                let access = register_access(info);
                let offset = parent_offset
                    + info.address_offset as u64
                    + i as u64 * dim.dim_increment as u64;
                let reset_value = info.properties.reset_value.unwrap_or(0) as u32;

                // Substitute the array index into the name pattern (%s / [%s]).
                let index_str = if let Some(dim_index) = &dim.dim_index {
                    if (i as usize) < dim_index.len() {
                        dim_index[i as usize].clone()
                    } else {
                        i.to_string()
                    }
                } else {
                    i.to_string()
                };
                let name = info
                    .name
                    .replace("[%s]", &index_str)
                    .replace("%s", &index_str);

                out.push(ProbeReg {
                    name,
                    offset,
                    access,
                    reset_value,
                });
            }
        }
    }
}

fn load_svd(path: &std::path::Path) -> anyhow::Result<Vec<SvdPeripheral>> {
    let xml = std::fs::read_to_string(path)?;
    let device = svd_parser::parse(&xml)?;
    let mut out = Vec::new();

    for p in &device.peripherals {
        // Extract the PeripheralInfo from the enum variant.
        let p_info = match p {
            Peripheral::Single(info) => info,
            Peripheral::Array(info, _) => info,
        };

        let base = p_info.base_address;
        let mut registers = Vec::new();

        if let Some(children) = &p_info.registers {
            for cluster in children {
                collect_probe_regs(cluster, 0, &mut registers);
            }
        }

        // If the peripheral derives from another and has no registers itself,
        // walk the derivedFrom chain.
        if registers.is_empty() {
            if let Some(base_name) = &p_info.derived_from {
                if let Some(base_p) = device.peripherals.iter().find(|other| {
                    let n = match other {
                        Peripheral::Single(i) => &i.name,
                        Peripheral::Array(i, _) => &i.name,
                    };
                    n == base_name
                }) {
                    let base_info = match base_p {
                        Peripheral::Single(i) => i,
                        Peripheral::Array(i, _) => i,
                    };
                    if let Some(children) = &base_info.registers {
                        for cluster in children {
                            collect_probe_regs(cluster, 0, &mut registers);
                        }
                    }
                }
            }
        }

        if !registers.is_empty() {
            out.push(SvdPeripheral {
                name: p_info.name.clone(),
                base,
                registers,
            });
        }
    }
    Ok(out)
}

struct BusTarget<'a> {
    bus: &'a mut labwired_core::bus::SystemBus,
    base: u64,
}

impl ProbeTarget for BusTarget<'_> {
    fn probe_read(&self, offset: u64) -> Option<u32> {
        self.bus.read_u32(self.base + offset).ok()
    }
    fn probe_write(&mut self, offset: u64, value: u32) -> bool {
        self.bus.write_u32(self.base + offset, value).is_ok()
    }
}

fn build_matrix(svd: &[SvdPeripheral]) -> CoverageMatrix {
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

    let mut matrix = BTreeMap::new();

    for sp in svd {
        // Build a fresh bus per peripheral so a probe-triggered panic (e.g.
        // RSA modulus-zero assertion) cannot poison the shared bus state for
        // subsequent peripherals.
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // Resolve the window the same way the bus router does: last-start-wins
        // binary search. This ensures that a narrower, later-registered twin
        // (e.g. uart0_s3) shadows a broader catch-all stub (e.g. low_mmio)
        // that has an equal base address, matching the actual dispatch behaviour
        // of read_u32 / write_u32.
        let Some((win_base, win_size)) = bus.resolve_window(sp.base) else {
            continue;
        };
        // The SVD base may sit INSIDE a broader covering window (a catch-all
        // stub that spans several SVD peripherals). Probe offsets are applied
        // from the SVD base, so clamp the probe window to the remaining span
        // of the resolved window — otherwise the baseline samples (taken near
        // the window's end) land PAST it on a different peripheral or on
        // nothing, breaking the write_roundtrips detection and crediting
        // generic storage as Modelled.
        let mut window_size = (win_base + win_size).saturating_sub(sp.base);
        // ALSO clamp at the next registered window start: under last-start-
        // wins layering, a narrower twin (e.g. i2s0_s3 inside the low-MMIO
        // catch-all) takes over dispatch above its start even though the
        // covering window continues underneath. Baseline samples taken past
        // that boundary would measure the TWIN's semantics (non-round-trip)
        // and falsely certify the catch-all-backed registers below it as
        // Modelled. Keeping the probe inside [sp.base, next_start) guarantees
        // baseline and registers are served by the same peripheral entry.
        if let Some(next_start) = bus.next_window_start(sp.base) {
            window_size = window_size.min(next_start - sp.base);
        }

        // NOTE: probes drive REAL model behavior (sentinel writes can fire FSMs). A
        // peripheral model that panics on a probe write must be fixed IN THE MODEL
        // (as the RSA zero-modulus guard was) — there is no runtime safety net here:
        // the workspace builds with panic="abort", so catch_unwind would be a no-op.
        let name = sp.name.clone();
        let regs = sp.registers.clone();
        let base = sp.base;

        let mut target = BusTarget {
            bus: &mut bus,
            base,
        };
        let results = probe_peripheral(&mut target, &regs, window_size);

        let cov = {
            let mut cov = PeripheralCoverage {
                modelled: 0,
                indeterminate: 0,
                unmodelled: 0,
                total: results.len(),
                unmodelled_regs: Vec::new(),
            };
            for r in &results {
                match r.status {
                    RegStatus::Modelled => cov.modelled += 1,
                    RegStatus::Indeterminate => cov.indeterminate += 1,
                    RegStatus::Unmodelled => {
                        cov.unmodelled += 1;
                        cov.unmodelled_regs.push(r.name.clone());
                    }
                }
            }
            cov
        };

        matrix.insert(name, cov);
    }
    CoverageMatrix(matrix)
}

pub fn render_text(m: &CoverageMatrix) -> String {
    let mut s = String::new();
    s.push_str("ESP32-S3 register coverage (behavioral probe)\n");
    s.push_str("peripheral            modelled  indet  unmod  total\n");
    let (mut tm, mut ti, mut tu, mut tt) = (0usize, 0usize, 0usize, 0usize);
    for (name, c) in &m.0 {
        s.push_str(&format!(
            "{name:<20}  {:>7}  {:>5}  {:>5}  {:>5}\n",
            c.modelled, c.indeterminate, c.unmodelled, c.total
        ));
        tm += c.modelled;
        ti += c.indeterminate;
        tu += c.unmodelled;
        tt += c.total;
    }
    s.push_str(&format!(
        "{:<20}  {tm:>7}  {ti:>5}  {tu:>5}  {tt:>5}\n",
        "TOTAL"
    ));
    s
}

pub fn run() -> Option<(CoverageMatrix, String)> {
    let svd_path = discover_svd()?;
    let svd = load_svd(&svd_path).ok()?;
    let matrix = build_matrix(&svd);
    let text = render_text(&matrix);
    Some((matrix, text))
}

#[cfg(test)]
mod tests {
    use labwired_core::coverage::Access;

    /// Register-level access is honored; field-level access is the fallback;
    /// mixed or absent field access defaults to ReadWrite.
    #[test]
    fn register_access_derives_from_fields_when_register_level_absent() {
        let svd = r#"<?xml version="1.0" encoding="utf-8"?>
<device schemaVersion="1.1"><name>T</name>
 <addressUnitBits>8</addressUnitBits><width>32</width>
 <peripherals><peripheral><name>P</name><baseAddress>0x0</baseAddress>
  <registers>
   <register><name>REG_RW</name><addressOffset>0x0</addressOffset><access>read-write</access></register>
   <register><name>REG_FIELDS_RO</name><addressOffset>0x4</addressOffset>
    <fields>
     <field><name>A</name><bitOffset>0</bitOffset><bitWidth>1</bitWidth><access>read-only</access></field>
     <field><name>B</name><bitOffset>1</bitOffset><bitWidth>1</bitWidth><access>read-only</access></field>
    </fields></register>
   <register><name>REG_FIELDS_WO</name><addressOffset>0x8</addressOffset>
    <fields>
     <field><name>C</name><bitOffset>0</bitOffset><bitWidth>1</bitWidth><access>write-only</access></field>
    </fields></register>
   <register><name>REG_FIELDS_MIXED</name><addressOffset>0xC</addressOffset>
    <fields>
     <field><name>D</name><bitOffset>0</bitOffset><bitWidth>1</bitWidth><access>read-only</access></field>
     <field><name>E</name><bitOffset>1</bitOffset><bitWidth>1</bitWidth><access>read-write</access></field>
    </fields></register>
   <register><name>REG_BARE</name><addressOffset>0x10</addressOffset></register>
  </registers></peripheral></peripherals></device>"#;
        let device = svd_parser::parse(svd).expect("parse");
        let p = match &device.peripherals[0] {
            svd_parser::svd::Peripheral::Single(i) => i,
            svd_parser::svd::Peripheral::Array(i, _) => i,
        };
        let mut regs = Vec::new();
        for cluster in p.registers.as_ref().unwrap() {
            super::collect_probe_regs(cluster, 0, &mut regs);
        }
        let by_name = |n: &str| {
            regs.iter()
                .find(|r| r.name == n)
                .unwrap_or_else(|| panic!("{n} missing"))
                .access
        };
        assert_eq!(by_name("REG_RW"), Access::ReadWrite, "register-level wins");
        assert_eq!(by_name("REG_FIELDS_RO"), Access::ReadOnly, "all fields RO");
        assert_eq!(by_name("REG_FIELDS_WO"), Access::WriteOnly, "all fields WO");
        assert_eq!(by_name("REG_FIELDS_MIXED"), Access::ReadWrite, "mixed");
        assert_eq!(
            by_name("REG_BARE"),
            Access::ReadWrite,
            "no info defaults RW"
        );
    }

    #[test]
    fn driver_runs_against_real_svd_if_present() {
        if super::discover_svd().is_none() {
            eprintln!("SKIP: no SVD");
            return;
        }
        let (m, text) = super::run().expect("run");
        eprintln!("{text}");
        // I2C0 is known-wired and known-modelled (interrupt-driven i2c_master works).
        if let Some(c) = m.0.get("I2C0") {
            assert!(c.total > 0, "I2C0 should have registers");
            assert!(
                c.modelled > 0,
                "I2C0 should have some modelled registers, got {c:?}"
            );
        }
    }
}
