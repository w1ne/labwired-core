// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Firmware coverage on a real run: the committed nRF54L15 smart-ring probe,
//! run through a `Session` with coverage on, mapped through its own DWARF.

use labwired_core::session::{OpenOptions, Session};
use labwired_core::system::builder::*;
use labwired_loader::coverage::CoverageReport;
use labwired_loader::SymbolProvider;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn open_ring(coverage: bool) -> Session {
    let sys_path = root().join("examples/nrf54l15-smart-ring/system.yaml");
    let mut manifest = labwired_config::SystemManifest::from_file(&sys_path).unwrap();
    let chip_path = sys_path.parent().unwrap().join(&manifest.chip);
    manifest.chip = chip_path.to_string_lossy().into_owned();
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path).unwrap();
    let fw = std::fs::read(root().join("tests/fixtures/nrf54l15-smart-ring.elf")).unwrap();
    let blobs = BlobMap::new();
    Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions {
            coverage,
            ..OpenOptions::default()
        },
    )
    .unwrap()
}

fn report(s: &Session) -> CoverageReport {
    let symbols = SymbolProvider::from_bytes(s.firmware_elf().to_vec()).unwrap();
    CoverageReport::from_run(&symbols, s.coverage().expect("coverage is on"))
}

fn function<'a>(
    r: &'a CoverageReport,
    name: &str,
) -> &'a labwired_loader::coverage::FunctionCoverage {
    r.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function {name}"))
}

/// The probe runs to "probe done" inside 20 000 cycles: all of main.c runs,
/// the fault handlers never do.
#[test]
fn full_run_maps_lines_and_functions() {
    let mut s = open_ring(true);
    s.run_cycles(20_000).unwrap();
    assert!(s.uart_transcript().contains("probe done"));
    let r = report(&s);

    let main_c = r.files.iter().find(|f| f.file == "main.c").unwrap();
    assert_eq!(
        main_c.lines_hit, main_c.lines_found,
        "every main.c statement runs"
    );
    let startup = r.files.iter().find(|f| f.file == "startup.c").unwrap();
    assert!(
        startup.lines_hit < startup.lines_found,
        "fault handlers never run"
    );
    assert!(r.covered_statements < r.total_statements);

    let main = function(&r, "main");
    assert!(main.entered);
    assert_eq!(main.file.as_deref(), Some("main.c"));
    assert_eq!(main.lines_hit, main.lines_found);
    assert!(function(&r, "twim_read_reg").entered);
    let hf = function(&r, "HardFault_Handler");
    assert!(!hf.entered);
    assert_eq!(hf.lines_hit, 0);
    assert_eq!(
        r.covered_functions,
        r.functions.iter().filter(|f| f.entered).count()
    );
    assert!(r.covered_functions < r.total_functions);
    assert!(r.total_branches > 0 && r.covered_branches > 0);

    let lcov = r.to_lcov();
    assert!(lcov.contains("SF:main.c\n"));
    assert!(lcov.contains(&format!("FN:{},main\n", main.line.unwrap())));
    assert!(lcov.contains("FNDA:1,main\n"));
    assert!(lcov.contains("FNDA:0,HardFault_Handler\n"));
    assert!(lcov.contains("BRDA:"));
    assert!(lcov.contains("end_of_record\n"));
}

/// A run stopped early covers strictly less than the full run.
#[test]
fn a_shorter_run_covers_less() {
    let mut short = open_ring(true);
    short.run_cycles(300).unwrap();
    let mut full = open_ring(true);
    full.run_cycles(20_000).unwrap();
    let (a, b) = (report(&short), report(&full));
    assert!(a.covered_statements < b.covered_statements);
    assert!(!function(&a, "probe8").entered);
    assert!(function(&b, "probe8").entered);
}

/// Same inputs, same report; and a restored session covers the same lines
/// as a straight run to the same point.
#[test]
fn deterministic_and_restore_equivalent() {
    let run = || {
        let mut s = open_ring(true);
        s.run_cycles(20_000).unwrap();
        report(&s)
    };
    assert_eq!(run(), run(), "same inputs must give the same report");

    let mut straight = open_ring(true);
    straight.run_cycles(20_000).unwrap();

    let mut restored = open_ring(true);
    restored.run_cycles(1_000).unwrap();
    let snap = restored.snapshot();
    restored.run_cycles(3_000).unwrap();
    restored.restore(&snap).unwrap();
    restored.run_cycles(19_000).unwrap();
    assert_eq!(restored.cycles(), straight.cycles());
    let (a, b) = (report(&straight), report(&restored));
    assert_eq!(a.files, b.files, "same lines covered");
    assert_eq!(a.functions, b.functions, "same functions entered");
    assert_eq!(a.executed_addresses, b.executed_addresses);
}

/// A session opened without coverage has no observer to report from.
#[test]
fn coverage_is_opt_in() {
    let s = open_ring(false);
    assert!(s.coverage().is_none());
}
