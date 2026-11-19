// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

use labwired_config::{MotorModelConfig, SystemManifest};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};

fn bldc_fixture_target_dir() -> PathBuf {
    std::env::temp_dir()
        .join("labwired-fixtures")
        .join("stm32l476-bldc")
}

fn ensure_bldc_firmware_built() -> PathBuf {
    static ELF: OnceLock<PathBuf> = OnceLock::new();
    ELF.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let target_dir = bldc_fixture_target_dir();
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "-p",
                "firmware-l476-demo",
                "--bin",
                "firmware-l476-bldc-six-step",
                "--target",
                "thumbv7em-none-eabihf",
                "--target-dir",
            ])
            .arg(&target_dir)
            .env_remove("CARGO_TARGET_DIR")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .current_dir(&root)
            .status()
            .expect("invoke cargo build for STM32L476 BLDC fixture");
        assert!(status.success(), "STM32L476 BLDC firmware build failed");
        let elf = target_dir.join("thumbv7em-none-eabihf/release/firmware-l476-bldc-six-step");
        assert!(elf.is_file(), "firmware ELF was not produced at {elf:?}");
        elf
    })
    .clone()
}

#[test]
fn l476_fixture_target_is_independent_of_outer_cargo_target_dir() {
    let dedicated = bldc_fixture_target_dir();
    let inherited = PathBuf::from("/tmp/custom-outer-cargo-target");
    assert_ne!(dedicated, inherited);
    assert!(dedicated.ends_with("labwired-fixtures/stm32l476-bldc"));
    assert!(dedicated
        .join("thumbv7em-none-eabihf/release/firmware-l476-bldc-six-step")
        .starts_with(&dedicated));
}

fn fixture(
    stalled: bool,
) -> (
    Machine<labwired_core::cpu::cortex_m::CortexM>,
    Arc<Mutex<Vec<u8>>>,
) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let resolved = labwired_config::ResolvedSystem::from_manifest_file(
        &root.join("examples/nucleo-l476rg-bldc/system.yaml"),
    )
    .unwrap();
    let mut bus = labwired_core::system::builder::build_system_bus(Some(&resolved)).unwrap();
    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let elf = ensure_bldc_firmware_built();
    let image = labwired_loader::load_elf(&elf)
        .unwrap_or_else(|error| panic!("build firmware fixture first ({elf:?}): {error}"));
    machine.load_firmware(&image).unwrap();
    if stalled {
        machine.set_input_on("drive_motor", "stall", 1.0).unwrap();
    }
    (machine, uart)
}

fn contains(uart: &Arc<Mutex<Vec<u8>>>, token: &[u8]) -> bool {
    uart.lock()
        .unwrap()
        .windows(token.len())
        .any(|bytes| bytes == token)
}

#[test]
fn l476_showcase_does_not_bind_aggregate_motor_fault_as_stall() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let manifest =
        SystemManifest::from_file(root.join("examples/nucleo-l476rg-bldc/system.yaml")).unwrap();
    let models = manifest.resolved_motor_models().unwrap();
    let MotorModelConfig::Bldc(motor) = &models[0] else {
        panic!("expected BLDC model")
    };
    assert_eq!(motor.motor_fault_pin, None);
}

#[test]
fn l476_bldc_stall_runs_real_firmware_and_disables_real_inverter() {
    let (mut machine, uart) = fixture(false);
    for _ in 0..500_000 {
        machine.step().unwrap();
        if contains(&uart, b"TARGET REACHED") {
            break;
        }
    }
    assert!(
        contains(&uart, b"TARGET REACHED"),
        "uart={} snapshot={:?}",
        String::from_utf8_lossy(&uart.lock().unwrap()),
        (
            machine.bus.motor_snapshots(),
            machine.bus.current_cycle,
            machine.bus.read_u32(0x4001_2c20).unwrap(),
            machine.bus.read_u32(0x4001_2c34).unwrap(),
            machine.bus.read_u32(0x4001_2c3c).unwrap(),
            machine.bus.read_u32(0xe000_e010).unwrap()
        )
    );
    let acquired = machine.bus.motor_snapshots().remove(0);
    assert!(
        acquired.speed_rpm.abs() > 1.0,
        "target token requires plant motion"
    );

    let injected_cycle = machine.bus.current_cycle;
    machine.set_input_on("drive_motor", "stall", 1.0).unwrap();
    for _ in 0..500_000 {
        machine.step().unwrap();
        if contains(&uart, b"INVERTER OFF") {
            break;
        }
    }
    let shutdown_cycle = machine.bus.current_cycle;
    assert!(contains(&uart, b"FAULT STALL"));
    assert!(contains(&uart, b"INVERTER OFF"));
    assert!(shutdown_cycle - injected_cycle <= 300_000);
    let stopped = machine.bus.motor_snapshots().remove(0);
    assert!(stopped.faults.iter().any(|fault| fault == "stalled"));
    assert_eq!(machine.bus.read_u32(0x4001_2c44).unwrap() & (1 << 15), 0);
    assert_eq!(machine.bus.read_u32(0x4800_0414).unwrap() & 1, 0);
}

#[test]
fn l476_bldc_never_reports_target_without_rotor_motion() {
    let (mut machine, uart) = fixture(true);
    for _ in 0..500_000 {
        machine.step().unwrap();
        if contains(&uart, b"INVERTER OFF") {
            break;
        }
    }
    assert!(contains(&uart, b"FAULT STALL"));
    assert!(!contains(&uart, b"TARGET REACHED"));
    assert_eq!(machine.bus.motor_snapshots()[0].speed_rpm, 0.0);
}

// The exact motor-fault path available to Studio is also a deterministic
// SimInput stimulus: no invented UART logs, no host poking CPU registers.
#[test]
fn l476_bldc_inverter_fault_sim_input_disables_inverter() {
    let (mut machine, uart) = fixture(false);
    let inputs = machine.bus.list_inputs();
    assert!(
        inputs
            .iter()
            .any(|(name, channel)| { name == "drive_motor" && channel.key == "inverter_fault" }),
        "BLDC inverter fault must be discoverable via list_inputs"
    );
    let channel = machine
        .bus
        .resolve_input(Some("drive_motor"), "inverter_fault")
        .expect("the same input must resolve before running");
    assert_eq!(channel.min, 0.0);
    assert_eq!(channel.max, 1.0);

    for _ in 0..500_000 {
        machine.step().unwrap();
        if contains(&uart, b"TARGET REACHED") {
            break;
        }
    }
    assert!(
        contains(&uart, b"TARGET REACHED"),
        "motor never reached the target"
    );

    let injected_cycle = machine.bus.current_cycle;
    machine
        .set_input_on("drive_motor", "inverter_fault", 1.0)
        .expect("scripted driver fault must be accepted");
    for _ in 0..500_000 {
        machine.step().unwrap();
        if contains(&uart, b"INVERTER OFF") {
            break;
        }
    }
    let latency = machine.bus.current_cycle - injected_cycle;
    assert!(
        contains(&uart, b"FAULT DRIVER"),
        "UART after injection: {}",
        String::from_utf8_lossy(&uart.lock().unwrap())
    );
    assert!(contains(&uart, b"INVERTER OFF"));
    assert!(latency <= 310_000, "shutdown took {latency} cycles");
    assert_eq!(
        machine.bus.read_u32(0x4001_2c44).unwrap() & (1 << 15),
        0,
        "TIM1.MOE"
    );
    assert_eq!(
        machine.bus.read_u32(0x4800_0414).unwrap() & 1,
        0,
        "PB0 enable"
    );
}
