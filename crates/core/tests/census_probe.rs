// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Liveness gate for the silent-path census.
//!
//! A census that reports zero is only meaningful if the counter is capable of
//! reporting non-zero. This drives a *real* peripheral model at an offset the
//! model demonstrably does not decode and asserts the counter moves — so a
//! "clean" row in `docs/coverage/silent-path-census.md` means "the firmware did
//! not take this path", never "the instrumentation was dead".
//!
//! The whole file compiles away without the `silent-census` feature, because
//! `census::reset`/`to_json` only exist when it is on. Run it with:
//! `cargo test -p labwired-core --features silent-census --test census_probe`

#![cfg(feature = "silent-census")]

use labwired_core::Peripheral;

/// The census is process-global (a run must aggregate whatever thread the sim
/// loop landed on), and every test here calls `reset()` and then asserts
/// absolute totals. `cargo test`'s harness runs the tests in this binary
/// concurrently, so they must be serialised or they read each other's tallies.
static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    labwired_core::census::reset();
    g
}

/// The F1 RCC model decodes 0x00..=0x28 and nothing else (RM0008 §7.3), so
/// 0xF0 is a known-undecoded offset. Reading it must fabricate a zero *and*
/// leave a census entry behind.
#[test]
fn undecoded_rcc_offset_is_counted_and_still_reads_zero() {
    let _guard = serialized();
    let mut rcc = labwired_core::peripherals::rcc::Rcc::new();

    // A decoded offset must NOT be counted: this is the negative control that
    // stops the counter from being trivially always-on.
    assert_eq!(rcc.read_u32(0x00).unwrap(), 0x0000_4A83, "CR reset value");
    assert_eq!(
        labwired_core::census::to_json()["undecoded_register_access"]["total"],
        0,
        "a decoded offset must never be recorded as undecoded"
    );

    // The undecoded offset still behaves exactly as before instrumentation:
    // the read fabricates zero and the write is discarded.
    assert_eq!(rcc.read_u32(0xF0).unwrap(), 0);
    rcc.write_u32(0xF0, 0xDEAD_BEEF).unwrap();
    assert_eq!(rcc.read_u32(0xF0).unwrap(), 0, "write must stay discarded");

    let j = labwired_core::census::to_json();
    let total = j["undecoded_register_access"]["total"].as_u64().unwrap();
    assert!(total > 0, "census_reg! never fired on an undecoded offset");

    let entries = j["undecoded_register_access"]["entries"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        entries
            .iter()
            .any(|e| e["peripheral"] == "rcc:F1Rcc" && e["offset"] == "0x00f0"),
        "expected an rcc:F1Rcc @ 0x00f0 entry, got {entries:?}"
    );
}

/// Documents the byte-granularity multiplier that the raw counts carry, so a
/// reader of the census table divides by the right number.
///
/// `Peripheral::read`/`write` are byte-granular and `read_u32`/`write_u32`
/// decompose into four byte accesses. On top of that, `Rcc::write` is a
/// read-modify-write: each of the four byte writes first calls `read_reg`.
/// So ONE 32-bit undecoded register write costs 4 write-hits AND 4 read-hits,
/// and one 32-bit undecoded read costs 4 read-hits.
#[test]
fn raw_counts_carry_a_four_times_byte_multiplier() {
    let _guard = serialized();
    let mut rcc = labwired_core::peripherals::rcc::Rcc::new();

    rcc.write_u32(0xF0, 0x1234_5678).unwrap();
    let j = labwired_core::census::to_json();
    let entries = j["undecoded_register_access"]["entries"]
        .as_array()
        .unwrap();
    let get = |kind: &str| -> u64 {
        entries
            .iter()
            .find(|e| e["kind"] == kind)
            .and_then(|e| e["count"].as_u64())
            .unwrap_or(0)
    };
    assert_eq!(get("write"), 4, "one u32 write == four byte writes");
    assert_eq!(get("read"), 4, "…each preceded by a read-modify-write read");
}

/// The gate for counter (b2), and the regression test for the wrong number the
/// first census published.
///
/// `configs/chips/stm32l073.yaml` is a *real, shipped* chip descriptor that
/// declares both `type: "nvic"` and `type: "scb"`. Neither string is matched by
/// `from_config`'s factory chain, so both fall through to `StubPeripheral` and
/// both are recorded by counter (b1). Then `configure_cortex_m` — which every
/// ARM construction path runs, and which `from_config` alone does not — replaces
/// those two entries with the real `Nvic` and `Scb` models.
///
/// The first census reported those (b1) rows as live stubs and called them the
/// only "not intentional" findings. They were replaced before a single
/// instruction executed. This test asserts, on the production code path and a
/// committed chip file:
///
/// * (b1) DOES record `nvic` and `scb` — the factory statistic is real,
/// * (b2) does NOT list them — they are not stubs on the assembled machine,
/// * (b2) is not vacuously empty — the chip's genuine `type: "stub"` entries
///   are still there, so a zero would have been a measurement, not a dead
///   counter.
#[test]
fn nvic_and_scb_trip_the_factory_counter_but_are_not_live_stubs() {
    let _guard = serialized();

    let chip_rel = "configs/chips/stm32l073.yaml";
    let chip_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(chip_rel);
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    let manifest = labwired_config::SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "census-probe".to_string(),
        chip: chip_rel.to_string(),
        cpu_hz: None,
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };

    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");

    // (b1) is populated by `from_config` alone.
    let after_factory = labwired_core::census::to_json();
    let factory: Vec<String> = after_factory["stub_factory_fallthrough"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["type"].as_str().unwrap().to_string())
        .collect();
    assert!(
        factory.iter().any(|t| t == "nvic"),
        "the factory counter must still see `nvic` fall through: {factory:?}"
    );
    assert!(
        factory.iter().any(|t| t == "scb"),
        "the factory counter must still see `scb` fall through: {factory:?}"
    );
    assert_eq!(
        after_factory["stub_live_post_construction"]["machines_swept"], 0,
        "the (b2) sweep must not have run yet — no machine exists"
    );

    // NEGATIVE CONTROL — watch the assertion below fail first. Sweep the bus at
    // the point `from_config` hands it back, i.e. the state the original census
    // implicitly measured. Here NVIC and SCB genuinely ARE `StubPeripheral`s, so
    // the "not a stub" assertion further down is discriminating construction
    // stages, not just failing to find an entry.
    labwired_core::census::record_live_stubs(&bus);
    let mid = labwired_core::census::to_json();
    let mid_bases: Vec<String> = mid["stub_live_post_construction"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["base"].as_str().unwrap().to_string())
        .collect();
    assert!(
        mid_bases.iter().any(|b| b == "0xe000e100"),
        "straight out of the factory, 0xE000_E100 must still be a stub: {mid_bases:?}"
    );
    assert!(
        mid_bases.iter().any(|b| b == "0xe000ed00"),
        "straight out of the factory, 0xE000_ED00 must still be a stub: {mid_bases:?}"
    );
    labwired_core::census::reset();

    // …and then construction continues.
    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let _machine = labwired_core::Machine::new(cpu, bus);

    let j = labwired_core::census::to_json();
    let live = &j["stub_live_post_construction"];
    assert_eq!(
        live["machines_swept"], 1,
        "the sweep must have run exactly once"
    );
    let names: Vec<String> = live["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    let bases: Vec<String> = live["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["base"].as_str().unwrap().to_string())
        .collect();

    assert!(
        !bases.iter().any(|b| b == "0xe000e100"),
        "NVIC @ 0xE000_E100 is a real Nvic after configure_cortex_m, not a stub: {names:?}"
    );
    assert!(
        !bases.iter().any(|b| b == "0xe000ed00"),
        "SCB @ 0xE000_ED00 is a real Scb after configure_cortex_m, not a stub: {names:?}"
    );

    // Non-vacuity: the sweep is capable of reporting a stub, and does.
    assert!(
        live["total"].as_u64().unwrap() > 0,
        "the (b2) sweep found nothing at all — the counter is dead, not clean"
    );
}
