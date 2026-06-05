// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-S3 SVD register-coverage driver: parse the SVD, build the wired model,
//! probe every peripheral's registers, and emit a coverage matrix.

use std::collections::BTreeMap;
use std::path::PathBuf;

use labwired_core::coverage::{probe_peripheral, Access, ProbeReg, ProbeTarget, RegStatus};
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

/// Discover the ESP32-S3 SVD: `LABWIRED_ESP32S3_SVD` override, else PlatformIO.
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
    pio.is_file().then_some(pio)
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

fn expand_probe_register(reg: &Register, parent_offset: u64, out: &mut Vec<ProbeReg>) {
    use svd_parser::svd::Access as SvdAccess;

    match reg {
        Register::Single(info) => {
            let access = match info.properties.access {
                Some(SvdAccess::ReadOnly) => Access::ReadOnly,
                Some(SvdAccess::WriteOnly) | Some(SvdAccess::WriteOnce) => Access::WriteOnly,
                _ => Access::ReadWrite,
            };
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
                let access = match info.properties.access {
                    Some(SvdAccess::ReadOnly) => Access::ReadOnly,
                    Some(SvdAccess::WriteOnly) | Some(SvdAccess::WriteOnce) => Access::WriteOnly,
                    _ => Access::ReadWrite,
                };
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

pub fn build_matrix(svd: &[SvdPeripheral]) -> CoverageMatrix {
    use labwired_core::bus::SystemBus;
    use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

    let mut matrix = BTreeMap::new();

    for sp in svd {
        // Build a fresh bus per peripheral so a probe-triggered panic (e.g.
        // RSA modulus-zero assertion) cannot poison the shared bus state for
        // subsequent peripherals.
        let mut bus = SystemBus::new();
        let _ = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());

        // Find the bus peripheral window that contains this SVD peripheral's
        // base address.
        let window = bus
            .peripherals
            .iter()
            .find(|e| sp.base >= e.base && sp.base < e.base + e.size)
            .map(|e| e.size);
        let Some(window_size) = window else {
            continue;
        };

        // Use catch_unwind so a peripheral model that panics on probe (e.g. RSA
        // asserting zero-modulus when we write a sentinel) doesn't abort the
        // entire driver. Those peripherals are marked indeterminate.
        let name = sp.name.clone();
        let regs = sp.registers.clone();
        let base = sp.base;

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut target = BusTarget {
                bus: &mut bus,
                base,
            };
            probe_peripheral(&mut target, &regs, window_size)
        }));

        let cov = match result {
            Ok(results) => {
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
            }
            Err(_) => {
                // Probe panicked (e.g. RSA zero-modulus). Score all registers
                // Indeterminate so the peripheral appears in the matrix but
                // does not inflate either modelled or unmodelled counts.
                PeripheralCoverage {
                    modelled: 0,
                    indeterminate: regs.len(),
                    unmodelled: 0,
                    total: regs.len(),
                    unmodelled_regs: Vec::new(),
                }
            }
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
