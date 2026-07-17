// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF5340 application-core boot conformance: the simulator must satisfy the
//! register sequences Zephyr's nRF drivers spin on, so unmodified firmware
//! reaches `main` instead of hanging.
//!
//! This is the deterministic, ELF-independent twin of
//! `test_nrf5340_zephyr_survival` in `firmware_survival.rs` (which boots a real
//! unmodified Zephyr v3.7 hello_world ELF end to end). It asserts, at the bus
//! level, the three things the boot path depends on:
//!   1. Peripherals are reachable at the NON-secure alias base 0x5000_0000 — the
//!      view the `nrf5340dk/nrf5340/cpuapp` devicetree links against.
//!   2. CLOCK HFCLKSTART/LFCLKSTART raise their STARTED events (the busy-loops in
//!      `clock_control` and `nrf_rtc_timer` init).
//!
//! (The ARMv8-M MPU MAIR0/MAIR1 SCS registers that `z_arm_mpu_init` writes on
//! this Cortex-M33 are covered by a round-trip unit test in
//! `crates/core/src/peripherals/scb.rs`.)
//!
//! Register facts: nRF5340 PS v1.5 (CLOCK §4.x) and the Zephyr DT address map.

use labwired_config::ChipDescriptor;
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Peripherals at the non-secure alias the cpuapp DT uses (peripheral@50000000).
const CLOCK: u64 = 0x5000_5000;
const UART0: u64 = 0x5000_8000;
const RTC1: u64 = 0x5001_5000;

// CLOCK register offsets (nRF5340 PS, shared with nRF52 CLOCK layout).
const TASKS_HFCLKSTART: u64 = 0x000;
const TASKS_LFCLKSTART: u64 = 0x008;
const EVENTS_HFCLKSTARTED: u64 = 0x100;
const EVENTS_LFCLKSTARTED: u64 = 0x104;
const HFCLKSTAT: u64 = 0x40C;
const LFCLKSRC: u64 = 0x518;

fn nrf5340_bus() -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/nrf5340.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    let manifest = labwired_config::SystemManifest {
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf5340-clock-boot".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble nrf5340 bus")
}

#[test]
fn peripherals_mapped_at_nonsecure_alias() {
    // The cpuapp firmware accesses peripherals through 0x5000_0000, not the
    // secure 0x4000_0000 base. A read here must resolve to a peripheral (not
    // fault into open bus), or the console/clock/timer drivers never start.
    let bus = nrf5340_bus();
    for (name, addr) in [("CLOCK", CLOCK), ("UART0", UART0), ("RTC1", RTC1)] {
        assert!(
            bus.read_u32(addr).is_ok(),
            "{name} must be mapped at non-secure alias 0x{addr:08X}"
        );
    }
}

#[test]
fn hfclk_start_raises_started_event() {
    // clock_control_on(HF): write TASKS_HFCLKSTART, then
    //   while (!EVENTS_HFCLKSTARTED) {}
    let mut bus = nrf5340_bus();
    assert_eq!(
        bus.read_u32(CLOCK + EVENTS_HFCLKSTARTED).unwrap(),
        0,
        "HFCLKSTARTED must be clear at reset"
    );

    bus.write_u32(CLOCK + TASKS_HFCLKSTART, 1).unwrap();
    // HFCLKSTAT.STATE (bit 16) reflects the running oscillator immediately.
    assert_ne!(
        bus.read_u32(CLOCK + HFCLKSTAT).unwrap() & (1 << 16),
        0,
        "HFCLKSTAT.STATE must report running after HFCLKSTART"
    );
    // The STARTED event settles on the peripheral tick, the same way silicon
    // raises it a few cycles later.
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u32(CLOCK + EVENTS_HFCLKSTARTED).unwrap(),
        1,
        "HFCLKSTARTED must set — the clock_control spin loop would hang otherwise"
    );
}

#[test]
fn lfclk_start_raises_started_event() {
    // nrf_rtc_timer init starts the LFCLK (RTC1 is LFCLK-clocked), then spins on
    // EVENTS_LFCLKSTARTED.
    let mut bus = nrf5340_bus();
    bus.write_u32(CLOCK + LFCLKSRC, 1).unwrap(); // Xtal source
    bus.write_u32(CLOCK + TASKS_LFCLKSTART, 1).unwrap();
    bus.tick_peripherals_fully();
    assert_eq!(
        bus.read_u32(CLOCK + EVENTS_LFCLKSTARTED).unwrap(),
        1,
        "LFCLKSTARTED must set — nrf_rtc_timer init would hang otherwise"
    );
}
