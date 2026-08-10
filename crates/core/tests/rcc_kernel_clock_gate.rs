// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32L0 RNG answers only while its **kernel clock** (HSI48) is running.
//!
//! # Where this test comes from — a silicon capture, not the model
//!
//! `examples/nucleo-l073rz/VALIDATION.md` records a side-by-side run of
//! `firmware-l073-demo` on a real NUCLEO-L073RZ and in the simulator:
//!
//! | reading  | silicon | simulator  |
//! |----------|---------|------------|
//! | RNG draw | `0`     | `CAFEBABE` |
//!
//! The silicon `0` is not a random draw that happened to be zero — it is *no
//! draw at all*. The firmware asked for HSI48 at the wrong address:
//!
//! ```text
//! const RCC_CRRCR: *mut u32 = 0x4002_1098 as *mut u32;  // pre-fix constant
//! ```
//!
//! `0x98` is where CRRCR sits on the WB / G4 / H5 families. On the L0 it is at
//! `0x08` — `tests/fixtures/real_world/stm32l073.svd`, `RCC.CRRCR@0x08` with
//! `HSI48ON@0` / `HSI48RDY@1`; the L0 RCC register file ends at CSR `0x50`, so
//! `0x98` is reserved space with no register behind it. HSI48 therefore never
//! started, the RNG never got a kernel clock, `SR.DRDY` never asserted, and
//! `DR` read `0`.
//!
//! The simulator returned `CAFEBABE` anyway: `peripherals/rng.rs` derives
//! `DRDY` from `CR.RNGEN` alone. That is a **false pass** — the one outcome a
//! hardware oracle may not produce — and it was filed in VALIDATION.md as
//! "non-deterministic — by design".
//!
//! # What is asserted
//!
//! The first tests below replay the pre-fix and post-fix firmware register
//! sequences over the *production* `from_config` bus built from the shipped
//! `configs/chips/stm32l073.yaml`, and assert the simulator now agrees with
//! silicon in both directions:
//!
//! * wrong CRRCR address ⇒ HSI48 never ready ⇒ no DRDY, `DR == 0` (silicon);
//! * right CRRCR address ⇒ HSI48 ready ⇒ DRDY, `DR` delivers a word.
//!
//! The second is the discriminator: without it, a gate that simply killed the
//! RNG outright would also "pass" the first. A third asserts the gate reads the
//! LIVE register map — switching HSI48 back off silences the RNG again.
//!
//! The corpus guards at the bottom cover the mechanism rather than this one
//! chip: every `clock:` declaration in every shipped chip descriptor must
//! resolve against its own family's RCC map, and an unresolvable or empty
//! declaration must fail the build loudly rather than become a gate that never
//! fires.

use labwired_core::Bus;

// ── Addresses, every one of them from tests/fixtures/real_world/stm32l073.svd ──

/// `RCC` peripheral `baseAddress` (SVD).
const RCC_BASE: u64 = 0x4002_1000;
/// `RCC.CRRCR` `addressOffset = 0x8` (SVD). The HSI48 control register.
const RCC_CRRCR: u64 = RCC_BASE + 0x08;
/// The address the pre-fix firmware used. Reserved space on the L0: the RCC
/// register file ends at `CSR@0x50`.
const RCC_CRRCR_WRONG: u64 = RCC_BASE + 0x98;
/// `RCC.AHBENR` `addressOffset = 0x30` (SVD).
const RCC_AHBENR: u64 = RCC_BASE + 0x30;
/// `RCC.AHBENR.RNGEN` `bitOffset = 20` (SVD). The RNG's *bus* clock — the
/// pre-fix firmware sets this one correctly, which is precisely why an
/// enable-register gate alone would not have caught the bug.
const AHBENR_RNGEN: u32 = 1 << 20;
/// `RCC.CRRCR.HSI48ON` `bitOffset = 0` (SVD).
const CRRCR_HSI48ON: u32 = 1 << 0;
/// `RCC.CRRCR.HSI48RDY` `bitOffset = 1` (SVD).
const CRRCR_HSI48RDY: u32 = 1 << 1;

/// `RNG` peripheral `baseAddress` (SVD).
const RNG_BASE: u64 = 0x4002_5000;
/// `RNG.CR` `addressOffset = 0x0` (SVD).
const RNG_CR: u64 = RNG_BASE;
/// `RNG.SR` `addressOffset = 0x4` (SVD).
const RNG_SR: u64 = RNG_BASE + 0x04;
/// `RNG.DR` `addressOffset = 0x8` (SVD).
const RNG_DR: u64 = RNG_BASE + 0x08;
/// `RNG.CR.RNGEN` `bitOffset = 2` (SVD).
const RNG_CR_RNGEN: u32 = 1 << 2;
/// `RNG.SR.DRDY` `bitOffset = 0` (SVD).
const RNG_SR_DRDY: u32 = 1 << 0;

/// The bounded poll `firmware-l073-demo::spin_until` uses.
const SPIN_LIMIT: u32 = 20_000;

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The production bus for the shipped `stm32l073` chip descriptor — the same
/// `SystemBus::from_config` path every lab and every `labwired run` builds.
fn l073_bus() -> labwired_core::bus::SystemBus {
    bus_for("configs/chips/stm32l073.yaml")
}

fn bus_for(chip_rel: &str) -> labwired_core::bus::SystemBus {
    let chip_path = workspace_root().join(chip_rel);
    let chip = labwired_config::ChipDescriptor::from_file(&chip_path).expect("chip yaml parses");
    let manifest = manifest_for(chip_rel);
    labwired_core::bus::SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("{chip_rel}: bus build failed: {e}"))
}

fn manifest_for(chip_rel: &str) -> labwired_config::SystemManifest {
    labwired_config::SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "rcc-kernel-clock-gate".to_string(),
        chip: chip_rel.to_string(),
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

/// One outcome of the firmware's RNG bring-up.
#[derive(Debug)]
struct RngDraw {
    /// Did the HSI48 ready handshake ever complete?
    hsi48_ready: bool,
    /// Did `RNG.SR.DRDY` ever assert?
    drdy: bool,
    /// The word `RNG.DR` handed back.
    dr: u32,
}

/// Replay `firmware-l073-demo`'s bring-up + draw, with `crrcr` as the address
/// the firmware believes CRRCR lives at. Everything else is byte-for-byte the
/// firmware's own sequence (`crates/firmware-l073-demo/src/main.rs`:
/// `init_clocks` then `test_rng`).
fn draw_rng(bus: &mut labwired_core::bus::SystemBus, crrcr: u64) -> RngDraw {
    // HSI48 (RC48) — the RNG kernel clock on the L0.
    let prev = bus.read_u32(crrcr).expect("crrcr read");
    bus.write_u32(crrcr, prev | CRRCR_HSI48ON)
        .expect("crrcr write");
    let mut hsi48_ready = false;
    for _ in 0..SPIN_LIMIT {
        if bus.read_u32(crrcr).expect("crrcr read") & CRRCR_HSI48RDY != 0 {
            hsi48_ready = true;
            break;
        }
    }

    // AHBENR.RNGEN — the RNG's bus clock. The pre-fix firmware got this right.
    let prev = bus.read_u32(RCC_AHBENR).expect("ahbenr read");
    bus.write_u32(RCC_AHBENR, prev | AHBENR_RNGEN)
        .expect("ahbenr write");

    // The draw itself.
    bus.write_u32(RNG_CR, RNG_CR_RNGEN).expect("rng cr write");
    let mut drdy = false;
    for _ in 0..SPIN_LIMIT {
        if bus.read_u32(RNG_SR).expect("rng sr read") & RNG_SR_DRDY != 0 {
            drdy = true;
            break;
        }
    }
    let dr = bus.read_u32(RNG_DR).expect("rng dr read");
    RngDraw {
        hsi48_ready,
        drdy,
        dr,
    }
}

/// **The acceptance test.** With the pre-fix firmware's wrong CRRCR address the
/// RNG must report *no data*, exactly as the NUCLEO-L073RZ silicon capture did.
///
/// Before this change the simulator answered `0xCAFEBABE` here.
#[test]
fn l073_rng_without_hsi48_reports_no_data_like_silicon() {
    let mut bus = l073_bus();
    let got = draw_rng(&mut bus, RCC_CRRCR_WRONG);

    assert!(
        !got.hsi48_ready,
        "0x{RCC_CRRCR_WRONG:08X} is reserved space on the L0 (stm32l073.svd: RCC \
         registers end at CSR@0x50); HSI48 cannot become ready through it, but \
         the model reported ready: {got:?}"
    );
    assert!(
        !got.drdy,
        "RNG.SR.DRDY asserted with no kernel clock; silicon never raises it \
         (examples/nucleo-l073rz/VALIDATION.md): {got:?}"
    );
    assert_eq!(
        got.dr, 0,
        "RNG.DR must read 0 with HSI48 off — that is what the NUCLEO-L073RZ \
         silicon capture recorded. A non-zero draw here is a FALSE PASS: the \
         simulator would bless firmware that never clocked the RNG. Got {got:?}"
    );
}

/// The discriminator: correctly clocked, the RNG must still work. Without this,
/// a gate that killed the RNG unconditionally would pass the test above.
#[test]
fn l073_rng_with_hsi48_still_delivers_a_word() {
    let mut bus = l073_bus();
    let got = draw_rng(&mut bus, RCC_CRRCR);

    assert!(
        got.hsi48_ready,
        "CRRCR@0x08 HSI48ON must raise HSI48RDY (stm32l073.svd): {got:?}"
    );
    assert!(got.drdy, "a correctly clocked RNG must raise DRDY: {got:?}");
    assert_ne!(
        got.dr, 0,
        "a correctly clocked RNG must deliver a word: {got:?}"
    );
}

/// The gate must be a *live register* read, not a build-time latch: turning
/// HSI48 back off has to silence the RNG again mid-run, the way silicon does.
#[test]
fn l073_rng_goes_quiet_again_when_hsi48_is_switched_off() {
    let mut bus = l073_bus();
    let on = draw_rng(&mut bus, RCC_CRRCR);
    assert!(on.drdy, "precondition: clocked draw works: {on:?}");

    // Clear HSI48ON — everything else (AHBENR.RNGEN, RNG.CR.RNGEN) stays set.
    let prev = bus.read_u32(RCC_CRRCR).expect("crrcr read");
    bus.write_u32(RCC_CRRCR, prev & !CRRCR_HSI48ON)
        .expect("crrcr write");
    assert_eq!(
        bus.read_u32(RCC_CRRCR).expect("crrcr read") & CRRCR_HSI48RDY,
        0,
        "HSI48RDY must drop when HSI48ON is cleared"
    );

    assert_eq!(
        bus.read_u32(RNG_SR).expect("rng sr read") & RNG_SR_DRDY,
        0,
        "DRDY must drop once the kernel clock stops"
    );
    assert_eq!(
        bus.read_u32(RNG_DR).expect("rng dr read"),
        0,
        "DR must read 0 once the kernel clock stops"
    );
}

// ── Corpus guards ───────────────────────────────────────────────────────────

/// Every `clock:` declaration in every shipped chip descriptor resolves against
/// its own family's RCC map.
///
/// Register offsets differ per family (L0 `crrcr@0x08` vs WB `crrcr@0x98`), so a
/// name that is right on one chip is wrong on the next. Resolution failure is a
/// hard `from_config` error by design; this sweeps the whole shipped corpus so a
/// typo is caught here rather than at a customer's first run. It also asserts
/// the sweep is not vacuous — it must actually find gates.
#[test]
fn every_shipped_chip_clock_gate_resolves() {
    let chips_dir = workspace_root().join("configs/chips");
    let mut checked = 0usize;
    let mut gated_peripherals = 0usize;

    let mut entries: Vec<_> = std::fs::read_dir(&chips_dir)
        .expect("configs/chips")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    entries.sort();

    for path in entries {
        let text = std::fs::read_to_string(&path).expect("read chip yaml");
        if !text.contains("clock:") {
            continue;
        }
        let rel = format!(
            "configs/chips/{}",
            path.file_name().unwrap().to_string_lossy()
        );
        let chip = labwired_config::ChipDescriptor::from_file(&path).expect("chip yaml parses");
        let declared: Vec<(String, usize)> = chip
            .peripherals
            .iter()
            .filter_map(|p| p.clock.as_ref().map(|g| (p.id.clone(), g.as_slice().len())))
            .collect();
        if declared.is_empty() {
            continue;
        }
        checked += 1;

        // `from_config` is where resolution happens; a bad `reg` name panics here
        // with the chip named.
        let bus = bus_for(&rel);

        for (id, want_bits) in declared {
            let idx = bus
                .find_peripheral_index_by_name(&id)
                .unwrap_or_else(|| panic!("{rel}: peripheral '{id}' not on the bus"));
            let gate = bus.peripherals[idx]
                .clock_gate
                .as_ref()
                .unwrap_or_else(|| panic!("{rel}: '{id}' declares clock: but resolved to no gate"));
            assert_eq!(
                gate.requires.len(),
                want_bits,
                "{rel}: '{id}' declares {want_bits} required RCC bit(s) but resolved {}",
                gate.requires.len()
            );
            gated_peripherals += 1;
        }
    }

    assert!(
        checked >= 10,
        "the sweep found only {checked} chip descriptors with clock gates — a \
         zero/near-zero here means the sweep stopped measuring, not that the \
         corpus is clean"
    );
    assert!(
        gated_peripherals >= 40,
        "only {gated_peripherals} gated peripherals found across the corpus"
    );
}

/// A `reg` name the active family does not have must be a LOUD build failure.
///
/// The dangerous alternative is a gate that silently resolves to nothing and
/// therefore never fires — the peripheral would answer unclocked firmware
/// forever and the yaml would still read as if it were gated.
#[test]
fn an_unknown_clock_register_name_fails_the_build_loudly() {
    let chip_rel = "configs/chips/stm32l073.yaml";
    let chip_path = workspace_root().join(chip_rel);
    let mut chip = labwired_config::ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    for p in &mut chip.peripherals {
        if p.id == "rng" {
            p.clock = Some(labwired_config::ClockGates::One(
                labwired_config::ClockGate {
                    // Real register — on the WB/G4/H5 families, not on the L0.
                    reg: "apb1enr2".to_string(),
                    bit: 1,
                },
            ));
        }
    }
    let err = labwired_core::bus::SystemBus::from_config(&chip, &manifest_for(chip_rel))
        .err()
        .expect("an unresolvable clock-gate register must fail the build");
    let msg = err.to_string();
    assert!(
        msg.contains("rng") && msg.contains("apb1enr2"),
        "the error must name the peripheral and the bad register: {msg}"
    );
}

/// An empty `clock: []` gates nothing while reading as a gate — reject it.
#[test]
fn an_empty_clock_gate_fails_the_build() {
    let chip_rel = "configs/chips/stm32l073.yaml";
    let chip_path = workspace_root().join(chip_rel);
    let mut chip = labwired_config::ChipDescriptor::from_file(&chip_path).expect("chip yaml");
    for p in &mut chip.peripherals {
        if p.id == "rng" {
            p.clock = Some(labwired_config::ClockGates::All(Vec::new()));
        }
    }
    let err = labwired_core::bus::SystemBus::from_config(&chip, &manifest_for(chip_rel))
        .err()
        .expect("an empty clock gate must fail the build");
    assert!(
        err.to_string().contains("rng"),
        "the error must name the peripheral: {err}"
    );
}
