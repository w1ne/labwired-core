// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA8M1 ADC1 scan end reaches the core through the ICU: a software-started
//! single scan (ADCSR = GBADIE | TRGE | ADST, what FSP `R_ADC_ScanStart`
//! writes) loads ADDRn, clears ADST and raises ELC_EVENT_ADC1_SCAN_END
//! (0x1B4, FSP ra8m1 `bsp_elc.h`), which only `IELSRn` links to an NVIC line.
//! Built from the chip YAML and examples/ek-ra8m1/adc-system.yaml, so the
//! `ra_adc12` wiring and the potentiometer / TMP36 sources are under test.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const ADC1: u64 = 0x4033_2200;
const ADCSR: u64 = ADC1;
const ADANSA0: u64 = ADC1 + 0x04;
const ADDR0: u64 = ADC1 + 0x20;
const ADDR2: u64 = ADC1 + 0x24;
const ADST: u16 = 1 << 15;
const TRGE: u16 = 1 << 9;
const GBADIE: u16 = 1 << 6;
const IELSR0: u64 = 0x4000_C300;
const IR: u32 = 1 << 16;
const ADC1_SCAN_END: u32 = 0x1B4;
/// ra8x1.dtsi `adc1` `scanend` line.
const LINE: u32 = 39;
const TABLE: u32 = 0x2200_0400;
const IDLE: u32 = 0x2200_2000;
const HANDLER: u32 = 0x2200_2100;
const NVIC_ISER: u64 = 0xE000_E100;
const SCB_VTOR: u64 = 0xE000_ED08;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/r7fa8m1ah.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("r7fa8m1ah chip");
    let mut manifest = SystemManifest::from_file(root("examples/ek-ra8m1/adc-system.yaml"))
        .expect("ek-ra8m1 adc system");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.bus.write_u16(u64::from(HANDLER), 0xE7FE).unwrap();
    m.bus
        .write_u32(u64::from(TABLE + 4 * (16 + LINE)), HANDLER | 1)
        .unwrap();
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2200_7F00);
    m
}

fn start_scan(m: &mut Machine<CortexM>) {
    m.bus.write_u16(ADANSA0, 0b101).unwrap();
    m.bus.write_u16(ADCSR, GBADIE | TRGE | ADST).unwrap();
}

fn run_until_handler(m: &mut Machine<CortexM>, steps: u32) -> bool {
    for _ in 0..steps {
        if m.cpu.get_pc() == HANDLER {
            return true;
        }
        m.step().unwrap();
    }
    false
}

/// Two channels at the nominal 1 µs each at 480 MHz are 960 cycles.
const BUDGET: u32 = 20_000;

#[test]
fn scan_end_vectors_the_core_through_ielsr_with_the_converted_codes() {
    let mut m = machine();
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(LINE), ADC1_SCAN_END)
        .unwrap();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(LINE / 32), 1 << (LINE % 32))
        .unwrap();
    start_scan(&mut m);
    assert_eq!(m.bus.read_u16(ADCSR).unwrap() & ADST, ADST, "scan running");
    assert!(
        run_until_handler(&mut m, BUDGET),
        "ADC1 scan end never vectored the core through IELSR{LINE}"
    );
    assert_eq!(m.bus.read_u16(ADCSR).unwrap() & ADST, 0, "single scan done");
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(LINE)).unwrap(),
        IR | ADC1_SCAN_END
    );
    // Potentiometer at mid travel (1650 mV) and TMP36 at 25 C (750 mV), on a
    // 12-bit, 3300 mV scale.
    assert_eq!(m.bus.read_u16(ADDR0).unwrap(), 0x07FF);
    assert_eq!(m.bus.read_u16(ADDR2).unwrap(), 0x03A2);
}

#[test]
fn an_unlinked_scan_end_reaches_no_line() {
    let mut m = machine();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(LINE / 32), 1 << (LINE % 32))
        .unwrap();
    start_scan(&mut m);
    assert!(!run_until_handler(&mut m, BUDGET));
    // The scan completed; only the link is missing.
    assert_eq!(m.bus.read_u16(ADCSR).unwrap() & ADST, 0);
    assert_eq!(m.bus.read_u16(ADDR0).unwrap(), 0x07FF);
    assert!(!m.bus.is_nvic_irq_pending(16 + LINE));
}
