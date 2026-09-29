// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! End-to-end coverage of the 74HC4051 analog mux: a Cortex-M program that
//! scans eight potentiometers through one mux into one ADC channel, the way a
//! real knob scanner does it (set S0..S2, start one conversion AT ONCE, read
//! the result). Fixture: `tests/fixtures/analog_mux/` (chip, board, and the
//! scan program's source `scan.S`).
//!
//! The unit tests next to the model cover the function table. This file
//! covers the wiring: that the CPU's GPIO stores move the mux before the
//! next conversion, that the ADC converts the routed input and no other, that
//! a stimulus on one pot moves only its own table entry, and that E opens the
//! switches.

mod common;
use common::root;
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

/// `scan.S`, assembled (arm-none-eabi-as -mcpu=cortex-m7 -mthumb, then
/// objcopy -O binary). Loaded at `PROGRAM`.
const SCAN: [u8; 60] = [
    0x0b, 0x48, 0x0c, 0x49, 0x0c, 0x4a, 0x40, 0xf2, 0x08, 0x24, 0x4c, 0x64, 0x70, 0x24, 0x44, 0x60,
    0x00, 0x23, 0x1c, 0x01, 0x04, 0x60, 0x03, 0x24, 0x0c, 0x60, 0x0c, 0x6a, 0xe4, 0x07, 0xfc, 0xd0,
    0x4c, 0x6a, 0x22, 0xf8, 0x13, 0x40, 0x01, 0x33, 0x08, 0x2b, 0xf2, 0xd1, 0xfe, 0xe7, 0x00, 0x00,
    0x00, 0x80, 0x1b, 0x40, 0x00, 0x40, 0x0c, 0x40, 0x00, 0x10, 0x00, 0x20,
];
const PROGRAM: u32 = 0x2000_0000;
/// The `b .` at the end of the scan.
const DONE: u32 = PROGRAM + 0x2c;
/// u16[8] result table the scan writes.
const TABLE: u64 = 0x2000_1000;
const GPIO1: u64 = 0x401B_8000;
const GPIO_DR: u64 = 0x00;
const GPIO_GDIR: u64 = 0x04;
/// PAD_E is GPIO1 bit 9 in the fixture chip.
const E_BIT: u32 = 1 << 9;

/// Pot positions in the fixture system.yaml, Y0..Y7 (%).
const POSITIONS: [f64; 8] = [5.0, 15.0, 25.0, 35.0, 45.0, 55.0, 65.0, 75.0];

fn load(system: &str) -> anyhow::Result<SystemBus> {
    let dir = root("crates/core/tests/fixtures/analog_mux");
    let chip = ChipDescriptor::from_file(dir.join("chip.yaml")).expect("chip.yaml");
    let manifest: SystemManifest = serde_yaml::from_str(system).expect("system yaml");
    SystemBus::from_config(&chip, &manifest)
}

fn fixture_system() -> String {
    let path = root("crates/core/tests/fixtures/analog_mux/system.yaml");
    std::fs::read_to_string(path).expect("system.yaml")
}

fn machine() -> Machine<impl Cpu> {
    let mut bus = load(&fixture_system()).expect("build bus");
    for (i, b) in SCAN.iter().enumerate() {
        bus.write_u8(PROGRAM as u64 + i as u64, *b).unwrap();
    }
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

/// Run the scan from the start until it parks on its final `b .`.
fn scan(m: &mut Machine<impl Cpu>) -> [u16; 8] {
    m.cpu.set_pc(PROGRAM);
    for _ in 0..20_000 {
        if m.cpu.get_pc() == DONE {
            let mut t = [0u16; 8];
            for (i, v) in t.iter_mut().enumerate() {
                *v = m.bus.read_u16(TABLE + 2 * i as u64).unwrap();
            }
            return t;
        }
        m.step().expect("step");
    }
    panic!("scan did not finish; pc = {:#x}", m.cpu.get_pc());
}

/// 12-bit count of a potentiometer at `pct` % (V = 3300 mV * pct / 100,
/// truncated to mV by the pot model, then the ADC's rounding).
fn counts(pct: f64) -> u16 {
    let mv = (3300.0 * pct / 100.0) as u32;
    ((mv * 4095 + 1650) / 3300) as u16
}

#[test]
fn scan_reads_every_pot_through_the_mux() {
    let mut m = machine();
    let table = scan(&mut m);
    let expected: Vec<u16> = POSITIONS.iter().map(|&p| counts(p)).collect();
    assert_eq!(table.to_vec(), expected, "entry n must be pot n");
    let mux = m.bus.analog_mux("mux").expect("mux attached");
    assert_eq!(mux.selected(), Some(7), "the scan ends on Y7");
    assert_eq!(mux.switch_count(), 7, "Y0 -> Y7, one switch per step");
}

#[test]
fn stimulus_moves_only_its_own_knob() {
    let mut m = machine();
    let before = scan(&mut m);
    m.bus
        .set_input(Some("pot3"), "position", 90.0)
        .expect("drive pot3");
    let after = scan(&mut m);
    for n in 0..8 {
        if n == 3 {
            assert_eq!(after[n], counts(90.0), "pot3 turned to 90 %");
        } else {
            assert_eq!(after[n], before[n], "entry {n} must not move");
        }
    }
}

/// A stimulus on the input that is routed NOW reaches the ADC at once,
/// without a select change.
#[test]
fn stimulus_on_the_routed_input_reaches_the_adc_at_once() {
    let mut m = machine();
    scan(&mut m); // leaves Y7 routed
    m.bus
        .set_input(Some("pot7"), "position", 10.0)
        .expect("drive pot7");
    // Convert channel 3 directly, with no GPIO store in between.
    m.bus.write_u32(0x400C_4000, 3).unwrap();
    for _ in 0..1000 {
        m.step().unwrap();
        if m.bus.read_u32(0x400C_4020).unwrap() & 1 != 0 {
            break;
        }
    }
    assert_eq!(m.bus.read_u32(0x400C_4024).unwrap() as u16, counts(10.0));
}

/// Negative control: E HIGH opens every switch. The same scan now reads the
/// open Z (0 mV) for all eight entries.
#[test]
fn enable_high_opens_every_switch() {
    let mut m = machine();
    // Park the CPU on the scan's final `b .`: only time must pass.
    m.cpu.set_pc(DONE);
    m.bus.write_u32(GPIO1 + GPIO_GDIR, E_BIT).unwrap();
    m.bus.write_u32(GPIO1 + GPIO_DR, E_BIT).unwrap();
    assert_eq!(m.bus.analog_mux("mux").unwrap().selected(), None);
    // Start a conversion with E high: 0 mV.
    m.bus.write_u32(0x400C_4044, 0x208).unwrap();
    m.bus.write_u32(0x400C_4000, 3).unwrap();
    for _ in 0..1000 {
        m.step().unwrap();
        if m.bus.read_u32(0x400C_4020).unwrap() & 1 != 0 {
            break;
        }
    }
    assert_eq!(m.bus.read_u32(0x400C_4024).unwrap(), 0);
    // E low again: the routed channel comes back without a select change.
    m.bus.write_u32(GPIO1 + GPIO_DR, 0).unwrap();
    assert_eq!(m.bus.analog_mux("mux").unwrap().selected(), Some(0));
}

/// Negative control: the mux follows the PAD, not the output latch. With
/// the select pads left as inputs, DR stores do not move them, and every
/// entry reads Y0.
#[test]
fn select_pads_configured_as_inputs_do_not_move_the_mux() {
    let mut m = machine();
    // Patch the scan's GDIR value (movs r4, #0x70 -> movs r4, #0x00).
    let at = SCAN
        .windows(2)
        .position(|w| w == [0x70, 0x24])
        .expect("movs r4, #0x70 in scan");
    m.bus.write_u8(PROGRAM as u64 + at as u64, 0x00).unwrap();
    let table = scan(&mut m);
    assert!(
        table.iter().all(|&v| v == counts(POSITIONS[0])),
        "every entry must read Y0, got {table:?}"
    );
}

/// A pot that names a mux declared AFTER it must fail the build, not put its
/// level on channel N of some ADC found by a bus scan.
#[test]
fn source_declared_before_its_mux_is_an_error() {
    let yaml = fixture_system();
    let mut lines: Vec<&str> = yaml.lines().collect();
    let pot0 = lines
        .iter()
        .position(|l| l.contains("id: \"pot0\""))
        .unwrap();
    let pot_line = lines.remove(pot0);
    let devices = lines
        .iter()
        .position(|l| l.starts_with("external_devices:"))
        .unwrap();
    lines.insert(devices + 1, pot_line);
    let err = load(&lines.join("\n")).err().expect("must fail");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("pot0") && msg.contains("declare it before"),
        "error must name the part and the fix: {msg}"
    );
}

#[test]
fn mux_input_out_of_range_is_an_error() {
    let yaml = fixture_system().replace("channel: 7, position: 75", "channel: 8, position: 75");
    let err = load(&yaml).err().expect("the 74HC4051 has no Y8");
    assert!(
        format!("{err:#}").contains("not an input of analog mux"),
        "{err:#}"
    );
}

#[test]
fn select_pad_not_on_the_chip_is_an_error() {
    let yaml = fixture_system().replace("s2_pin: \"PAD_S2\"", "s2_pin: \"PAD_NONE\"");
    let err = load(&yaml).err().expect("must fail");
    assert!(format!("{err:#}").contains("PAD_NONE"), "{err:#}");
}
