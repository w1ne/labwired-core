// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// End-to-end coverage for the examples/h563-uds-ecu demo: a real two-node CAN
// bus where the `uds-tester` device (declared in system.yaml) drives UDS
// requests over FDCAN and the committed responder firmware answers with real
// UDSLib. This is the exact lab the `stm32h5-uds-ecu` web playground runs, and
// the LogicAnalyzer's UDS decoder consumes `fdcan_trace_snapshot()` — the very
// API asserted on here.
//
// This test is what keeps that example from silently rotting: the demo once
// shipped a stale firmware whose FDCAN exchange failed, so the playground's
// logic analyzer showed *zero* frames. A regression like that — a broken
// firmware, a dropped tester device, an FDCAN model change that stops the bus —
// fails the merge gate here instead of on the live blog.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::peripherals::fdcan::Fdcan;
use labwired_core::Machine;
use labwired_loader::load_elf;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("canonicalize repo root")
}

/// True if any byte window of `frame` equals `needle`.
fn frame_contains(frame: &[u8], needle: &[u8]) -> bool {
    frame.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn h563_uds_ecu_answers_read_data_by_identifier_on_the_fdcan_trace() {
    let root = repo_root();
    let example = root.join("examples/h563-uds-ecu");

    let program = load_elf(&example.join("firmware/h563_uds_ecu.elf"))
        .expect("load committed responder ELF (run build-firmware / make to regenerate)");

    let manifest =
        SystemManifest::from_file(example.join("system.yaml")).expect("load system manifest");
    let chip_path = example.join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("load chip descriptor at {chip_path:?}"));

    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest)
        .expect("build system bus (incl. the uds-tester device on fdcan1)");

    let uart_sink: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart_sink.clone(), false);

    let (cpu, _nvic) = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&program).expect("load firmware");

    // Read the FDCAN frame trace exactly as the web LogicAnalyzer does:
    // downcast the `fdcan1` peripheral and snapshot its trace ring.
    let fdcan_trace = |machine: &Machine<_>| -> Vec<Vec<u8>> {
        machine
            .bus
            .peripherals
            .iter()
            .filter(|p| p.name == "fdcan1")
            .flat_map(|p| {
                p.dev
                    .as_any()
                    .and_then(|a| a.downcast_ref::<Fdcan>())
                    .map(|fd| fd.trace_snapshot(&p.name))
                    .unwrap_or_default()
            })
            .map(|f| f.data)
            .collect()
    };

    // Step until the ECU has answered the first ReadDataByIdentifier (the bytes
    // the blog caption promises), or give up after a generous budget.
    const MAX_STEPS: usize = 1_000_000;
    let mut saw_response = false;
    for _ in 0..MAX_STEPS {
        if machine.step().is_err() {
            break;
        }
        if fdcan_trace(&machine)
            .iter()
            .any(|f| frame_contains(f, &[0x62, 0xF1, 0x90]))
        {
            saw_response = true;
            break;
        }
    }

    let uart = String::from_utf8_lossy(&uart_sink.lock().unwrap()).to_string();
    assert!(
        uart.contains("ECU_READY"),
        "firmware never reached ECU_READY (boot/FDCAN init failed)\n--- uart ---\n{uart}"
    );

    let frames = fdcan_trace(&machine);
    assert!(
        !frames.is_empty(),
        "FDCAN trace is empty — the logic analyzer would show no UDS data (the regression this guards)\n--- uart ---\n{uart}"
    );
    assert!(
        frames.iter().any(|f| frame_contains(f, &[0x22, 0xF1, 0x90])),
        "no 0x22 F1 90 ReadDataByIdentifier *request* on the bus; the uds-tester did not drive fdcan1\nframes: {frames:02X?}"
    );
    assert!(
        saw_response && frames.iter().any(|f| frame_contains(f, &[0x62, 0xF1, 0x90])),
        "no 0x62 F1 90 positive *response* on the bus; the ECU did not answer over the real CAN bus\nframes: {frames:02X?}\n--- uart ---\n{uart}"
    );
}
