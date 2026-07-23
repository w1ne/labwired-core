// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! RP2040 PIO executing-fidelity golden test.
//!
//! The PIO block is the RP2040's signature peripheral, and the shipped
//! `rp2040-pio` onboarding lab (`examples/rp2040-pio/io-smoke.yaml`) drives it.
//! This test loads that lab's real compiled firmware
//! (`firmware-rp2040-pio-onboarding`) onto the full RP2040 chip bus and runs it,
//! asserting the firmware's own PIO self-check reaches its success verdict over
//! the UART console.
//!
//! What the firmware exercises, end to end, through the modelled PIO
//! (`peripherals/pio.rs`) — none of it reachable by a register poke:
//!   1. writes a 2-instruction PIO program (`SET X,10`; `PULL block`) into
//!      INSTR_MEM,
//!   2. configures SM0 wrap and enables the state machine (CTRL.SM0),
//!   3. the state machine executes `SET`, advances PC, and **stalls on `PULL`**
//!      with an empty TX FIFO,
//!   4. the CPU pushes a word into TXF0, unblocking the stalled `PULL`,
//!   5. the state machine consumes the FIFO word, which the firmware confirms by
//!      reading FSTAT.TXEMPTY(SM0) and printing `PIO_OK`.
//!
//! `PIO_FAIL` (the FIFO never drained → the state machine never ran the program)
//! is asserted-against explicitly, so a broken PIO cannot pass by emitting the
//! failure banner.
//!
//! The fixture is built by the core-ci "Build test firmware fixture" step:
//! ```text
//! RUSTFLAGS="-C link-arg=-Tlink.x" cargo build -p firmware-rp2040-pio-onboarding \
//!     --release --target thumbv6m-none-eabi
//! ```
//! When it is absent (a plain `cargo test` without that pre-build) the test
//! skips with a notice rather than failing spuriously.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rp2040_chip() -> (ChipDescriptor, SystemManifest) {
    let chip_path = workspace_root().join("configs/chips/rp2040.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|e| panic!("load rp2040 chip {chip_path:?}: {e}"));
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "rp2040-pio-onboarding".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    (chip, manifest)
}

#[test]
fn rp2040_pio_onboarding_reaches_pio_ok() {
    let firmware =
        workspace_root().join("target/thumbv6m-none-eabi/release/firmware-rp2040-pio-onboarding");
    if !firmware.exists() {
        eprintln!(
            "SKIP rp2040_pio_onboarding_reaches_pio_ok: fixture not built at {firmware:?}. \
             Build it with: RUSTFLAGS=\"-C link-arg=-Tlink.x\" cargo build \
             -p firmware-rp2040-pio-onboarding --release --target thumbv6m-none-eabi"
        );
        return;
    }

    let (chip, manifest) = rp2040_chip();
    // Bare-metal PIO smoke links at low VMA and uses Cortex-M flash boot alias
    // at 0; empty env opts out of the in-tree mask ROM so alias wins.
    std::env::set_var("LABWIRED_RP2040_BOOTROM", "");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build RP2040 bus");
    let uart_sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&firmware)
        .unwrap_or_else(|e| panic!("load ELF {firmware:?}: {e}"));
    machine.load_firmware(&image).expect("load firmware");

    // The onboarding lab caps at 10k steps; give a generous budget and stop as
    // soon as the firmware has printed its verdict.
    const MAX_STEPS: u32 = 200_000;
    for step in 0..MAX_STEPS {
        machine
            .step()
            .unwrap_or_else(|e| panic!("sim crashed at step {step}: {e}"));
        if step % 512 == 0 {
            let out = uart_sink.lock().unwrap();
            if out.windows(6).any(|w| w == b"PIO_OK") || out.windows(8).any(|w| w == b"PIO_FAIL") {
                break;
            }
        }
    }

    let out = uart_sink.lock().unwrap().clone();
    let text = String::from_utf8_lossy(&out);
    assert!(
        !text.contains("PIO_FAIL"),
        "PIO state machine failed its self-check (FIFO never drained). UART: {text:?}"
    );
    assert!(
        text.contains("PIO_OK"),
        "RP2040 PIO onboarding firmware did not reach PIO_OK. UART: {text:?}"
    );
}
