// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! GO/NO-GO gate: firmware FDCAN TX crosses `CanBus` to a peer node.
//!
//! Proves that the `h563-uds-ecu` firmware's `0x7E8` UDS response, when
//! `fdcan1` is attached to a `CanBus`, is delivered to a second independent
//! FDCAN observer on the same bus. This exercises the `Some(bus_tx)` TX
//! branch in `peripherals/fdcan.rs` that no firmware had exercised before.
//!
//! SKIP cleanly if the pre-built ELF is absent.
//! Run: cargo test -p labwired-core --test fdcan_firmware_crossing -- --nocapture

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::cortex_m::CortexM;
use labwired_core::network::{CanBus, Interconnect};
use labwired_core::peripherals::fdcan::Fdcan;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::world::MachineTrait;
use labwired_core::{Bus, Machine, Peripheral};
use std::path::PathBuf;

/// Marker address: `g_uds_result` placed at 0x20010000 by the linker script.
const UDS_RESULT_ADDR: u64 = 0x2001_0000;
/// Value written when the ECU successfully sent the `62 F1 90` UDS response.
const UDS_RESULT_OK: u32 = 0x62F1_90A5;

/// FDCAN SRAMCAN offset within the peripheral window.
const RAM: u64 = 0x800;
/// FDCAN CCCR register offset.
const REG_CCCR: u64 = 0x018;
/// FDCAN RXF0S register offset.
const REG_RXF0S: u64 = 0x090;

fn rd(dev: &Fdcan, offset: u64) -> u32 {
    Peripheral::read_u32(dev, offset).unwrap()
}

fn wr(dev: &mut Fdcan, offset: u64, value: u32) {
    Peripheral::write_u32(dev, offset, value).unwrap()
}

/// Leave INIT so the observer can receive frames (reset state has INIT set).
fn observer_start(dev: &mut Fdcan) {
    wr(dev, REG_CCCR, 0x0);
    assert_eq!(rd(dev, REG_CCCR) & 0x1, 0, "observer must leave INIT");
}

#[test]
fn fdcan_firmware_tx_crosses_can_bus_to_peer_observer() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let elf_path = manifest_dir.join(
        "../../examples/h563-uds-ecu/firmware/build/h563_uds_ecu.elf",
    );

    if !elf_path.exists() {
        eprintln!(
            "SKIP fdcan_firmware_crossing: ELF not found at {elf_path:?}. \
             Build with: make -C examples/h563-uds-ecu/firmware"
        );
        return;
    }

    // -- Build the ECU machine ------------------------------------------------
    let system_path = manifest_dir
        .join("../../examples/h563-uds-ecu/system.yaml");
    let sysman =
        SystemManifest::from_file(&system_path).expect("load system.yaml");
    let chip_path = system_path
        .parent()
        .unwrap()
        .join(&sysman.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip");
    let mut bus =
        labwired_core::bus::SystemBus::from_config(&chip, &sysman)
            .expect("build SystemBus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf_path).expect("parse ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine.reset().expect("reset machine");

    // -- Wire fdcan1 → CanBus -------------------------------------------------
    let mut can = CanBus::new();
    let (tx_a, rx_a) = can.attach();
    machine
        .attach_can_bus("fdcan1", tx_a, rx_a)
        .expect("attach_can_bus fdcan1");

    // -- Observer node B: bare Fdcan on the same bus --------------------------
    let (tx_b, rx_b) = can.attach();
    let mut observer = Fdcan::new_with_bus(tx_b, rx_b);
    observer_start(&mut observer);

    // -- Drive the simulation -------------------------------------------------
    const MAX_ITER: u32 = 300_000;
    let mut observed_frame_id: Option<u32> = None;
    let mut uds_result_at_halt: u32 = 0;

    for iter in 0..MAX_ITER {
        machine.step().expect("step ECU machine");
        can.tick().expect("CanBus tick");
        observer.tick();

        // Check whether the observer received any frame.
        if observed_frame_id.is_none() {
            let rxf0s = rd(&observer, REG_RXF0S);
            let fill = rxf0s & 0x7F;
            if fill > 0 {
                // Read frame id from RX FIFO0 element 0 (R0, bits [28:18] = std id).
                let r0 = rd(&observer, RAM + 0xB0);
                let id = (r0 >> 18) & 0x7FF;
                eprintln!(
                    "Observer received frame at iter {iter}: id=0x{id:03X} \
                     fill={fill}"
                );
                observed_frame_id = Some(id);
                // Capture the ECU result marker while we're here.
                uds_result_at_halt = read_u32_le(&machine, UDS_RESULT_ADDR);
                break;
            }
        }
    }

    if observed_frame_id.is_none() {
        // Diagnostic chain: did the ECU even produce a response?
        uds_result_at_halt = read_u32_le(&machine, UDS_RESULT_ADDR);
        eprintln!(
            "NO-GO: observer never received any frame after {MAX_ITER} iterations."
        );
        eprintln!(
            "  ECU UDS result marker @ 0x{UDS_RESULT_ADDR:08X} = 0x{uds_result_at_halt:08X} \
             (expected 0x{UDS_RESULT_OK:08X} for a completed response)"
        );
        if uds_result_at_halt != UDS_RESULT_OK {
            eprintln!(
                "  ECU did NOT complete its UDS response — the firmware never \
                 reached the TX path. Chain break: firmware did not TX."
            );
        } else {
            eprintln!(
                "  ECU DID complete its UDS response (marker = OK), \
                 but TX did not reach the observer. \
                 Chain break: bus_tx send or CanBus tick or observer RX drain."
            );
        }
        panic!(
            "NO-GO: FDCAN firmware TX did not cross to the observer node. \
             ECU marker=0x{uds_result_at_halt:08X}"
        );
    }

    let frame_id = observed_frame_id.unwrap();
    eprintln!(
        "GO: observer received frame with id=0x{frame_id:03X} \
         (ECU UDS result marker=0x{uds_result_at_halt:08X})"
    );

    assert_eq!(
        frame_id, 0x7E8,
        "Expected the ECU's UDS response on id 0x7E8, got 0x{frame_id:03X}"
    );
}

/// Read a little-endian `u32` from machine memory by reading four bytes
/// via the `Bus` trait (the CPU access path).
fn read_u32_le(machine: &Machine<CortexM>, addr: u64) -> u32 {
    let b0 = Bus::read_u8(&machine.bus, addr).unwrap_or(0) as u32;
    let b1 = Bus::read_u8(&machine.bus, addr + 1).unwrap_or(0) as u32;
    let b2 = Bus::read_u8(&machine.bus, addr + 2).unwrap_or(0) as u32;
    let b3 = Bus::read_u8(&machine.bus, addr + 3).unwrap_or(0) as u32;
    b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
}
