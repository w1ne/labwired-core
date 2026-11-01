// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA8M1 SCI9 (SCI_B) interrupts reach the core only through the ICU: RXI,
//! TXI and TEI are ELC events (0x163/0x164/0x165, FSP ra8m1 `bsp_elc.h`),
//! linked to NVIC lines by `IELSRn` (Zephyr ra8x1.dtsi wires sci9 to lines
//! 24-27). Built from the EK-RA8M1 descriptor, so the chip YAML's
//! `elc_event` / `clock_hz` / `baud` on sci9 are what is under test: one
//! 10-bit frame at 115200 baud is 41 666 cycles of the 480 MHz guest clock.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const SCI9: u64 = 0x4035_8900;
const RDR: u64 = SCI9;
const TDR: u64 = SCI9 + 0x04;
const CCR0: u64 = SCI9 + 0x08;
const FRSR: u64 = SCI9 + 0x50;
const RE: u32 = 1 << 0;
const TE: u32 = 1 << 4;
const RIE: u32 = 1 << 16;
const TIE: u32 = 1 << 20;
const TEIE: u32 = 1 << 21;
const IELSR0: u64 = 0x4000_C300;
const IR: u32 = 1 << 16;
const SCI9_RXI: u32 = 0x163;
const SCI9_TXI: u32 = 0x164;
const SCI9_TEI: u32 = 0x165;
/// Zephyr ra8x1.dtsi sci9 `interrupts = <24 1>, <25 1>, <26 1>, <27 1>`.
const RXI_LINE: u32 = 24;
const TXI_LINE: u32 = 25;
const TEI_LINE: u32 = 26;
const FRAME: u32 = 41_666;
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
    for line in [RXI_LINE, TXI_LINE, TEI_LINE] {
        m.bus
            .write_u32(u64::from(TABLE + 4 * (16 + line)), HANDLER | 1)
            .unwrap();
    }
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2200_7F00);
    m
}

fn link(m: &mut Machine<CortexM>, line: u32, event: u32) {
    m.bus
        .write_u32(IELSR0 + 4 * u64::from(line), event)
        .unwrap();
}

fn enable(m: &mut Machine<CortexM>, line: u32) {
    m.bus
        .write_u32(NVIC_ISER + 4 * u64::from(line / 32), 1 << (line % 32))
        .unwrap();
}

fn pending(m: &Machine<CortexM>, line: u32) -> bool {
    m.bus.is_nvic_irq_pending(16 + line)
}

/// Step until `line` is pending; the number of steps it took.
fn steps_until_pending(m: &mut Machine<CortexM>, line: u32, budget: u32) -> Option<u32> {
    for n in 0..budget {
        if pending(m, line) {
            return Some(n);
        }
        m.step().unwrap();
    }
    None
}

#[test]
fn tie_on_an_idle_transmitter_vectors_txi_through_ielsr() {
    let mut m = machine();
    link(&mut m, TXI_LINE, SCI9_TXI);
    enable(&mut m, TXI_LINE);
    // FSP r_sci_b_uart: TE and TIE set by one store.
    m.bus.write_u32(CCR0, TE | TIE).unwrap();
    for _ in 0..100 {
        if m.cpu.get_pc() == HANDLER {
            break;
        }
        m.step().unwrap();
    }
    assert_eq!(m.cpu.get_pc(), HANDLER, "TXI never vectored the core");
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(TXI_LINE)).unwrap(),
        IR | SCI9_TXI
    );
}

#[test]
fn tei_comes_one_frame_after_the_last_byte() {
    let mut m = machine();
    link(&mut m, TEI_LINE, SCI9_TEI);
    m.bus.write_u32(CCR0, TE).unwrap();
    m.bus.write_u8(TDR, b'x').unwrap();
    // TEIE while the frame is on the wire: TEND is still 0.
    m.bus.write_u32(CCR0, TE | TEIE).unwrap();
    let n = steps_until_pending(&mut m, TEI_LINE, 2 * FRAME).expect("TEI never pended");
    assert!(
        (FRAME - 10..=FRAME + 10).contains(&n),
        "TEI after {n} steps, frame is {FRAME}"
    );
    // The handler's IR clear drops the pend, and nothing re-raises it.
    link(&mut m, TEI_LINE, SCI9_TEI);
    assert!(!pending(&m, TEI_LINE));
    assert_eq!(steps_until_pending(&mut m, TEI_LINE, 2 * FRAME), None);
}

#[test]
fn rxi_is_raised_per_character_a_frame_apart() {
    let mut m = machine();
    link(&mut m, RXI_LINE, SCI9_RXI);
    m.bus.write_u32(CCR0, RE | RIE).unwrap();
    m.bus.deliver_uart_rx_named("sci9", b"ab").unwrap();
    assert!(steps_until_pending(&mut m, RXI_LINE, 2048).is_some());
    // FRSR.R counts the receive FIFO.
    assert_eq!((m.bus.read_u32(FRSR).unwrap() >> 8) & 0x3F, 2);
    assert_eq!(m.bus.read_u8(RDR).unwrap(), b'a');
    link(&mut m, RXI_LINE, SCI9_RXI);
    assert!(!pending(&m, RXI_LINE));
    // The second character is a frame behind the first.
    let n = steps_until_pending(&mut m, RXI_LINE, 2 * FRAME).expect("second RXI");
    assert!(n > FRAME - 2048, "second RXI after only {n} steps");
    assert_eq!(m.bus.read_u8(RDR).unwrap(), b'b');
    link(&mut m, RXI_LINE, SCI9_RXI);
    // Queue empty: no more RXI.
    assert_eq!(steps_until_pending(&mut m, RXI_LINE, 2 * FRAME), None);
}

#[test]
fn rxi_needs_rie() {
    let mut m = machine();
    link(&mut m, RXI_LINE, SCI9_RXI);
    m.bus.write_u32(CCR0, RE).unwrap();
    m.bus.deliver_uart_rx_named("sci9", b"a").unwrap();
    assert_eq!(steps_until_pending(&mut m, RXI_LINE, 4096), None);
}
