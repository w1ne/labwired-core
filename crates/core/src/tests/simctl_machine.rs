// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! End-to-end proof for the `simctl` device: **real Thumb firmware ends its own
//! run and the machine reports the firmware's verdict.**
//!
//! The unit tests in [`crate::peripherals::simctl`] exercise the device in
//! isolation — they would pass even if nothing ever drained it. These tests go
//! through the whole path a customer's firmware takes: an `STR` to an MMIO
//! address, the bus write, the advance-loop drain, and the returned
//! [`AdvanceStop`].
//!
//! [`a_bus_without_simctl_never_stops_early`] is the anti-vacuity control: the
//! identical program on a bus with **no** `simctl` must run to the fuel limit.
//! Without it, a test that stopped for any unrelated reason would still be
//! green.

use crate::cpu::CortexM;
use crate::machine::AdvanceStop;
use crate::peripherals::simctl::{SimCtl, WINDOW};
use crate::system::cortex_m::configure_cortex_m;
use crate::{AdvanceRequest, Bus, Machine};

/// Where the device is mapped in these tests.
const SIMCTL_BASE: u64 = 0x5000_0000;
/// `EXIT` register, per the device's register map.
const EXIT_OFFSET: u64 = 0x00;

/// Assemble the test program: store `code` to `SIMCTL_BASE + reg_offset`, then
/// spin forever.
///
/// ```text
///   0x0000  LDR  r0, [pc, #12]   ; r0 = target MMIO address (literal @ 0x10)
///   0x0002  LDR  r1, [pc, #16]   ; r1 = value            (literal @ 0x14)
///   0x0004  STR  r1, [r0, #0]    ; the write that ends the run
///   0x0006  B    .               ; spin — reaching here means simctl did nothing
///   0x0010  .word target
///   0x0014  .word value
/// ```
///
/// The trailing spin matters: it means "the run ended" can only be caused by
/// the device, never by the program falling off the end.
fn load_store_to_simctl_program(machine: &mut Machine<CortexM>, target: u64, value: u32) {
    machine.bus.write_u16(0x0000, 0x4803).unwrap(); // LDR r0, [pc, #12]
    machine.bus.write_u16(0x0002, 0x4904).unwrap(); // LDR r1, [pc, #16]
    machine.bus.write_u16(0x0004, 0x6001).unwrap(); // STR r1, [r0]
    machine.bus.write_u16(0x0006, 0xE7FE).unwrap(); // B . (spin)
    machine.bus.write_u32(0x0010, target as u32).unwrap();
    machine.bus.write_u32(0x0014, value).unwrap();
    machine.cpu.pc = 0x0000;
}

/// A Cortex-M machine, optionally carrying the `simctl` device.
fn machine(with_simctl: bool) -> Machine<CortexM> {
    let mut bus = crate::bus::SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    if with_simctl {
        bus.add_peripheral("simctl", SIMCTL_BASE, WINDOW, None, Box::new(SimCtl::new()));
    }
    Machine::new(cpu, bus)
}

#[test]
fn firmware_ends_its_own_run_with_a_pass() {
    let mut m = machine(true);
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + EXIT_OFFSET, 0);

    let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();

    assert_eq!(
        report.stop,
        AdvanceStop::FirmwareExit { code: 0 },
        "firmware wrote EXIT 0; the run must end carrying that verdict"
    );
}

#[test]
fn a_nonzero_exit_code_reaches_the_harness_intact() {
    // The value the harness cares about most: a specific failure code, not a
    // truncated first byte.
    let mut m = machine(true);
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + EXIT_OFFSET, 0x0000_002A);

    let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();

    assert_eq!(report.stop, AdvanceStop::FirmwareExit { code: 42 });
}

#[test]
fn a_bus_without_simctl_never_stops_early() {
    // ANTI-VACUITY CONTROL. Same program, same addresses, no device: the store
    // lands in unmapped space and the firmware spins until it runs out of fuel.
    // If this test ever starts reporting a firmware verdict, the ones above are
    // proving nothing about the device.
    let mut m = machine(false);
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + EXIT_OFFSET, 0);

    let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();

    assert_eq!(
        report.stop,
        AdvanceStop::FuelLimit,
        "with no simctl on the bus nothing may end the run early"
    );
}

#[test]
fn the_run_ends_at_the_store_not_later() {
    // The verdict must be observed at the boundary of the instruction that
    // wrote it. The program spins forever afterwards, so a late drain would
    // burn the whole 64-unit budget; an on-time one costs a handful.
    let mut m = machine(true);
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + EXIT_OFFSET, 0);

    let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();

    assert!(
        matches!(report.stop, AdvanceStop::FirmwareExit { .. }),
        "expected a firmware verdict, got {:?}",
        report.stop
    );
    assert!(
        report.primary_steps <= 8,
        "the run should end at the store (3 instructions in), not spin; \
         took {} steps",
        report.primary_steps
    );
}

#[test]
fn the_debug_control_surface_reports_the_firmware_verdict() {
    // `DebugControl::run` is the interactive/DAP path. A firmware exit there
    // must not be flattened into a generic "step done".
    use crate::{DebugControl, StopReason};

    let mut m = machine(true);
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + EXIT_OFFSET, 3);

    let reason = m.run(Some(64)).unwrap();

    assert_eq!(reason, StopReason::FirmwareExit(3));
}

#[test]
fn firmware_output_streams_are_readable_after_the_run() {
    // SOUT is a channel distinct from UART: a harness reads it off the device
    // once the run has ended.
    let mut m = machine(true);
    // SOUT lives at 0x10.
    load_store_to_simctl_program(&mut m, SIMCTL_BASE + 0x10, u32::from(b'K'));

    // The store to SOUT is not a verdict, so this runs to the fuel limit.
    let report = m.advance(AdvanceRequest::run(Some(64))).unwrap();
    assert_eq!(report.stop, AdvanceStop::FuelLimit);

    assert_eq!(
        m.simctl().expect("device is on the bus").stdout(),
        b"K",
        "the byte firmware wrote to SOUT must be readable from the device"
    );
}

/// The declaration path: `type: simctl` in a chip descriptor must actually
/// build the device, with its config knobs applied.
///
/// The tests above hand-attach the device, so they would stay green even if the
/// factory arm routed `simctl` to a stub — the failure a user would actually
/// hit when they write the YAML.
mod from_declaration {
    use super::*;
    use labwired_config::{Arch, ChipDescriptor, MemoryRange, PeripheralConfig, SystemManifest};
    use std::collections::HashMap;

    fn chip_declaring_simctl() -> ChipDescriptor {
        ChipDescriptor {
            schema_version: "1.0".to_string(),
            name: "simctl-test-chip".to_string(),
            arch: Arch::Arm,
            core: None,
            flash: MemoryRange {
                base: 0x0,
                size: "128KB".to_string(),
            },
            ram: MemoryRange {
                base: 0x2000_0000,
                size: "20KB".to_string(),
            },
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            peripherals: vec![PeripheralConfig {
                id: "simctl".to_string(),
                r#type: "simctl".to_string(),
                base_address: SIMCTL_BASE,
                size: None,
                irq: None,
                clock: None,
                config: HashMap::new(),
            }],
            pins: Default::default(),
        }
    }

    fn manifest() -> SystemManifest {
        SystemManifest {
            parts: Vec::new(),
            walk_deleted: Some(false),
            schema_version: "1.0".to_string(),
            name: "simctl-test-system".to_string(),
            chip: "simctl-test-chip".to_string(),
            memory_overrides: HashMap::new(),
            external_devices: Vec::new(),
            cosim_models: Vec::new(),
            motor_models: Vec::new(),
            board_io: Vec::new(),
            debug_uart: None,
            wifi_ap: None,
            peripherals: Vec::new(),
        }
    }

    fn built() -> crate::bus::SystemBus {
        crate::bus::SystemBus::from_config(&chip_declaring_simctl(), &manifest()).unwrap()
    }

    fn device(bus: &crate::bus::SystemBus) -> &SimCtl {
        bus.peripherals[0]
            .dev
            .as_any()
            .and_then(|a| a.downcast_ref::<SimCtl>())
            .expect("`type: simctl` must build a SimCtl, not a stub")
    }

    #[test]
    fn declaring_the_type_builds_the_real_device() {
        let bus = built();
        assert_eq!(bus.peripherals.len(), 1);
        let _ = device(&bus);
    }
}
