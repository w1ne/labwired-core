// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test for the nrf52840-proximity-lab: the same firmware ELF that
// flashes to a real nRF52840 must, in the simulator, range the HC-SR04 and
// raise the ALARM line (P0.06) when the host-controlled target distance is
// inside the 500 mm (50 cm) threshold — and drop it when the target moves away.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::Machine;
use std::path::{Path, PathBuf};
use std::process::Command;

type Cm = Machine<CortexM>;

const GPIO0_OUT: u64 = 0x5000_0504;
const ALARM: u32 = 1 << 6; // P0.06

fn ensure_firmware_built() -> PathBuf {
    let elf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/thumbv7em-none-eabi/release/firmware-nrf52840-proximity");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/firmware-nrf52840-proximity/src/main.rs");
    if let (Ok(e), Ok(s)) = (std::fs::metadata(&elf), std::fs::metadata(&src)) {
        if e.modified().unwrap() >= s.modified().unwrap() {
            return elf;
        }
    }
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "firmware-nrf52840-proximity",
            "--target",
            "thumbv7em-none-eabi",
            "--release",
        ])
        // See e2e_epaper_tricolor: clear coverage instrumentation flags so the
        // no_std firmware cross-build doesn't fail with E0463 under llvm-cov.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .current_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .status()
        .expect("cargo build firmware-nrf52840-proximity");
    assert!(status.success(), "firmware-nrf52840-proximity build failed");
    assert!(elf.exists(), "ELF not found at {elf:?}");
    elf
}

fn build_machine(elf: &Path) -> Cm {
    let system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/nrf52840-proximity-lab/system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load manifest");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn run_steps(machine: &mut Cm, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

fn alarm_high(machine: &Cm) -> bool {
    machine.bus.read_u32(GPIO0_OUT).expect("read GPIO0 OUT") & ALARM != 0
}

// Slow path is the firmware build; run with:
//   cargo test -p labwired-core --test e2e_nrf52840_proximity -- --ignored --nocapture
#[test]
#[ignore = "slow: builds + runs the proximity firmware in-sim"]
fn alarm_tracks_target_distance() {
    let elf = ensure_firmware_built();
    let mut machine = build_machine(&elf);

    // Near target (system.yaml default 10 cm = 100 mm < 500 mm threshold):
    // the firmware must raise ALARM.
    run_steps(&mut machine, 3_000_000);
    assert!(
        alarm_high(&machine),
        "ALARM (P0.06) should be HIGH with the target at 10 cm"
    );

    // Move the target out of range (100 cm > 50 cm threshold); the next ranging
    // cycles must drop ALARM.
    machine.bus.hcsr04[0].set_distance_cm(100.0);
    run_steps(&mut machine, 3_000_000);
    assert!(
        !alarm_high(&machine),
        "ALARM (P0.06) should be LOW with the target at 100 cm"
    );

    // And bring it back in range.
    machine.bus.hcsr04[0].set_distance_cm(8.0);
    run_steps(&mut machine, 3_000_000);
    assert!(
        alarm_high(&machine),
        "ALARM (P0.06) should re-arm when the target returns to 8 cm"
    );
}
