// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! nRF52 PWM duty reaches the pad on a whole machine (micro:bit V2, nRF52833).
//!
//! Before the pad waveform, `analogWrite` on an nRF chip programmed PWM0's
//! registers and played its sequence, and the pad it named read the GPIO
//! port's own OUT the whole time: a dimmed LED on ring 0 was dark and a servo
//! saw no pulses. These tests drive the bus the way CODAL's
//! `NRF52Pin::setAnalogValue` does (one Individual step, MODE Up, LOOP 0,
//! SHORTS 0, no POLARITY bit) and measure the pad through `read_gpio_pad`,
//! the same read the logic analyzer and bw-board's pad view use.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::{
    memory::ProgramImage, system::cortex_m::configure_cortex_m, Arch, Bus, Machine,
};
use std::path::PathBuf;

const P0: u64 = 0x5000_0000;
const OUTCLR: u64 = 0x50C;
const DIRSET: u64 = 0x518;
const PWM0: u64 = 0x4001_C000;
const TASKS_STOP: u64 = 0x004;
const TASKS_SEQSTART0: u64 = 0x008;
const ENABLE: u64 = 0x500;
const MODE: u64 = 0x504;
const COUNTERTOP: u64 = 0x508;
const PRESCALER: u64 = 0x50C;
const DECODER: u64 = 0x510;
const SEQ0_PTR: u64 = 0x520;
const SEQ0_CNT: u64 = 0x524;
const PSEL_OUT0: u64 = 0x560;
/// micro:bit ring 0.
const RING0: u32 = 2;
const SEQ_RAM: u64 = 0x2000_1000;

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

fn ring0(machine: &Machine<CortexM>) -> bool {
    let idx = machine.bus.find_peripheral_index_by_name("gpio0").unwrap();
    machine.bus.peripherals[idx]
        .dev
        .read_gpio_pad(RING0 as u8)
        .expect("P0.02 is a real pad")
}

/// CODAL's pin setup (`connectPin`: a GPIO output driven low) and its PWM0
/// configuration at `top` ticks of 16 MHz, then one step: channel 0 = `comp`.
fn analog_write(m: &mut Machine<CortexM>, top: u32, comp: u16) {
    m.bus.write_u32(P0 + DIRSET, 1 << RING0).unwrap();
    m.bus.write_u32(P0 + OUTCLR, 1 << RING0).unwrap();
    m.bus.write_u32(PWM0 + PSEL_OUT0, RING0).unwrap();
    m.bus.write_u32(PWM0 + MODE, 0).unwrap();
    m.bus.write_u32(PWM0 + PRESCALER, 0).unwrap();
    m.bus.write_u32(PWM0 + COUNTERTOP, top).unwrap();
    m.bus.write_u32(PWM0 + DECODER, 2).unwrap(); // LOAD = Individual
    for (i, v) in [comp, 0, 0, 0].iter().enumerate() {
        let at = SEQ_RAM + 2 * i as u64;
        m.bus.write_u8(at, (*v & 0xFF) as u8).unwrap();
        m.bus.write_u8(at + 1, (*v >> 8) as u8).unwrap();
    }
    m.bus.write_u32(PWM0 + SEQ0_PTR, SEQ_RAM as u32).unwrap();
    m.bus.write_u32(PWM0 + SEQ0_CNT, 4).unwrap();
    m.bus.write_u32(PWM0 + ENABLE, 1).unwrap();
    m.bus.write_u32(PWM0 + TASKS_SEQSTART0, 1).unwrap();
}

/// Fraction of machine cycles the pad spends high over `cycles`, weighting
/// each step's reading by the cycles it took, plus the number of rising edges
/// seen.
fn measure(m: &mut Machine<CortexM>, cycles: u64) -> (f64, u32) {
    let start = m.total_cycles;
    let (mut high, mut rises, mut last) = (0u64, 0u32, ring0(m));
    while m.total_cycles - start < cycles {
        let before = m.total_cycles;
        m.step().expect("step");
        let level = ring0(m);
        if level {
            high += m.total_cycles - before;
        }
        if level && !last {
            rises += 1;
        }
        last = level;
    }
    (high as f64 / (m.total_cycles - start) as f64, rises)
}

#[test]
fn analog_write_duty_reaches_the_pad() {
    // COUNTERTOP 1000 at 16 MHz on a 64 MHz core = 4000-cycle period.
    for (comp, duty) in [(750u16, 0.25), (500, 0.50), (100, 0.90)] {
        let mut m = microbit_v2();
        analog_write(&mut m, 1000, comp);
        m.step().unwrap(); // the EasyDMA drain
        let (high, rises) = measure(&mut m, 20 * 4000);
        assert!(
            (high - duty).abs() < 0.01,
            "COMP {comp}: pad high {:.4} of the time, want {duty}",
            high
        );
        assert!(
            (19..=21).contains(&rises),
            "COMP {comp}: {rises} periods in 20"
        );
    }
}

#[test]
fn stop_hands_the_pad_back_to_the_port() {
    let mut m = microbit_v2();
    analog_write(&mut m, 1000, 0); // COMP 0 with rising polarity: always high
    m.step().unwrap();
    let (high, _) = measure(&mut m, 8000);
    assert!(
        high > 0.99,
        "a playing 100 % channel holds the pad high: {high}"
    );
    m.bus.write_u32(PWM0 + TASKS_STOP, 1).unwrap();
    m.step().unwrap();
    assert!(
        !ring0(&m),
        "stopped: the pad reads the port's OUT (low) again"
    );
}
