// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The micro:bit V2 5×5 display, the way CODAL drives it: rows from the GPIO
//! output register, columns from GPIOTE Task-mode channels that a TIMER
//! toggles over PPI (codal-microbit-v2 NRF52LedMatrix.cpp).
//!
//! Each test drives the bus directly (the stock nrf52833 config tests do the
//! same) with a `b .` firmware, so no Nordic-derived image is needed. What a
//! real MakeCode V2 program showed before these fixes, and what each test
//! pins:
//!
//! * the column pads read the port's OUT (low) instead of the GPIOTE level,
//!   because CODAL leaves each column configured as a GPIO output after its
//!   light-sense strobe — `gpiote_owns_its_pad_over_the_ports_registers`;
//! * GPIOTE's pad drive reset every other latched input on the port, so both
//!   buttons read "pressed" and the runtime took its buttons-held boot path —
//!   `a_gpiote_drive_leaves_the_buttons_released`;
//! * the matrix model shows what the pads carry —
//!   `the_matrix_shows_the_pixel_gpiote_and_the_row_light`.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::{
    inspect::InspectOpts, memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus,
    Machine,
};
use std::path::PathBuf;

const P0: u64 = 0x5000_0000;
const OUTSET: u64 = 0x508;
const OUTCLR: u64 = 0x50C;
const DIRSET: u64 = 0x518;
const GPIOTE: u64 = 0x4000_6000;
const GPIOTE_TASKS_OUT: u64 = 0x000;
const GPIOTE_CONFIG: u64 = 0x510;
const PPI: u64 = 0x4001_F000;
const TIMER4: u64 = 0x4001_B000;
const TIMER_TASKS_START: u64 = 0x000;

/// CONFIG[n]: MODE = Task, PSEL, PORT 0, POLARITY = Toggle, OUTINIT.
fn task_config(pin: u32, outinit: bool) -> u32 {
    3 | (pin << 8) | (3 << 16) | (u32::from(outinit) << 20)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn microbit_v2() -> Machine<CortexM> {
    let sys = workspace_root().join("configs/systems/microbit-v2.yaml");
    let mut manifest = SystemManifest::from_file(&sys).expect("load microbit-v2");
    let chip_path = sys.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load nrf52833");
    manifest.chip = chip_path.to_str().expect("utf-8 chip path").to_string();
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("microbit-v2 bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    let mut image = ProgramImage::new(0x101, Arch::Arm);
    let mut flash = vec![0u8; 0x104];
    flash[0..4].copy_from_slice(&0x2000_4000u32.to_le_bytes());
    flash[4..8].copy_from_slice(&0x0000_0101u32.to_le_bytes());
    flash[0x100..0x102].copy_from_slice(&0xE7FEu16.to_le_bytes()); // b .
    image.add_segment(0, flash);
    machine.load_firmware(&image).expect("load firmware");
    machine
}

fn run(machine: &mut Machine<CortexM>, steps: u64) {
    for _ in 0..steps {
        machine.step().expect("step");
    }
}

fn pad(machine: &Machine<CortexM>, port: &str, pin: u8) -> Option<bool> {
    let idx = machine.bus.find_peripheral_index_by_name(port)?;
    machine.bus.peripherals[idx].dev.read_gpio_pad(pin)
}

#[test]
fn gpiote_owns_its_pad_over_the_ports_registers() {
    let mut m = microbit_v2();
    // COL1 = P0.28 as CODAL leaves it after a light-sense strobe: a GPIO
    // output driving LOW.
    m.bus.write_u32(P0 + DIRSET, 1 << 28).unwrap();
    m.bus.write_u32(P0 + OUTCLR, 1 << 28).unwrap();
    run(&mut m, 8);
    assert_eq!(pad(&m, "gpio0", 28), Some(false), "port drives it low");

    // GPIOTE channel 1 takes the pin in Task mode at OUTINIT = 1.
    m.bus
        .write_u32(GPIOTE + GPIOTE_CONFIG + 4, task_config(28, true))
        .unwrap();
    run(&mut m, 8);
    assert_eq!(
        pad(&m, "gpio0", 28),
        Some(true),
        "a Task-mode channel drives the pin to OUTINIT whatever DIR/OUT say"
    );
    let idx = m.bus.find_peripheral_index_by_name("gpio0").unwrap();
    let routing = m.bus.peripherals[idx]
        .dev
        .gpio_routing(28)
        .expect("routing");
    assert_eq!(routing.func.as_deref(), Some("GPIOTE_OUT1"));

    // TASKS_OUT (Toggle) moves the pad; the port's OUT never does.
    m.bus.write_u32(GPIOTE + GPIOTE_TASKS_OUT + 4, 1).unwrap();
    run(&mut m, 8);
    assert_eq!(pad(&m, "gpio0", 28), Some(false), "toggled low");

    // Mode away from Task: the port's own registers own the pin again.
    m.bus.write_u32(P0 + OUTSET, 1 << 28).unwrap();
    m.bus.write_u32(GPIOTE + GPIOTE_CONFIG + 4, 0).unwrap();
    run(&mut m, 8);
    assert_eq!(pad(&m, "gpio0", 28), Some(true), "released back to OUT = 1");
}

#[test]
fn a_gpiote_drive_leaves_the_buttons_released() {
    let mut m = microbit_v2();
    run(&mut m, 8);
    assert_eq!(
        pad(&m, "gpio0", 14),
        Some(true),
        "button A released at boot"
    );
    assert_eq!(
        pad(&m, "gpio0", 23),
        Some(true),
        "button B released at boot"
    );
    // CODAL's column drive: every GPIOTE drive used to latch a whole IN word
    // from a zeroed shadow, pulling both buttons low ("pressed").
    for (ch, pin) in [(1u64, 28u32), (2, 11), (3, 31), (5, 30)] {
        m.bus
            .write_u32(GPIOTE + GPIOTE_CONFIG + 4 * ch, task_config(pin, false))
            .unwrap();
    }
    run(&mut m, 16);
    assert_eq!(pad(&m, "gpio0", 14), Some(true), "button A still released");
    assert_eq!(pad(&m, "gpio0", 23), Some(true), "button B still released");
}

#[test]
fn a_timer_compare_over_ppi_toggles_the_pad() {
    let mut m = microbit_v2();
    m.bus
        .write_u32(GPIOTE + GPIOTE_CONFIG + 4, task_config(28, false))
        .unwrap();
    run(&mut m, 8);
    assert_eq!(pad(&m, "gpio0", 28), Some(false));
    // PPI CH0: TIMER4 EVENTS_COMPARE[1] -> GPIOTE TASKS_OUT[1].
    m.bus
        .write_u32(PPI + 0x510, (TIMER4 + 0x144) as u32)
        .unwrap(); // CH[0].EEP
    m.bus
        .write_u32(PPI + 0x514, (GPIOTE + GPIOTE_TASKS_OUT + 4) as u32)
        .unwrap(); // CH[0].TEP
    m.bus.write_u32(PPI + 0x504, 1).unwrap(); // CHENSET
    m.bus.write_u32(TIMER4 + 0x508, 3).unwrap(); // BITMODE 32
    m.bus.write_u32(TIMER4 + 0x510, 0).unwrap(); // PRESCALER 0
    m.bus.write_u32(TIMER4 + 0x544, 200).unwrap(); // CC[1]
    m.bus.write_u32(TIMER4 + TIMER_TASKS_START, 1).unwrap();
    run(&mut m, 5_000);
    assert_eq!(
        pad(&m, "gpio0", 28),
        Some(true),
        "the compare toggled the column through GPIOTE, with no CPU store"
    );
}

#[test]
fn the_matrix_shows_the_pixel_gpiote_and_the_row_light() {
    let mut m = microbit_v2();
    // All five columns owned by GPIOTE: COL1 lit (low), the rest dark (high).
    let cols = [
        (1u64, 28u32, false),
        (2, 11, true),
        (3, 31, true),
        (5, 30, true),
    ];
    for (ch, pin, dark) in cols {
        m.bus
            .write_u32(GPIOTE + GPIOTE_CONFIG + 4 * ch, task_config(pin, dark))
            .unwrap();
    }
    // COL4 is P1.05: PORT = 1.
    m.bus
        .write_u32(
            GPIOTE + GPIOTE_CONFIG + 4 * 4,
            task_config(5, true) | 1 << 13,
        )
        .unwrap();
    // ROW1 = P0.21 driven high from the port.
    m.bus.write_u32(P0 + DIRSET, 1 << 21).unwrap();
    m.bus.write_u32(P0 + OUTSET, 1 << 21).unwrap();
    // 60 ms at 64 MHz: well past one 20 ms persistence window.
    run(&mut m, 4_000_000);
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    let frame = m
        .bus
        .display_artifact("led_matrix", &opts)
        .expect("the micro:bit V2 declares its LED matrix");
    assert_eq!(frame.meta["format"], "gray8");
    // Only ROW1 is ever selected, so the lit pixel is at full row-slot duty
    // divided by five rows: the one LED a static row can show.
    let bytes = frame.bytes.expect("bytes");
    assert!(bytes[0] > 0, "pixel (0,0) is lit: {bytes:?}");
    assert!(
        bytes[1..].iter().all(|&b| b == 0),
        "no other pixel is lit: {bytes:?}"
    );
}
