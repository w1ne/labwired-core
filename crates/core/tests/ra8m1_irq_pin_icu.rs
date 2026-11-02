// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA8M1 external IRQ pins reach the core through the ICU: an edge on S1
//! (P009, IRQ13 per Zephyr `r7fa8m1xh.dtsi`) that matches `IRQCR13.IRQMD`,
//! on a pad whose `PmnPFS.ISEL` is set, raises ELC_EVENT_ICU_IRQ13 (0x00E,
//! FSP ra8m1 `bsp_elc.h`), which `IELSRn` links to an NVIC line. Built from
//! the EK-RA8M1 descriptor, so the chip YAML's two `ra_icu` windows (IRQ
//! pins at R_ICU_BASE, IELSR at R_ICU_BASE + 0x6000) are what is under test,
//! and the edge enters where a board button drives it: the pad.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const IRQCR13: u64 = 0x4000_600D;
const IELSR0: u64 = 0x4000_C300;
const IR: u32 = 1 << 16;
const ICU_IRQ13: u32 = 0x00E;
/// Zephyr ek_ra8m1.dts `port_irq13` line.
const LINE: u32 = 89;
const PFS_P009: u64 = 0x4040_0800 + 9 * 4;
const PWPRS: u64 = 0x4040_0D14;
const PFS_PCR: u32 = 1 << 4;
const PFS_ISEL: u32 = 1 << 14;
const TABLE: u32 = 0x2200_0400;
const IDLE: u32 = 0x2200_2000;
const HANDLER: u32 = 0x2200_2100;
const NVIC_ISER: u64 = 0xE000_E100;
const SCB_VTOR: u64 = 0xE000_ED08;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/r7fa8m1ah.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("r7fa8m1ah chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/ek-ra8m1.yaml")).expect("ek-ra8m1");
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

/// What Zephyr's button sample sets up: P009 pulled up (and an IRQ input
/// when `isel`), `IRQCR13.IRQMD = irqmd`, IELSR89 -> ICU_IRQ13, line enabled.
fn configure(m: &mut Machine<CortexM>, isel: bool, irqmd: u8) {
    // R_BSP_PinAccessEnable: clear B0WI, then set PFSWE.
    m.bus.write_u8(PWPRS, 0).unwrap();
    m.bus.write_u8(PWPRS, 1 << 6).unwrap();
    let pfs = PFS_PCR | if isel { PFS_ISEL } else { 0 };
    m.bus.write_u32(PFS_P009, pfs).unwrap();
    m.bus.write_u8(IRQCR13, irqmd).unwrap();
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(LINE), ICU_IRQ13)
        .unwrap();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(LINE / 32), 1 << (LINE % 32))
        .unwrap();
}

/// Drive S1's pad (`true` = released, high) and run a few steps.
fn drive_s1(m: &mut Machine<CortexM>, level: bool) -> bool {
    let port0 = m.bus.find_peripheral_index_by_name("port0").unwrap();
    assert!(m.bus.set_peripheral_gpio_input(port0, 9, level));
    for _ in 0..64 {
        if m.cpu.get_pc() == HANDLER {
            return true;
        }
        m.step().unwrap();
    }
    m.cpu.get_pc() == HANDLER
}

#[test]
fn a_falling_edge_on_s1_vectors_the_core_through_irq13() {
    let mut m = machine();
    configure(&mut m, true, 0);
    assert!(!drive_s1(&mut m, true), "the released level is not an edge");
    assert!(
        drive_s1(&mut m, false),
        "S1 press never reached line {LINE}"
    );
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(LINE)).unwrap(),
        IR | ICU_IRQ13
    );
}

#[test]
fn a_pad_without_isel_raises_nothing() {
    let mut m = machine();
    configure(&mut m, false, 0);
    drive_s1(&mut m, true);
    assert!(!drive_s1(&mut m, false));
    assert!(!m.bus.is_nvic_irq_pending(16 + LINE));
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(LINE)).unwrap() & IR,
        0
    );
}

#[test]
fn rising_mode_takes_the_release_not_the_press() {
    let mut m = machine();
    configure(&mut m, true, 1);
    drive_s1(&mut m, true);
    assert!(!drive_s1(&mut m, false), "IRQMD rising fired on the press");
    assert!(drive_s1(&mut m, true), "IRQMD rising missed the release");
}
