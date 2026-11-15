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
//!   1. Peripherals are reachable at the secure base 0x5000_0000 — the view
//!      the `nrf5340dk/nrf5340/cpuapp` devicetree links against. The `/ns`
//!      target uses the 0x4000_0000 aliases and is a separate compile profile.
//!   2. CLOCK HFCLKSTART/LFCLKSTART raise their STARTED events (the busy-loops in
//!      `clock_control` and `nrf_rtc_timer` init).
//!
//! (The ARMv8-M MPU MAIR0/MAIR1 SCS registers that `z_arm_mpu_init` writes on
//! this Cortex-M33 are covered by a round-trip unit test in
//! `crates/core/src/peripherals/scb.rs`.)
//!
//! Register facts: nRF5340 PS v1.5 (CLOCK §4.x) and the Zephyr DT address map.

use labwired_config::{ChipDescriptor, ExternalDevice};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

// Peripherals at the secure cpuapp view (the `/ns` target uses 0x4000_0000).
const CLOCK: u64 = 0x5000_5000;
const UART0: u64 = 0x5000_8000;
const I2C1: u64 = 0x5000_9000;
const RTC1: u64 = 0x5001_5000;
const RAM: u64 = 0x2000_0000;

// Shared Nordic TWIM register offsets. nRF5340 TWIM1_S derives from TWIM0_NS
// in the vendor SVD and preserves these nRF52 offsets.
const TWIM_TASKS_STARTTX: u64 = 0x008;
const TWIM_EVENTS_LASTRX: u64 = 0x15C;
const TWIM_SHORTS: u64 = 0x200;
const TWIM_ERRORSRC: u64 = 0x4C4;
const TWIM_ENABLE: u64 = 0x500;
const TWIM_PSEL_SCL: u64 = 0x508;
const TWIM_PSEL_SDA: u64 = 0x50C;
const TWIM_RXD_PTR: u64 = 0x534;
const TWIM_RXD_MAXCNT: u64 = 0x538;
const TWIM_TXD_PTR: u64 = 0x544;
const TWIM_TXD_MAXCNT: u64 = 0x548;
const TWIM_ADDRESS: u64 = 0x588;
const TWIM_SHORT_LASTTX_STARTRX: u32 = 1 << 7;
const TWIM_SHORT_LASTRX_STOP: u32 = 1 << 12;

// CLOCK register offsets (nRF5340 PS, shared with nRF52 CLOCK layout).
const TASKS_HFCLKSTART: u64 = 0x000;
const TASKS_LFCLKSTART: u64 = 0x008;
const EVENTS_HFCLKSTARTED: u64 = 0x100;
const EVENTS_LFCLKSTARTED: u64 = 0x104;
const HFCLKSTAT: u64 = 0x40C;
const LFCLKSRC: u64 = 0x518;

fn nrf5340_bus_with(external_devices: Vec<ExternalDevice>) -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/nrf5340.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load nrf5340 chip");
    let manifest = labwired_config::SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "nrf5340-clock-boot".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices,
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("assemble nrf5340 bus")
}

fn nrf5340_bus() -> SystemBus {
    nrf5340_bus_with(Vec::new())
}

#[test]
fn peripherals_mapped_at_secure_cpuapp_view() {
    // The secure cpuapp firmware accesses peripherals through 0x5000_0000.
    // A read here must resolve to a peripheral (not fault into open bus), or
    // the console/clock/timer/I2C drivers never start.
    let bus = nrf5340_bus();
    for (name, addr) in [
        ("CLOCK", CLOCK),
        ("UART0", UART0),
        ("TWIM1", I2C1),
        ("RTC1", RTC1),
    ] {
        assert!(
            bus.read_u32(addr).is_ok(),
            "{name} must be mapped at secure cpuapp address 0x{addr:08X}"
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

#[test]
fn twim1_reads_bmi270_chip_id_over_easydma() {
    let mut bus = nrf5340_bus_with(vec![ExternalDevice {
        id: "imu".to_string(),
        r#type: "bmi270".to_string(),
        connection: "i2c1".to_string(),
        channel: None,
        route: [
            ("sda".to_string(), "P1.02".to_string()),
            ("scl".to_string(), "P1.03".to_string()),
        ]
        .into_iter()
        .collect(),
        config: Default::default(),
    }]);

    // BMI270 CHIP_ID pointer write followed by a repeated-START one-byte read.
    bus.write_u8(RAM, 0x00).unwrap();
    bus.write_u8(RAM + 0x100, 0xEE).unwrap();
    bus.write_u32(I2C1 + TWIM_ENABLE, 6).unwrap();
    // nRF PSEL encodes PORT in bit 5 and PIN in bits 4:0.
    bus.write_u32(I2C1 + TWIM_PSEL_SDA, (1 << 5) | 2).unwrap();
    bus.write_u32(I2C1 + TWIM_PSEL_SCL, (1 << 5) | 3).unwrap();
    bus.write_u32(I2C1 + TWIM_ADDRESS, 0x68).unwrap();
    bus.write_u32(I2C1 + TWIM_TXD_PTR, RAM as u32).unwrap();
    bus.write_u32(I2C1 + TWIM_TXD_MAXCNT, 1).unwrap();
    bus.write_u32(I2C1 + TWIM_RXD_PTR, (RAM + 0x100) as u32)
        .unwrap();
    bus.write_u32(I2C1 + TWIM_RXD_MAXCNT, 1).unwrap();
    bus.write_u32(
        I2C1 + TWIM_SHORTS,
        TWIM_SHORT_LASTTX_STARTRX | TWIM_SHORT_LASTRX_STOP,
    )
    .unwrap();
    bus.write_u32(I2C1 + TWIM_TASKS_STARTTX, 1).unwrap();

    let mut completed = false;
    for _ in 0..500_000 {
        bus.tick_peripherals_fully_forced();
        if bus.read_u32(I2C1 + TWIM_EVENTS_LASTRX).unwrap() == 1 {
            completed = true;
            break;
        }
    }
    assert!(
        completed,
        "TWIM1 did not complete the BMI270 repeated-START read"
    );
    assert_eq!(
        bus.read_u32(I2C1 + TWIM_ERRORSRC).unwrap(),
        0,
        "BMI270 must ACK on the nRF5340 DK i2c1 bus"
    );
    assert_eq!(
        bus.read_u8(RAM + 0x100).unwrap(),
        0x24,
        "BMI270 CHIP_ID must come from the attached model, not the 0xEE sentinel"
    );
}
