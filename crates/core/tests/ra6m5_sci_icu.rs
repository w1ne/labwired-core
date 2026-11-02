// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RA6M5 SCI0 (classic SCI, 16-stage FIFO) interrupts reach the core only
//! through the ICU: RXI, TXI and TEI are ELC events (0x180/0x181/0x182, FSP
//! ra6m5 `bsp_elc.h`), linked to NVIC lines by `IELSRn` (Zephyr
//! ra6-cm33-common.dtsi wires sci0 to lines 0-3). The enables are SCR bits
//! (TIE 7, RIE 6, TE 5, RE 4, TEIE 2). Built from the EK-RA6M5 descriptor,
//! so the chip YAML's `elc_event` / `clock_hz` / `baud` on sci0 are what is
//! under test: one 10-bit frame at 115200 baud is 17 361 cycles of the
//! 200 MHz guest clock.

mod common;
use common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::cpu::CortexM;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Cpu, Machine};

const SCI0: u64 = 0x4011_8000;
const SCR: u64 = SCI0 + 0x02;
const TDR: u64 = SCI0 + 0x03;
const SSR: u64 = SCI0 + 0x04;
const FTDRL: u64 = SCI0 + 0x0F;
const FRDRL: u64 = SCI0 + 0x11;
const FCR: u64 = SCI0 + 0x14;
const FDR: u64 = SCI0 + 0x16;
const TIE: u8 = 1 << 7;
const RIE: u8 = 1 << 6;
const TE: u8 = 1 << 5;
const RE: u8 = 1 << 4;
const TEIE: u8 = 1 << 2;
/// FCR: FM, RTRG = 8 (FSP r_sci_uart with Zephyr's rx_fifo_trigger 8).
const FCR_FIFO_RTRG8: u16 = 1 | (8 << 8);
const SSR_FIFO_TDFE: u8 = 1 << 7;
const SSR_FIFO_RDF: u8 = 1 << 6;
const SSR_FIFO_TEND: u8 = 1 << 2;
const SSR_FIFO_DR: u8 = 1 << 0;
const IELSR0: u64 = 0x4000_6300;
const IR: u32 = 1 << 16;
const SCI0_RXI: u32 = 0x180;
const SCI0_TXI: u32 = 0x181;
const SCI0_TEI: u32 = 0x182;
const RXI_LINE: u32 = 0;
const TXI_LINE: u32 = 1;
const TEI_LINE: u32 = 2;
const FRAME: u32 = 17_361;
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
    m.bus.write_u16(u64::from(IDLE), 0xE7FE).unwrap();
    m.bus.write_u16(u64::from(HANDLER), 0xE7FE).unwrap();
    for line in [RXI_LINE, TXI_LINE, TEI_LINE] {
        m.bus
            .write_u32(u64::from(TABLE + 4 * (16 + line)), HANDLER | 1)
            .unwrap();
    }
    m.bus.write_u32(SCB_VTOR, TABLE).unwrap();
    m.cpu.set_pc(IDLE);
    m.cpu.set_sp(0x2000_7F00);
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
fn zephyr_irq_tx_enable_vectors_txi_through_ielsr() {
    let mut m = machine();
    link(&mut m, TXI_LINE, SCI0_TXI);
    enable(&mut m, TXI_LINE);
    // FSP r_sci_uart_open leaves TE | RE | RIE; Zephyr's irq_tx_enable then
    // does `SCR |= TIE | TEIE`, so SCR has to read back.
    m.bus.write_u8(SCR, TE | RE | RIE).unwrap();
    let scr = m.bus.read_u8(SCR).unwrap();
    m.bus.write_u8(SCR, scr | TIE | TEIE).unwrap();
    assert_eq!(m.bus.read_u8(SCR).unwrap(), TE | RE | RIE | TIE | TEIE);
    for _ in 0..100 {
        if m.cpu.get_pc() == HANDLER {
            break;
        }
        m.step().unwrap();
    }
    assert_eq!(m.cpu.get_pc(), HANDLER, "TXI never vectored the core");
    assert_eq!(
        m.bus.read_u32(IELSR0 + 4 * u64::from(TXI_LINE)).unwrap(),
        IR | SCI0_TXI
    );
}

#[test]
fn tei_comes_one_frame_after_the_last_fifo_byte() {
    let mut m = machine();
    link(&mut m, TEI_LINE, SCI0_TEI);
    m.bus.write_u16(FCR, FCR_FIFO_RTRG8).unwrap();
    m.bus.write_u8(SCR, TE).unwrap();
    m.bus.write_u8(FTDRL, b'x').unwrap();
    // TEIE while the frame is on the wire: TEND is still 0.
    m.bus.write_u8(SCR, TE | TEIE).unwrap();
    let n = steps_until_pending(&mut m, TEI_LINE, 2 * FRAME).expect("TEI never pended");
    assert!(
        (FRAME - 10..=FRAME + 10).contains(&n),
        "TEI after {n} steps, frame is {FRAME}"
    );
    // The handler's IR clear drops the pend, and nothing re-raises it.
    link(&mut m, TEI_LINE, SCI0_TEI);
    assert!(!pending(&m, TEI_LINE));
    assert_eq!(steps_until_pending(&mut m, TEI_LINE, 2 * FRAME), None);
}

#[test]
fn fifo_rxi_and_ssr_fifo_follow_the_receive_queue() {
    let mut m = machine();
    link(&mut m, RXI_LINE, SCI0_RXI);
    m.bus.write_u16(FCR, FCR_FIFO_RTRG8).unwrap();
    m.bus.write_u8(SCR, RE | RIE).unwrap();
    assert_eq!(m.bus.read_u8(SSR).unwrap(), SSR_FIFO_TDFE | SSR_FIFO_TEND);
    m.bus.deliver_uart_rx_named("sci0", b"ab").unwrap();
    assert!(steps_until_pending(&mut m, RXI_LINE, 2048).is_some());
    // Fewer than RTRG characters: DR, not RDF. FDR.R counts them.
    assert_eq!(
        m.bus.read_u8(SSR).unwrap(),
        SSR_FIFO_TDFE | SSR_FIFO_TEND | SSR_FIFO_DR
    );
    assert_eq!(m.bus.read_u8(FDR).unwrap(), 2);
    assert_eq!(m.bus.read_u8(FRDRL).unwrap(), b'a');
    link(&mut m, RXI_LINE, SCI0_RXI);
    assert!(!pending(&m, RXI_LINE));
    // The second character is a frame behind the first.
    let n = steps_until_pending(&mut m, RXI_LINE, 2 * FRAME).expect("second RXI");
    assert!(n > FRAME - 2048, "second RXI after only {n} steps");
    assert_eq!(m.bus.read_u8(FRDRL).unwrap(), b'b');
    link(&mut m, RXI_LINE, SCI0_RXI);
    assert_eq!(m.bus.read_u8(SSR).unwrap(), SSR_FIFO_TDFE | SSR_FIFO_TEND);
    // Queue empty: no more RXI.
    assert_eq!(steps_until_pending(&mut m, RXI_LINE, 2 * FRAME), None);
    // RTRG characters waiting: RDF.
    m.bus.deliver_uart_rx_named("sci0", b"01234567").unwrap();
    assert_eq!(
        m.bus.read_u8(SSR).unwrap(),
        SSR_FIFO_TDFE | SSR_FIFO_TEND | SSR_FIFO_RDF
    );
}

#[test]
fn rxi_needs_rie() {
    let mut m = machine();
    link(&mut m, RXI_LINE, SCI0_RXI);
    m.bus.write_u8(SCR, RE).unwrap();
    m.bus.deliver_uart_rx_named("sci0", b"a").unwrap();
    assert_eq!(steps_until_pending(&mut m, RXI_LINE, 4096), None);
}

#[test]
fn non_fifo_ssr_keeps_its_classic_meaning() {
    let mut m = machine();
    // FM = 0: SSR is TDRE | TEND, plus RDRF with a character waiting.
    assert_eq!(m.bus.read_u8(SSR).unwrap(), 0x84);
    m.bus.deliver_uart_rx_named("sci0", b"z").unwrap();
    assert_eq!(m.bus.read_u8(SSR).unwrap(), 0xC4);
}

#[test]
fn non_fifo_tdr_paces_txi_by_the_frame() {
    let mut m = machine();
    link(&mut m, TXI_LINE, SCI0_TXI);
    m.bus.write_u8(SCR, TE | TIE).unwrap();
    assert!(steps_until_pending(&mut m, TXI_LINE, 100).is_some());
    link(&mut m, TXI_LINE, SCI0_TXI);
    // Idle shifter: the first byte moves in at once (TXI), the second waits
    // a frame in TDR.
    m.bus.write_u8(TDR, b'o').unwrap();
    assert!(steps_until_pending(&mut m, TXI_LINE, 100).is_some());
    link(&mut m, TXI_LINE, SCI0_TXI);
    m.bus.write_u8(TDR, b'k').unwrap();
    let n = steps_until_pending(&mut m, TXI_LINE, 2 * FRAME).expect("second TXI");
    assert!(
        (FRAME - 200..=FRAME + 10).contains(&n),
        "second TXI after {n} steps, frame is {FRAME}"
    );
}
