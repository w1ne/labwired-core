// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// End-to-end test for the iolink-dido lab (the playground's "IO-Link DI/DO
// Device" board): the real iolinki DEVICE stack runs as firmware on a simulated
// STM32L476 while the NATIVE Rust `iolink-master` peripheral drives the link
// over USART2 — the exact wiring the web playground's IO-Link Analyzer taps.
//
// The master's schedule is deterministic: it marches to OPERATE and emits
// cyclic frames even against a dead board, finalizing each with ck_ok=false.
// The CLI example test (test.yaml) only asserts the DEVICE's debug prints, so
// a firmware that goes silent on the current engine (e.g. an ELF built before
// RCC clock-gating was modeled) sails through nothing — the analyzer just
// shows a red CK on every cyclic row. This test closes that gap by asserting
// the MASTER's verdict: cyclic frames must carry ck_ok=Some(true) and the
// decoded process data must be the 74HC165 preset.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::peripherals::components::{IolinkLinkState, IolinkMaster, IolinkXfer};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::{Path, PathBuf};

type Cm = Machine<CortexM>;

fn example_root() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/iolink-dido"
    ))
}

// Same contract as world_multichip.rs: missing prebuilt C firmware skips the
// test locally (the workspace gate has no arm-none-eabi/CubeL4), but the
// `iolink-station-l476` board-CI job builds the ELF and sets
// LABWIRED_REQUIRE_IOLINK_ELFS=1, turning "missing" into a hard failure.
fn skip_or_fail_missing_elf(elf: &Path) -> bool {
    if elf.exists() {
        return false;
    }
    if std::env::var_os("LABWIRED_REQUIRE_IOLINK_ELFS").is_some() {
        panic!(
            "required iolink-dido ELF missing while LABWIRED_REQUIRE_IOLINK_ELFS is set; \
             build it: make -C examples/iolink-dido/firmware"
        );
    }
    eprintln!("SKIP: iolink-dido ELF not built; build it: make -C examples/iolink-dido/firmware");
    true
}

fn build_machine(elf: &Path) -> Cm {
    let system_path = example_root().join("system.yaml");
    let manifest = SystemManifest::from_file(&system_path).expect("load system.yaml");
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(elf).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn master_trace(machine: &Cm) -> Vec<IolinkXfer> {
    for p in &machine.bus.peripherals {
        let Some(any) = p.dev.as_any() else { continue };
        let Some(uart) = any.downcast_ref::<labwired_core::peripherals::uart::Uart>() else {
            continue;
        };
        for stream in &uart.attached_streams {
            if let Some(m) = stream
                .as_any()
                .and_then(|a| a.downcast_ref::<IolinkMaster>())
            {
                return m.trace_snapshot();
            }
        }
    }
    panic!("no IolinkMaster attached to any UART");
}

#[test]
fn native_master_reaches_operate_with_green_ck_and_real_pd() {
    let elf = example_root().join("firmware/iolink_dido.elf");
    if skip_or_fail_missing_elf(&elf) {
        return;
    }
    let mut machine = build_machine(&elf);

    // system.yaml presets the 74HC165 inputs to 165 (0xA5); a valid cyclic
    // exchange must decode exactly that as process-data input.
    const EXPECTED_PD: u8 = 0xA5;
    const GOOD_FRAMES_WANTED: usize = 3;

    let mut good = 0usize;
    for i in 0..30_000_000u64 {
        machine.step().expect("step");
        // Trace scanning is cheap only when done sparsely.
        if i % 100_000 != 0 {
            continue;
        }
        let trace = master_trace(&machine);
        good = trace
            .iter()
            .filter(|x| {
                x.link_state == IolinkLinkState::Operate
                    && x.ck_ok == Some(true)
                    && x.pd_valid == Some(true)
                    && x.pd_in.first().copied() == Some(EXPECTED_PD)
            })
            .count();
        if good >= GOOD_FRAMES_WANTED {
            break;
        }
    }

    let trace = master_trace(&machine);
    let cyclic: Vec<_> = trace
        .iter()
        .filter(|x| x.link_state == IolinkLinkState::Operate)
        .collect();
    assert!(
        good >= GOOD_FRAMES_WANTED,
        "master never saw {GOOD_FRAMES_WANTED} checksum-valid cyclic frames with PD={EXPECTED_PD:#04x}; \
         cyclic frames: {} (first few: {:?})",
        cyclic.len(),
        cyclic
            .iter()
            .take(3)
            .map(|x| (x.ck_ok, x.pd_valid, x.pd_in.clone(), x.raw_device.clone()))
            .collect::<Vec<_>>()
    );
}
