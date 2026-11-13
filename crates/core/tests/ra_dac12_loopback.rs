// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA6M5 / RA8M1 DAC0 channel 0 (DA0, P014) jumpered to ADC0 channel 0
//! (AN000): a conversion of AN000 reads the level DA0 drives at that moment.
//! Built from the chip YAMLs and examples/ek-ra{6m5,8m1}/dac-loopback-system.yaml,
//! so the `ra_dac12` / `ra_adc12` wiring and the `analog_links` jumper are
//! under test, and from the plain board system.yaml for the unwired case.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const DADR0: u64 = 0x00;
const DACR: u64 = 0x04;
const DAOE0: u8 = 1 << 6;
const ADCSR: u64 = 0x00;
const ADANSA0: u64 = 0x04;
const ADDR0: u64 = 0x20;
const ADST: u16 = 1 << 15;

struct Board {
    chip: &'static str,
    system: &'static str,
    dac: u64,
    adc0: u64,
    /// A branch-to-self in on-chip SRAM to park the core on.
    idle: u32,
}

const RA6M5: Board = Board {
    chip: "configs/chips/r7fa6m5bh.yaml",
    system: "examples/ek-ra6m5/dac-loopback-system.yaml",
    dac: 0x4017_1000,
    adc0: 0x4017_0000,
    idle: 0x2000_2000,
};
const RA8M1: Board = Board {
    chip: "configs/chips/r7fa8m1ah.yaml",
    system: "examples/ek-ra8m1/dac-loopback-system.yaml",
    dac: 0x4033_3000,
    adc0: 0x4033_2000,
    idle: 0x2200_2000,
};

fn machine(b: &Board, system: &str) -> Machine<CortexM> {
    let chip_path = root(b.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system)).expect("system");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(b.idle), 0xE7FE).unwrap();
    m.cpu.set_pc(b.idle);
    m
}

/// One software-started single scan of AN000; the code it converts.
fn convert_an000(m: &mut Machine<CortexM>, b: &Board) -> u16 {
    m.bus.write_u16(b.adc0 + ADANSA0, 1).unwrap();
    m.bus.write_u16(b.adc0 + ADCSR, ADST).unwrap();
    for _ in 0..100_000 {
        if m.bus.read_u16(b.adc0 + ADCSR).unwrap() & ADST == 0 {
            return m.bus.read_u16(b.adc0 + ADDR0).unwrap();
        }
        m.step().unwrap();
    }
    panic!("ADC0 scan never completed");
}

fn jumper_carries_da0_to_an000(b: &Board) {
    let mut m = machine(b, b.system);
    m.bus.write_u16(b.dac + DADR0, 0x0800).unwrap();
    assert_eq!(
        convert_an000(&mut m, b),
        0,
        "DACR.DAOE0 = 0: DA0 is not driven, AN000 reads its held 0 V"
    );
    m.bus.write_u8(b.dac + DACR, DAOE0).unwrap();
    // 2048 * 3300 / 4096 = 1650 mV; 1650 * 4095 / 3300 = 0x07FF.
    assert_eq!(convert_an000(&mut m, b), 0x07FF);
    // A later DADR0 write is seen by the next conversion: full scale is
    // 4095 * 3300 / 4096 = 3299 mV, 3299 * 4095 / 3300 = 4093.
    m.bus.write_u16(b.dac + DADR0, 0x0FFF).unwrap();
    assert_eq!(convert_an000(&mut m, b), 4093);
    m.bus.write_u8(b.dac + DACR, 0).unwrap();
    assert_eq!(convert_an000(&mut m, b), 0);
}

fn no_jumper_no_level(b: &Board, plain: &str) {
    let mut m = machine(b, plain);
    m.bus.write_u16(b.dac + DADR0, 0x0800).unwrap();
    m.bus.write_u8(b.dac + DACR, DAOE0).unwrap();
    assert_eq!(m.bus.read_u16(b.dac + DADR0).unwrap(), 0x0800);
    assert_eq!(m.bus.read_u8(b.dac + DACR).unwrap(), 0x5F);
    assert_eq!(convert_an000(&mut m, b), 0, "AN000 is not wired to DA0");
}

#[test]
fn ra6m5_jumper_carries_da0_to_an000() {
    jumper_carries_da0_to_an000(&RA6M5);
}

#[test]
fn ra8m1_jumper_carries_da0_to_an000() {
    jumper_carries_da0_to_an000(&RA8M1);
}

#[test]
fn ra6m5_without_the_jumper_an000_reads_zero() {
    no_jumper_no_level(&RA6M5, "examples/ek-ra6m5/system.yaml");
}

#[test]
fn ra8m1_without_the_jumper_an000_reads_zero() {
    no_jumper_no_level(&RA8M1, "examples/ek-ra8m1/system.yaml");
}
