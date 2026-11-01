// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA6M5 AGT0 compare match A reaches the core through the ICU: the event
//! (ELC_EVENT_AGT0_COMPARE_A, 0x041, FSP ra6m5 `bsp_elc.h`) is linked to an
//! NVIC line only by `IELSRn` (R_ICU_BASE + 0x300 + 4n), sets `IELSRn.IR`,
//! and the line follows `IR` as a level: the pend drops when firmware clears
//! `IR` (FSP `R_BSP_IrqStatusClear`). Built from the EK-RA6M5 descriptor, so
//! the chip YAML's `ra_icu` / `ra_agt` wiring is what is under test.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const AGT0: u64 = 0x400E_8000;
const AGTCMA: u64 = AGT0 + 0x02;
const AGTCR: u64 = AGT0 + 0x08;
const AGTMR1: u64 = AGT0 + 0x09;
const AGTCMSR: u64 = AGT0 + 0x0E;
const TCMAF: u8 = 1 << 6;
const IELSR0: u64 = 0x4000_6300;
const IR: u32 = 1 << 16;
const AGT0_COMPARE_A: u32 = 0x041;
/// Zephyr's in-tree ek_ra6m5.overlay line for agt0 `agtcmai`.
const LINE: u32 = 84;
/// Vector table in SRAM (VTOR: 512-byte alignment covers 16 + 96 vectors).
const TABLE: u32 = 0x2000_0400;
const IDLE: u32 = 0x2000_2000;
const HANDLER: u32 = 0x2000_2100;
const NVIC_ISER: u64 = 0xE000_E100;
const SCB_VTOR: u64 = 0xE000_ED08;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/r7fa6m5bh.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("r7fa6m5bh chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/ek-ra6m5.yaml")).expect("ek-ra6m5");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    // Idle loop and handler are both `b .`.
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.bus.write_u16(u64::from(HANDLER), 0xE7FE).unwrap();
    m.bus
        .write_u32(u64::from(TABLE + 4 * (16 + LINE)), HANDLER | 1)
        .unwrap();
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2000_7F00);
    m
}

/// AGT0 on LOCO /1 from the reset count 0xFFFF, compare A `ticks` ahead.
fn arm_compare_a(m: &mut Machine<CortexM>, ticks: u16) {
    m.bus.write_u8(AGTMR1, 4 << 4).unwrap();
    m.bus.write_u16(AGTCMA, 0xFFFF - ticks).unwrap();
    m.bus.write_u8(AGTCMSR, 1).unwrap();
    m.bus.write_u8(AGTCR, 1).unwrap();
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

/// 10 LOCO ticks at the 200 MHz `clock_hz` are ~61 000 cycles.
const BUDGET: u32 = 400_000;

#[test]
fn compare_match_a_vectors_the_core_through_ielsr() {
    let mut m = machine();
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(LINE), AGT0_COMPARE_A)
        .unwrap();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(LINE / 32), 1 << (LINE % 32))
        .unwrap();
    arm_compare_a(&mut m, 10);
    assert!(
        run_until_handler(&mut m, BUDGET),
        "AGT0 compare A never vectored the core through IELSR{LINE}"
    );
    assert_eq!(m.bus.read_u8(AGTCR).unwrap() & TCMAF, TCMAF);
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(LINE)).unwrap(),
        IR | AGT0_COMPARE_A
    );
    // Not before the compare value is reached.
    let count = m.bus.read_u16(AGT0).unwrap();
    assert!(
        (0xFFFF - 11..=0xFFFF - 10).contains(&count),
        "fired at count {count:#x}"
    );
}

#[test]
fn an_unlinked_event_reaches_no_line() {
    let mut m = machine();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(LINE / 32), 1 << (LINE % 32))
        .unwrap();
    arm_compare_a(&mut m, 10);
    assert!(!run_until_handler(&mut m, BUDGET));
    // The AGT matched; only the link is missing.
    assert_eq!(m.bus.read_u8(AGTCR).unwrap() & TCMAF, TCMAF);
    assert!(!m.bus.is_nvic_irq_pending(16 + LINE));
}

#[test]
fn clearing_ir_drops_the_pend() {
    let mut m = machine();
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(LINE), AGT0_COMPARE_A)
        .unwrap();
    // Line not enabled: the pend stays visible instead of being taken.
    arm_compare_a(&mut m, 10);
    for _ in 0..BUDGET {
        if m.bus.is_nvic_irq_pending(16 + LINE) {
            break;
        }
        m.step().unwrap();
    }
    assert!(m.bus.is_nvic_irq_pending(16 + LINE), "event never pended");
    // R_BSP_IrqStatusClear: IELSRn &= ~IR.
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(LINE), AGT0_COMPARE_A)
        .unwrap();
    assert!(
        !m.bus.is_nvic_irq_pending(16 + LINE),
        "IR cleared but the NVIC line is still pending"
    );
}
