// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
//! THE CONTRACT: every peripheral registered on a bus must own at least one
//! address inside its own window.
//!
//! A peripheral that owns none is dead code. It cannot be read or written, its
//! model never runs, and its presence in the system builder is a lie that reads
//! as authoritative — the next person to debug that block will study a model
//! the machine never consults.
//!
//! This is not hypothetical. `apb_ctrl` was registered at SYSCON's own base on
//! classic ESP32 with a wider window, so — under the router's last-start-wins
//! tie-break — it answered every SYSCON register and the real SYSCON model was
//! dead from the day it was written. The visible symptom was a divide-by-zero
//! in Arduino's `_get_effective_baudrate`, three layers away, which got
//! "explained" by stubbing the whole HardwareSerial path. Serial on classic
//! ESP32 was dead for about a year behind that stub.
//!
//! Why the existing checks could not catch it:
//!
//!   * `SystemBus::add_peripheral` says outright "**No overlap check is
//!     performed**". Nothing validates registrations against each other.
//!   * `chip_conformance`'s estate check asserts `bus.read_u32(base).is_ok()`
//!     for each peripheral. A SHADOWED peripheral passes that trivially — the
//!     shadower answers the read. It is green precisely when the bug is
//!     present.
//!
//! So this gate asks the only question that distinguishes the two: for each
//! registered peripheral, is there any address the router actually dispatches
//! to IT? Overlap on its own is fine and often deliberate (a narrow model
//! layered over a broad catch-all). Being *entirely* covered is the defect.

use crate::bus::SystemBus;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

fn repo_root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The first address in peripheral `idx`'s window that the router dispatches to
/// `idx`, or `None` when the peripheral owns nothing at all.
///
/// The routing winner is constant between consecutive *breakpoints*, and a
/// breakpoint is a window START **or** a window END: when a narrow overlapping
/// window ends, the broader one underneath resumes ownership. Sampling starts
/// alone is NOT enough — a broad catch-all whose base is taken by a narrow twin
/// still owns the gap just past that twin's end, and skipping ends reports it
/// as dead. (That mistake made this gate's first run flag `low_mmio` and
/// `mmio_rest` on ESP32-S3, both of which genuinely own their gaps.)
///
/// Sampling every breakpoint inside the range is therefore exact, not a scan.
fn first_owned_address(bus: &SystemBus, idx: usize) -> Option<u64> {
    let (base, size) = {
        let p = &bus.peripherals[idx];
        (p.base, p.size)
    };
    let end = base.checked_add(size)?;

    let mut candidates = vec![base];
    for (j, p) in bus.peripherals.iter().enumerate() {
        if j == idx {
            continue;
        }
        for edge in [p.base, p.base.saturating_add(p.size)] {
            if edge > base && edge < end {
                candidates.push(edge);
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();

    candidates
        .into_iter()
        .find(|&addr| bus.find_peripheral_index(addr) == Some(idx))
}

/// Every fully-shadowed peripheral on `bus`, as `(name, base, size)`.
fn shadowed(bus: &SystemBus) -> Vec<(String, u64, u64)> {
    (0..bus.peripherals.len())
        .filter(|&i| first_owned_address(bus, i).is_none())
        .map(|i| {
            let p = &bus.peripherals[i];
            (p.name.clone(), p.base, p.size)
        })
        .collect()
}

fn report(system: &str, bus: &SystemBus) -> Option<String> {
    let dead = shadowed(bus);
    if dead.is_empty() {
        return None;
    }
    let mut msg = format!(
        "{system}: {} registered peripheral(s) own no address — the router can \
         never dispatch to them, so their models are dead code:\n",
        dead.len()
    );
    for (name, base, size) in &dead {
        // Name who actually serves the base, because that is the fix: either
        // register later (last-start-wins on an equal base), narrow the
        // shadower, or delete the dead entry.
        let owner = bus
            .find_peripheral_index(*base)
            .map(|i| bus.peripherals[i].name.clone())
            .unwrap_or_else(|| "<unmapped>".to_string());
        msg.push_str(&format!(
            "  {name:<24} @{base:#012x} +{size:#x}  — {base:#012x} is served by `{owner}`\n"
        ));
    }
    msg.push_str(
        "\nOverlap itself is fine (a narrow model over a broad catch-all is a \
         normal layering). Being ENTIRELY covered is the defect. Remember the \
         router's rule: greatest start wins, and EQUAL starts resolve to the \
         LAST registered.\n",
    );
    Some(msg)
}

fn dummy_manifest(chip_path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "peripheral-reachability".to_string(),
        chip: chip_path.to_string(),
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

/// Every chip descriptor we ship, enumerated FROM DISK so a chip added later
/// cannot quietly escape the gate by not being listed here.
#[test]
fn no_declarative_chip_registers_a_shadowed_peripheral() {
    let dir = repo_root("configs/chips");
    let mut failures = Vec::new();
    let mut checked = 0usize;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    paths.sort();

    for path in paths {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        // Internal harness fixtures, not silicon we ship.
        if stem.starts_with("ci-fixture-") {
            continue;
        }
        let chip = match ChipDescriptor::from_file(&path) {
            Ok(c) => c,
            // Loading is another gate's job; don't fail this one for it.
            Err(_) => continue,
        };
        let abs = path.to_string_lossy().to_string();
        let bus = match SystemBus::from_config(&chip, &dummy_manifest(&abs)) {
            Ok(b) => b,
            Err(_) => continue,
        };
        checked += 1;
        if let Some(msg) = report(&stem, &bus) {
            failures.push(msg);
        }
    }

    assert!(
        checked > 0,
        "no chip descriptors were checked — the gate is vacuous"
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The hand-built Xtensa systems, which `from_config` never sees. This is where
/// the apb_ctrl/SYSCON shadow lived: these builders call `add_peripheral`
/// directly, dozens of times, with no overlap validation anywhere.
#[test]
fn no_handbuilt_xtensa_system_registers_a_shadowed_peripheral() {
    let mut failures = Vec::new();

    let mut esp32 = SystemBus::new();
    let _cpu = crate::system::xtensa::configure_xtensa_esp32(&mut esp32);
    if let Some(msg) = report("esp32 (configure_xtensa_esp32)", &esp32) {
        failures.push(msg);
    }

    let mut esp32s3 = SystemBus::new();
    let _wiring = crate::system::xtensa::configure_xtensa_esp32s3(
        &mut esp32s3,
        &crate::system::xtensa::Esp32s3Opts::default(),
    );
    if let Some(msg) = report("esp32s3 (configure_xtensa_esp32s3)", &esp32s3) {
        failures.push(msg);
    }

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// The gate must be able to SEE the defect, not merely pass. Build the exact
/// apb_ctrl/SYSCON shape — a wider window registered later at an equal base —
/// and assert it is reported. Without this, a refactor that broke
/// `first_owned_address` would leave every check above vacuously green, which
/// is the failure mode this whole file exists to prevent.
#[test]
fn the_gate_detects_a_shadowed_peripheral() {
    use crate::system::xtensa::RamPeripheral;

    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "victim",
        0x4000_0000,
        0x100,
        None,
        Box::new(RamPeripheral::new(0x100)),
    );
    // Equal base, wider window, registered LATER → wins outright.
    bus.add_peripheral(
        "shadower",
        0x4000_0000,
        0x1000,
        None,
        Box::new(RamPeripheral::new(0x1000)),
    );

    let dead = shadowed(&bus);
    assert_eq!(
        dead.len(),
        1,
        "expected exactly the victim to be reported, got {dead:?}"
    );
    assert_eq!(dead[0].0, "victim");

    // And the inverse: a narrow model layered at a GREATER start is normal
    // layering, not a defect — the gate must not flag it.
    let mut ok = SystemBus::new();
    ok.add_peripheral(
        "broad",
        0x5000_0000,
        0x1000,
        None,
        Box::new(RamPeripheral::new(0x1000)),
    );
    ok.add_peripheral(
        "narrow",
        0x5000_0800,
        0x100,
        None,
        Box::new(RamPeripheral::new(0x100)),
    );
    assert!(
        shadowed(&ok).is_empty(),
        "layering a narrow model over a broad one must not be flagged: {:?}",
        shadowed(&ok)
    );
}

/// The sampling must look at window ENDS, not just starts.
///
/// A broad catch-all whose base is taken by a narrow later-registered twin
/// still owns everything past that twin's end. An earlier version of this file
/// walked `next_window_start` only and so reported such a catch-all as dead —
/// a false positive that, acted on, would have deleted a live peripheral. This
/// pins the sampling down.
#[test]
fn a_catch_all_whose_base_is_taken_still_owns_the_gap_past_the_twin() {
    use crate::system::xtensa::RamPeripheral;

    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "catch_all",
        0x6000_0000,
        0x7000,
        None,
        Box::new(RamPeripheral::new(0x7000)),
    );
    // Equal base, narrower, registered later → owns only 0x6000_0000..0x100.
    bus.add_peripheral(
        "uart0",
        0x6000_0000,
        0x100,
        None,
        Box::new(RamPeripheral::new(0x100)),
    );
    // A second island further up, so the only proof of life for `catch_all`
    // lies in the gaps BETWEEN these two — reachable only by sampling ends.
    bus.add_peripheral(
        "spimem",
        0x6000_2000,
        0x100,
        None,
        Box::new(RamPeripheral::new(0x100)),
    );

    assert!(
        shadowed(&bus).is_empty(),
        "catch_all owns 0x6000_0100.. and must not be reported dead: {:?}",
        shadowed(&bus)
    );
    // Be explicit about WHERE it lives, so a future change that keeps the test
    // green for the wrong reason is still caught.
    let idx = bus
        .peripherals
        .iter()
        .position(|p| p.name == "catch_all")
        .unwrap();
    assert_eq!(first_owned_address(&bus, idx), Some(0x6000_0100));
}

/// Diagnostic probe: pin down the equal-base tie-break empirically, and prove
/// it does not depend on query order (i.e. the routing caches cannot change
/// the answer). Routing must be a pure function of the address.
#[test]
fn equal_base_tiebreak_is_last_registered_and_order_independent() {
    use crate::system::xtensa::RamPeripheral;

    let build = || {
        let mut b = SystemBus::new();
        b.add_peripheral(
            "broad",
            0x6000_0000,
            0x7000,
            None,
            Box::new(RamPeripheral::new(0x7000)),
        );
        b.add_peripheral(
            "narrow",
            0x6000_0000,
            0x100,
            None,
            Box::new(RamPeripheral::new(0x100)),
        );
        b
    };
    let name = |b: &SystemBus, a: u64| {
        b.find_peripheral_index(a)
            .map(|i| b.peripherals[i].name.clone())
    };

    // Cold query at the shared base.
    let cold = build();
    let at_base_cold = name(&cold, 0x6000_0000);

    // Same query, but after touching an address only `broad` owns — which
    // primes the routing cache with `broad`.
    let warm = build();
    let _ = name(&warm, 0x6000_0100);
    let at_base_warm = name(&warm, 0x6000_0000);

    assert_eq!(
        at_base_cold, at_base_warm,
        "routing at {:#x} changed with query order: cold={:?} warm={:?} — the \
         cache is not transparent, so routing is not a pure function of address",
        0x6000_0000u64, at_base_cold, at_base_warm
    );
    assert_eq!(
        at_base_cold.as_deref(),
        Some("narrow"),
        "equal starts must resolve to the LAST registered entry"
    );
}
