// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! The Cortex-M fault verdict on the CLI: decode what the CPU recorded, with
//! the firmware's own symbols, and print it for a human.
//!
//! The decode itself is `labwired_core::fault_verdict::decode_fault`; this
//! module only supplies the ELF symbolizer and the terminal rendering.

use labwired_core::fault_verdict::{decode_fault, FaultSymbolizer, FaultVerdict};

/// The verdict for `cpu`, symbolized against `elf` when it parses. `None`
/// when nothing faulted (or the CPU models no ARM fault registers).
///
/// `policy` is the run's memory-policy report: when the fault address matches
/// the first recorded violation in a read-only region, the verdict names the
/// region.
pub fn fault_verdict<C: labwired_core::Cpu + ?Sized>(
    cpu: &C,
    elf: Option<&[u8]>,
    policy: Option<&labwired_core::bus::MemoryPolicyReport>,
) -> Option<FaultVerdict> {
    let capture = cpu.fault_capture()?;
    let symbols = elf.and_then(|b| labwired_loader::SymbolProvider::from_bytes(b.to_vec()).ok());
    let symbolizer = symbols.as_ref().map(|s| s as &dyn FaultSymbolizer);
    let mut verdict = decode_fault(&capture, symbolizer, None)?;
    if let Some(region) =
        policy.and_then(|report| read_only_region_for(report, verdict.fault_address.as_deref()))
    {
        verdict = decode_fault(
            &capture,
            symbolizer,
            Some(&labwired_core::fault_verdict::DataFaultReason::ReadOnlyRegion { region }),
        )
        .expect("capture is a fault");
    }
    Some(verdict)
}

/// The region name when `report`'s first recorded violation in a `fault`-mode
/// region matches the verdict's fault address. `None` when the address is
/// absent/unparsable, the region is not in `fault` mode, or no violation is
/// recorded for it.
fn read_only_region_for(
    report: &labwired_core::bus::MemoryPolicyReport,
    addr_hex: Option<&str>,
) -> Option<&'static str> {
    let addr_hex = addr_hex?;
    // Robust to display separators: `hex()` emits `0x08001000`, but the
    // grouped form `0x0800_1000` must not silently lose the reason.
    let cleaned: String = addr_hex.strip_prefix("0x").map(|s| s.replace('_', ""))?;
    let addr = u64::from_str_radix(&cleaned, 16).ok()?;
    [("flash", &report.flash), ("ram", &report.ram)]
        .into_iter()
        .find_map(|(name, stats)| {
            (stats.mode == labwired_core::bus::CpuWritePolicy::Fault
                && stats.first.as_ref().is_some_and(|v| v.addr == addr))
            .then_some(name)
        })
}

/// The two human lines: the sentence, then the raw registers it came from.
pub fn render(v: &FaultVerdict) -> String {
    let flags: Vec<&str> = v.causes.iter().map(|c| c.flag.as_str()).collect();
    let mut regs = format!("HFSR={} CFSR={}", v.registers.hfsr, v.registers.cfsr);
    if !flags.is_empty() {
        regs.push_str(&format!(" [{}]", flags.join(" ")));
    }
    if let (Some(addr), Some(src)) = (&v.fault_address, &v.fault_address_source) {
        regs.push_str(&format!(" {src}={addr}"));
    }
    format!("FAULT  {}\n       {}", v.summary, regs)
}

/// Print the verdict to stderr (stdout carries firmware UART and `--json`).
pub fn eprint(v: &FaultVerdict) {
    eprintln!("{}{}", crate::log_ts::prefix(), render(v));
}

#[cfg(test)]
mod tests {
    use super::read_only_region_for;
    use labwired_core::bus::{
        CpuWritePolicy, MemoryPolicyReport, RegionPolicyStats, RegionViolation,
    };

    fn stats(mode: CpuWritePolicy, first: Option<RegionViolation>) -> RegionPolicyStats {
        RegionPolicyStats {
            mode,
            violations: u64::from(first.is_some()),
            first,
        }
    }

    fn violation(addr: u64) -> RegionViolation {
        RegionViolation {
            addr,
            cycle: 7,
            pc: Some(0x0800_0042),
        }
    }

    fn report(
        flash_mode: CpuWritePolicy,
        flash_first: Option<RegionViolation>,
        ram_mode: CpuWritePolicy,
        ram_first: Option<RegionViolation>,
    ) -> MemoryPolicyReport {
        MemoryPolicyReport {
            flash: stats(flash_mode, flash_first),
            ram: stats(ram_mode, ram_first),
        }
    }

    #[test]
    fn flash_fault_matching_addr_names_flash() {
        let report = report(
            CpuWritePolicy::Fault,
            Some(violation(0x0800_1000)),
            CpuWritePolicy::Allow,
            None,
        );
        assert_eq!(
            read_only_region_for(&report, Some("0x08001000")),
            Some("flash")
        );
    }

    #[test]
    fn grouped_hex_separator_is_tolerated() {
        let report = report(
            CpuWritePolicy::Fault,
            Some(violation(0x0800_1000)),
            CpuWritePolicy::Allow,
            None,
        );
        assert_eq!(
            read_only_region_for(&report, Some("0x0800_1000")),
            Some("flash")
        );
    }

    #[test]
    fn ram_fault_matching_addr_names_ram() {
        let report = report(
            CpuWritePolicy::Allow,
            None,
            CpuWritePolicy::Fault,
            Some(violation(0x2000_0004)),
        );
        assert_eq!(
            read_only_region_for(&report, Some("0x20000004")),
            Some("ram")
        );
    }

    #[test]
    fn warn_region_does_not_name_a_region() {
        let report = report(
            CpuWritePolicy::Warn,
            Some(violation(0x0800_1000)),
            CpuWritePolicy::Allow,
            None,
        );
        assert_eq!(read_only_region_for(&report, Some("0x08001000")), None);
    }

    #[test]
    fn allow_region_without_violation_is_none() {
        let report = report(CpuWritePolicy::Allow, None, CpuWritePolicy::Allow, None);
        assert_eq!(read_only_region_for(&report, Some("0x08001000")), None);
    }

    #[test]
    fn non_matching_addr_is_none() {
        let report = report(
            CpuWritePolicy::Fault,
            Some(violation(0x0800_2000)),
            CpuWritePolicy::Allow,
            None,
        );
        assert_eq!(read_only_region_for(&report, Some("0x08001000")), None);
    }

    #[test]
    fn absent_or_malformed_addr_is_none() {
        let report = report(
            CpuWritePolicy::Fault,
            Some(violation(0x0800_1000)),
            CpuWritePolicy::Allow,
            None,
        );
        assert_eq!(read_only_region_for(&report, None), None);
        assert_eq!(read_only_region_for(&report, Some("0xZZ")), None);
        assert_eq!(read_only_region_for(&report, Some("08001000")), None);
    }
}
