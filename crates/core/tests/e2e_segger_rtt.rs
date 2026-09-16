// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end proof that a stock SEGGER_RTT.c firmware's RAM control block is
//! drained by the simulator exactly like a probe would drain it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

fn ensure_firmware_built(root: &std::path::Path) -> PathBuf {
    let bin = labwired_core::test_support::target_dir()
        .join("thumbv7em-none-eabi/release/firmware-nrf52840-rtt");
    let status = Command::new("cargo")
        .current_dir(root)
        .args([
            "build",
            "-p",
            "firmware-nrf52840-rtt",
            "--release",
            "--target",
            "thumbv7em-none-eabi",
        ])
        // See e2e_epaper_tricolor: clear coverage instrumentation flags so the
        // no_std firmware cross-build doesn't fail with E0463 under llvm-cov.
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .status()
        .expect("execute cargo build");
    assert!(
        status.success(),
        "failed to build firmware-nrf52840-rtt (needs gcc-arm-none-eabi)"
    );
    assert!(bin.exists(), "expected binary at {:?}", bin);
    bin
}

#[test]
fn stock_segger_rtt_firmware_output_is_drained_from_ram() {
    let root = repo_root();
    let elf_path = ensure_firmware_built(&root);
    let elf_bytes = std::fs::read(&elf_path).expect("read ELF");

    let chip = labwired_config::ChipDescriptor::from_file(root.join("configs/chips/nrf52840.yaml"))
        .expect("nrf52840 chip");
    let manifest: labwired_config::SystemManifest =
        serde_yaml::from_str("name: rtt-e2e\nchip: ignored\n").expect("manifest");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

    let control_block = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "_SEGGER_RTT");
    assert!(
        control_block.is_some(),
        "firmware ELF must export _SEGGER_RTT"
    );
    bus.attach_segger_rtt(control_block);

    let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
    assert!(bus.attach_rtt_sink(Some(sink.clone()), false));

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let image = labwired_loader::load_elf(&elf_path).expect("load ELF");
    machine.load_firmware(&image).expect("load firmware");

    for _ in 0..5_000_000u64 {
        machine.step().expect("simulator step");
        if String::from_utf8_lossy(&sink.lock().unwrap()).contains("RTT hello from labwired") {
            break;
        }
    }

    let captured = String::from_utf8_lossy(&sink.lock().unwrap()).to_string();
    assert!(
        captured.contains("RTT hello from labwired"),
        "expected RTT output, captured {captured:?}"
    );
    let status = machine.bus.segger_rtt_status().expect("rtt status");
    assert!(status.control_block_found);
    assert!(status.bytes_drained >= 24);
}

// Runs in the feature-enabled lanes only — `cargo test -p labwired-core
// --features jit,event-scheduler` (core-full/nightly). PR shards skip it.
#[cfg(all(feature = "jit", feature = "event-scheduler"))]
mod jit_overflow {
    use super::*;
    use labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
    use labwired_core::DebugControl;

    const PATTERN: &[u8] = b"BLOCK16:0123456789ABCDEF\n";

    fn ensure_blocking_fixture_built(root: &std::path::Path) -> PathBuf {
        let bin = labwired_core::test_support::target_dir()
            .join("thumbv7em-none-eabi/release/firmware-nrf52840-rtt-blocking");
        let status = Command::new("cargo")
            .current_dir(root)
            .args([
                "build",
                "-p",
                "firmware-nrf52840-rtt-blocking",
                "--release",
                "--target",
                "thumbv7em-none-eabi",
            ])
            // See e2e_epaper_tricolor: clear coverage instrumentation flags so
            // the no_std firmware cross-build doesn't fail with E0463 under
            // llvm-cov.
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .status()
            .expect("execute cargo build");
        assert!(
            status.success(),
            "failed to build firmware-nrf52840-rtt-blocking (needs gcc-arm-none-eabi)"
        );
        assert!(bin.exists(), "expected binary at {:?}", bin);
        bin
    }

    #[test]
    fn blocking_mode_progresses_and_stays_byte_exact_under_jit_windows() {
        let root = repo_root();
        let elf_path = ensure_blocking_fixture_built(&root);
        let elf_bytes = std::fs::read(&elf_path).expect("read ELF");

        let chip =
            labwired_config::ChipDescriptor::from_file(root.join("configs/chips/nrf52840.yaml"))
                .expect("nrf52840 chip");
        let manifest: labwired_config::SystemManifest =
            serde_yaml::from_str("name: rtt-blocking-e2e\nchip: ignored\n").expect("manifest");
        let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");

        let control_block = labwired_loader::resolve_symbol_in_elf(&elf_bytes, "_SEGGER_RTT");
        assert!(
            control_block.is_some(),
            "firmware ELF must export _SEGGER_RTT"
        );
        bus.attach_segger_rtt(control_block);

        let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
        assert!(bus.attach_rtt_sink(Some(sink.clone()), false));

        let (cpu, _nvic) = configure_cortex_m(&mut bus);
        let mut machine = Machine::new(cpu, bus);
        let image = labwired_loader::load_elf(&elf_path).expect("load ELF");
        machine.load_firmware(&image).expect("load firmware");

        machine.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
        machine.bus.config.peripheral_tick_interval = RECOMMENDED_TICK_INTERVAL;
        machine.config.cortex_m_jit_enabled = true;
        machine.bus.config.cortex_m_jit_enabled = true;
        machine.bus.legacy_walk_disabled = true;

        let target_bytes = PATTERN.len() * 200;
        let max_steps: u64 = 2_000_000;
        let mut steps = 0u64;
        while sink.lock().unwrap().len() < target_bytes && steps < max_steps {
            machine.run(Some(64)).expect("simulator run");
            steps += 64;
        }

        let captured = sink.lock().unwrap().clone();
        assert!(
            captured.len() >= target_bytes,
            "blocking firmware made no progress in {max_steps} steps: \
             captured {} of {target_bytes} bytes",
            captured.len()
        );
        for (i, byte) in captured.iter().enumerate() {
            assert_eq!(
                *byte,
                PATTERN[i % PATTERN.len()],
                "byte {i} diverged: got {byte:#04x}, want {:#04x} \
                 (captured {} bytes)",
                PATTERN[i % PATTERN.len()],
                captured.len()
            );
        }
        let status = machine.bus.segger_rtt_status().expect("rtt status");
        assert!(status.control_block_found);
        assert_eq!(status.bytes_drained, captured.len() as u64);

        let stats = machine
            .cpu
            .jit_stats()
            .expect("JIT engine was never created");
        assert!(
            stats.block_runs > 0,
            "JIT never ran a compiled block: {stats:?}"
        );
    }
}
