// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test for the nokia5110-invaders-lab: the same firmware ELF that
// flashes to a real NUCLEO-L476RG + Nokia 5110 + HC-SR04 must, in the
// simulator, (a) drive the PCD8544 framebuffer via the bus D/C-pin keystone
// and (b) move the player ship as the host-controlled HC-SR04 distance
// changes — proving both new peripheral models work through real firmware.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::{SystemBus, RECOMMENDED_TICK_INTERVAL};
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::peripherals::components::Pcd8544;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{DebugControl, Machine};
use std::path::{Path, PathBuf};
use std::process::Command;

type Cm = Machine<CortexM>;

const W: usize = 84;

fn ensure_firmware_built() -> PathBuf {
    let elf = labwired_core::test_support::target_dir()
        .join("thumbv7em-none-eabihf/release/nokia5110-invaders-lab");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nokia5110-invaders-lab/src/main.rs");
    if let (Ok(e), Ok(s)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
        if e.modified().unwrap() >= s.modified().unwrap() {
            return elf;
        }
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "nokia5110-invaders-lab",
            "--target",
            "thumbv7em-none-eabihf",
            "--release",
        ])
        // See e2e_epaper_tricolor: clear coverage instrumentation flags so the
        // no_std firmware cross-build doesn't fail with E0463 under llvm-cov.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .status()
        .expect("cargo build nokia5110-invaders-lab");
    assert!(status.success(), "nokia5110-invaders-lab build failed");
    assert!(elf.exists(), "ELF not found at {elf:?}");
    elf
}

fn build_machine(elf: &Path) -> Cm {
    build_machine_with(elf, None)
}

/// Build the invaders machine, optionally overriding the manifest's
/// `walk_deleted` field (`None` keeps the yaml's explicit `Some(true)`).
fn build_machine_with(elf: &Path, walk_deleted: Option<Option<bool>>) -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nokia5110-invaders-lab/system.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load manifest");
    if let Some(wd) = walk_deleted {
        manifest.walk_deleted = wd;
    }
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn framebuffer(machine: &Cm) -> Vec<u8> {
    let idx = machine.bus.find_peripheral_index_by_name("spi1").unwrap();
    let spi = machine.bus.peripherals[idx]
        .dev
        .as_any()
        .unwrap()
        .downcast_ref::<labwired_core::peripherals::spi::Spi>()
        .unwrap();
    spi.attached_devices
        .iter()
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Pcd8544>()))
        .expect("PCD8544 attached to spi1")
        .framebuffer()
        .to_vec()
}

/// Left-most column of the player ship (bank 5 holds the ship rows).
fn ship_left(fb: &[u8]) -> Option<usize> {
    (0..W).find(|&x| fb[5 * W + x] != 0)
}

fn step_frames(machine: &mut Cm, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

/// Dump the sim's PCD8544 panel framebuffer (what the modeled panel received
/// over SPI) as little-endian 32-bit words, in the same layout `mdw` prints,
/// so it can be diffed byte-for-byte against a capture read off real silicon
/// (the firmware's static FB at 0x20000000). Proves the sim's SPI+D/C path
/// delivers exactly the bytes the firmware emits.
#[test]
#[ignore = "prints the sim splash framebuffer for HW diff"]
fn dump_splash_framebuffer() {
    let elf = ensure_firmware_built();
    let mut machine = build_machine(&elf);
    // Step into the splash hold (after init + render + the 504-byte push,
    // well before the 8M-cycle delay ends).
    step_frames(&mut machine, 800_000);
    let fb = framebuffer(&machine);
    assert_eq!(fb.len(), 504);
    for (i, chunk) in fb.chunks(4).enumerate() {
        let w = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        print!("{w:08x} ");
        if i % 8 == 7 {
            println!();
        }
    }
    println!();
}

// Builds the firmware and steps several million cycles in the sim (~90 s), so
// it's gated behind --ignored rather than run in the default suite. Requires
// the example to be a workspace member (examples/nokia5110-invaders-lab):
//   cargo test -p labwired-core --test e2e_nokia5110_invaders -- --ignored --nocapture
#[test]
#[ignore = "slow: builds + runs the Space Invaders firmware in-sim"]
fn firmware_draws_and_ship_tracks_distance() {
    let elf = ensure_firmware_built();
    let mut machine = build_machine(&elf);

    // Near target → short echo → ship to the right edge.
    machine.bus.hcsr04[0].set_distance_cm(8.0);
    step_frames(&mut machine, 4_000_000);
    let fb_near = framebuffer(&machine);
    assert!(
        fb_near.iter().any(|&b| b != 0),
        "firmware should have drawn something to the Nokia framebuffer"
    );
    let near = ship_left(&fb_near).expect("ship visible (near)");

    // Far target → long echo → ship to the left edge.
    machine.bus.hcsr04[0].set_distance_cm(300.0);
    step_frames(&mut machine, 4_000_000);
    let fb_far = framebuffer(&machine);
    let far = ship_left(&fb_far).expect("ship visible (far)");

    assert!(
        far < near,
        "ship should move left as the target moves away: near_x={near}, far_x={far}"
    );
}

/// Phase 1.6 gate: the splash framebuffer the panel receives over SPI must be
/// byte-identical at interval 1 and at the batching intervals the host uses,
/// through the real batched `Machine::run` loop. Before the cycle-exact
/// scheduler conversion the tick-index timebase stretched every SPI frame
/// ×interval, so the push was still in flight (or mis-clocked) at the same
/// cycle budget.
///
/// Checks BOTH 64 (the interval this gate was written against) and
/// [`RECOMMENDED_TICK_INTERVAL`] — the interval the browser actually raises to
/// on a walk-deleted bus. Taking the latter from the constant rather than a
/// literal is the point: this file previously hard-coded `64` in two places,
/// and when the constant moved to 512 the gate went on asserting a number
/// nothing ships, undetected because the test is `#[ignore]`d.
#[test]
#[ignore = "slow: builds + runs the Space Invaders firmware in-sim"]
fn splash_framebuffer_matches_across_tick_intervals() {
    let elf = ensure_firmware_built();
    let fb_at = |interval: u32| {
        let mut machine = build_machine(&elf);
        machine.config.peripheral_tick_interval = interval;
        machine.bus.config.peripheral_tick_interval = interval;
        // Into the splash hold via the batched run loop (see
        // dump_splash_framebuffer for the budget rationale).
        while machine.total_cycles < 800_000 {
            let remaining = (800_000 - machine.total_cycles).min(u32::MAX as u64) as u32;
            machine.run(Some(remaining)).expect("run");
        }
        framebuffer(&machine)
    };
    let fb1 = fb_at(1);
    assert_eq!(fb1.len(), 504);
    assert!(
        fb1.iter().any(|&b| b != 0),
        "splash must be drawn at interval 1"
    );
    assert_eq!(
        fb1,
        fb_at(64),
        "splash framebuffer must be byte-identical at tick interval 64"
    );
    assert_eq!(
        fb1,
        fb_at(RECOMMENDED_TICK_INTERVAL),
        "splash framebuffer must be byte-identical at the SHIPPING batching \
         interval ({RECOMMENDED_TICK_INTERVAL})"
    );
}

/// Walk-deletion output-safety differential: the machine built from the invaders
/// config with the `walk_deleted:` line REMOVED (`None` ⇒ conservative
/// auto-derivation, which for this config KEEPS the walk on because of the native
/// timers/SysTick/ADC/DMA/EXTI the descriptor instantiates) must produce
/// byte-identical observable output — framebuffer AND total_cycles — to the
/// explicit-flag machine (`Some(true)` ⇒ walk deleted), over 20M+ cycles at the
/// same tick interval. This proves the hand `walk_deleted: true` opt-in is
/// output-equivalent to the derived decision (walk-on ≡ walk-off for THIS
/// firmware), i.e. the flag is a pure performance opt-in, safe to keep or drop
/// for correctness.
///
/// It ALSO documents the perf reality honestly: the explicit-flag bus unlocks
/// `max_safe_tick_interval == RECOMMENDED_TICK_INTERVAL` (browser batching),
/// while the flag-removed (derived) bus stays at 1 — the conservative
/// derivation cannot recover the firmware-specific batching win, so removing
/// the flag is a throughput regression even though it is output-safe.
///
/// The expectation reads the CONSTANT, never a literal. It used to assert a
/// bare `64`; `RECOMMENDED_TICK_INTERVAL` later moved to 512 and this gate went
/// on demanding a number nothing ships. Nobody noticed because the test is
/// `#[ignore]`d, so only `--include-ignored` ever runs it.
#[test]
#[ignore = "slow: builds + runs the Space Invaders firmware in-sim"]
fn derived_and_explicit_walk_flag_are_output_identical() {
    let elf = ensure_firmware_built();

    // The differential compares walk-DELETED against walk-KEPT, both stated
    // EXPLICITLY. It used to pit `Some(true)` against `None` (derive) and lean
    // on the derivation choosing walk-kept. The derivation has since caught up
    // and now deletes the walk for this config too — which would have made the
    // comparison two identical buses, i.e. a differential that always passes
    // because it compares a thing to itself. Naming both sides keeps walk-on vs
    // walk-off the thing actually under test, whatever the derivation decides.
    let mut walk_deleted = build_machine_with(&elf, Some(Some(true)));
    let mut walk_kept = build_machine_with(&elf, Some(Some(false)));

    // Honest perf reality: deleting the walk is what unlocks host batching.
    if cfg!(feature = "event-scheduler") {
        assert_eq!(
            walk_deleted.bus.max_safe_tick_interval(),
            RECOMMENDED_TICK_INTERVAL
        );
        assert_eq!(
            walk_kept.bus.max_safe_tick_interval(),
            1,
            "a bus still running the legacy walk must stay cycle-exact"
        );
    }

    // Record — without asserting — what the conservative auto-derivation picks
    // for this config. It is a policy decision that may legitimately move as
    // more peripherals migrate to the scheduler; the OUTPUT-equivalence proved
    // below is what makes any such move safe, and that is what this test gates.
    let derived_interval = build_machine_with(&elf, Some(None))
        .bus
        .max_safe_tick_interval();
    println!("NOKIA_WALK_DERIVATION derived_max_safe_tick_interval={derived_interval}");

    // Run both at interval 1 so the comparison isolates walk-on vs walk-off
    // (not batching). 20M cycles well past the splash + several game frames.
    let budget = 20_000_000u64;
    for m in [&mut walk_deleted, &mut walk_kept] {
        while m.total_cycles < budget {
            let remaining = (budget - m.total_cycles).min(u32::MAX as u64) as u32;
            m.run(Some(remaining)).expect("run");
        }
    }

    assert_eq!(
        walk_deleted.total_cycles, walk_kept.total_cycles,
        "total_cycles must match walk-deleted vs walk-kept"
    );
    let fb_deleted = framebuffer(&walk_deleted);
    let fb_kept = framebuffer(&walk_kept);
    assert!(
        fb_deleted.iter().any(|&b| b != 0),
        "the firmware must have drawn to the framebuffer"
    );
    assert_eq!(
        fb_deleted, fb_kept,
        "framebuffer must be byte-identical: walk deletion is output-safe here"
    );
}

#[test]
#[ignore = "slow: builds + runs the Space Invaders firmware in-sim"]
fn firmware_tracks_minimum_hcsr04_distance() {
    let elf = ensure_firmware_built();
    let mut machine = build_machine(&elf);

    // A user-entered 1 cm value is clamped by the HC-SR04 component to its
    // datasheet minimum, 2 cm. The demo firmware must still observe that as a
    // near reading rather than treating it as a missing echo and holding center.
    machine.bus.hcsr04[0].set_distance_cm(2.0);
    step_frames(&mut machine, 4_000_000);
    let min = ship_left(&framebuffer(&machine)).expect("ship visible (minimum distance)");

    assert!(
        min >= 56,
        "minimum distance should visibly move the paddle right, got min_x={min}"
    );

    machine.bus.hcsr04[0].set_distance_cm(200.0);
    step_frames(&mut machine, 4_000_000);
    let far = ship_left(&framebuffer(&machine)).expect("ship visible (far distance)");

    assert!(
        far <= 8,
        "200 cm should move the paddle to the left edge: min_x={min}, far_x={far}"
    );
}
