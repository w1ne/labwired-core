// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! The CC2538 sleep-timer compare reaches the core: a compare loaded through
//! `ST3`..`ST0` (SWRU319C §13.2) fires the SM Timer interrupt, IRQ 145 in the
//! regular map (Table 5-2), on the bus built from the chip descriptor, and the
//! Cortex-M3 vectors to its handler. The stock Contiki-NG hello-world reads the
//! count but never arms a compare, so this is the only end-to-end check of
//! that path.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const SMWDTHROSC: u64 = 0x400D_5000;
const ST0: u64 = SMWDTHROSC + 0x40;
const ST1: u64 = SMWDTHROSC + 0x44;
const ST2: u64 = SMWDTHROSC + 0x48;
const ST3: u64 = SMWDTHROSC + 0x4C;
const SMT_IRQ: u32 = 145;
/// Vector table in SRAM (VTOR needs 512-byte alignment for 162 vectors).
const TABLE: u32 = 0x2000_0400;
const IDLE: u32 = 0x2000_2000;
const HANDLER: u32 = 0x2000_2100;
const NVIC_ISER: u64 = 0xE000_E100;
const SCB_VTOR: u64 = 0xE000_ED08;

fn machine() -> Machine<CortexM> {
    let chip_path = root("configs/chips/cc2538sf53.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("cc2538sf53 chip");
    let mut manifest =
        SystemManifest::from_file(root("configs/systems/cc2538dk.yaml")).expect("cc2538dk");
    manifest.chip = chip_path.to_str().expect("utf8").to_string();
    let mut bus = labwired_core::bus::SystemBus::from_config(&chip, &manifest).expect("bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    Machine::new(cpu, bus)
}

fn read_count(m: &mut Machine<CortexM>) -> u32 {
    let b0 = m.bus.read_u32(ST0).unwrap();
    b0 | (m.bus.read_u32(ST1).unwrap() << 8)
        | (m.bus.read_u32(ST2).unwrap() << 16)
        | (m.bus.read_u32(ST3).unwrap() << 24)
}

#[test]
fn sleep_timer_compare_vectors_the_core_to_the_smt_handler() {
    let mut m = machine();
    // Idle loop and handler are both `b .`.
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.bus.write_u16(u64::from(HANDLER), 0xE7FE).unwrap();
    m.bus
        .write_u32(u64::from(TABLE + 4 * (16 + SMT_IRQ)), HANDLER | 1)
        .unwrap();
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(SMT_IRQ / 32), 1 << (SMT_IRQ % 32))
        .unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2000_7F00);

    // Compare 20 ticks ahead (~9 800 cycles at 16 MHz), ST0 written last.
    let target = read_count(&mut m).wrapping_add(20);
    m.bus.write_u32(ST3, target >> 24).unwrap();
    m.bus.write_u32(ST2, (target >> 16) & 0xFF).unwrap();
    m.bus.write_u32(ST1, (target >> 8) & 0xFF).unwrap();
    m.bus.write_u32(ST0, target & 0xFF).unwrap();

    let mut fired_at = None;
    for step in 0..200_000u32 {
        if m.cpu.get_pc() == HANDLER {
            fired_at = Some(step);
            break;
        }
        m.step().unwrap();
    }
    let step = fired_at.expect("the SM Timer interrupt never vectored the core");
    // Not before the compare value is reached.
    assert!(
        read_count(&mut m).wrapping_sub(target) < 2,
        "fired at count {} for compare {target} (step {step})",
        read_count(&mut m)
    );
}
