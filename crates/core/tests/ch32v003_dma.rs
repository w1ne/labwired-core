// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 DMA1 on the COMMITTED ch32fun-smoke chip profile: USART1 TX on
//! channel 4, the channel's flags as a PFIC level (DMA1_Channel4_IRQn = 25),
//! and the request LEVEL a channel sees when it is enabled.
//!
//! ch32fun's uart_tx_dma sets USART1 CTLR3.DMAT in uart_setup() and only
//! later enables DMA1 channel 4. A request line is a level (TXE & DMAT), so
//! on silicon the channel starts at once. The engine used to deliver the
//! USART's TX request only as an edge at the DMAT / DATAR store, which a
//! disabled channel drops, so the example sent nothing.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::qingke_pfic::QingkePficState;
use labwired_core::{system::riscv::configure_riscv, Bus, Machine};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const RCC_AHBPCENR: u64 = 0x4002_1014;
const RCC_APB2PCENR: u64 = 0x4002_1018;
const USART1: u64 = 0x4001_3800;
const USART1_DATAR: u64 = USART1 + 0x04;
const USART1_CTLR1: u64 = USART1 + 0x0C;
const USART1_CTLR3: u64 = USART1 + 0x14;
const DMA1: u64 = 0x4002_0000;
const INTFR: u64 = DMA1;
const INTFCR: u64 = DMA1 + 0x04;
const CH4_CFGR: u64 = DMA1 + 0x08 + 3 * 20;
const CH4_CNTR: u64 = CH4_CFGR + 0x04;
const CH4_PADDR: u64 = CH4_CFGR + 0x08;
const CH4_MADDR: u64 = CH4_CFGR + 0x0C;
const GPIOD_OUTDR: u64 = 0x4001_140C;
const DMA1_CHANNEL4_IRQ: u32 = 25;
/// uart_tx_dma's CFGR: MINC | DIR (memory to peripheral) | TCIE.
const CFGR_TX: u32 = (1 << 7) | (1 << 4) | (1 << 1);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml")).unwrap()
}

fn manifest() -> SystemManifest {
    serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap()
}

fn bus_and_pfic() -> (SystemBus, Arc<QingkePficState>) {
    let mut bus = SystemBus::from_config(&chip(), &manifest()).unwrap();
    let _cpu = configure_riscv(&mut bus);
    let pfic = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.qingke_pfic_state())
        .unwrap();
    bus.write_u32(RCC_AHBPCENR, 1).unwrap(); // DMA1EN
    bus.write_u32(RCC_APB2PCENR, (1 << 14) | (1 << 5)).unwrap(); // USART1EN, IOPDEN
    (bus, pfic)
}

/// Channel 4's `active` flag: a latched request waiting for the arbiter.
fn ch4_has_request(bus: &SystemBus) -> bool {
    let dma = bus.find_peripheral_index_by_name("dma1").unwrap();
    bus.peripherals[dma].dev.snapshot()["channels"][3]["active"]
        .as_bool()
        .unwrap()
}

/// uart_tx_dma's setup order: USART1 with DMAT first, then channel 4 armed
/// with `len` items and enabled.
fn uart_then_channel(bus: &mut SystemBus, dmat: bool, len: u32) {
    bus.write_u16(USART1_CTLR1, 1 << 3).unwrap(); // TE
    bus.write_u16(USART1_CTLR3, if dmat { 1 << 7 } else { 0 })
        .unwrap();
    bus.write_u16(USART1_CTLR1, (1 << 13) | (1 << 3)).unwrap(); // UE | TE
    bus.write_u32(CH4_PADDR, USART1_DATAR as u32).unwrap();
    bus.write_u32(CH4_CFGR, CFGR_TX).unwrap();
    bus.write_u32(CH4_CNTR, len).unwrap();
    bus.write_u32(CH4_MADDR, 0x2000_0100).unwrap();
    assert!(!ch4_has_request(bus), "nothing asks before EN");
    bus.write_u32(CH4_CFGR, CFGR_TX | 1).unwrap();
}

#[test]
fn enabling_channel_4_serves_a_standing_usart1_tx_request() {
    let (mut bus, _pfic) = bus_and_pfic();
    uart_then_channel(&mut bus, true, 14);
    assert!(
        ch4_has_request(&bus),
        "TXE & DMAT was already high when EN rose: the channel must be served"
    );
}

#[test]
fn without_dmat_the_enabled_channel_waits() {
    let (mut bus, _pfic) = bus_and_pfic();
    uart_then_channel(&mut bus, false, 14);
    assert!(!ch4_has_request(&bus), "no DMAT, no request");
}

#[test]
fn channel_4_flags_are_a_pfic_level_until_intfcr() {
    let (mut bus, pfic) = bus_and_pfic();
    let requested = || pfic.level[0].load(Ordering::SeqCst) & (1 << DMA1_CHANNEL4_IRQ) != 0;
    // A one-item mem-to-mem copy on channel 4 latches TCIF4 without a USART.
    bus.write_u32(0x2000_0100, 0x5A).unwrap();
    bus.write_u32(CH4_PADDR, 0x2000_0200).unwrap();
    bus.write_u32(CH4_MADDR, 0x2000_0100).unwrap();
    bus.write_u32(CH4_CNTR, 1).unwrap();
    bus.write_u32(CH4_CFGR, (1 << 14) | (1 << 4) | (1 << 1) | 1)
        .unwrap();
    assert!(!requested());
    let dma = bus.find_peripheral_index_by_name("dma1").unwrap();
    for _ in 0..4 {
        let res = bus.peripherals[dma].dev.tick_elapsed_forced(1);
        // The bus's copy, then the commit the profile waits for.
        for req in res.dma_requests.unwrap_or_default() {
            bus.execute_dma(std::slice::from_ref(&req)).unwrap();
            bus.peripherals[dma].dev.dma_transfer_complete(&req);
        }
    }
    assert_eq!(bus.read_u32(0x2000_0200).unwrap() & 0xFF, 0x5A);
    assert_eq!(
        bus.read_u32(INTFR).unwrap() & 0xF000,
        0x3000,
        "TCIF4 | GIF4"
    );
    assert!(requested(), "TCIF4 & TCIE requests interrupt 25");
    // The handler's `DMA1->INTFCR = DMA_CTCIF4` drops it on the store.
    bus.write_u32(INTFCR, 1 << 13).unwrap();
    assert_eq!(bus.read_u32(INTFR).unwrap() & 0xF000, 0);
    assert!(!requested());
}

#[test]
fn ch32fun_uart_tx_dma_completes_its_first_transfer() {
    let mut bus = SystemBus::from_config(&chip(), &manifest()).unwrap();
    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v003-evt/ch32fun/uart_tx_dma.elf"))
            .expect("committed ch32fun uart_tx_dma ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).unwrap();
    // led_setup() drives PD6 high; the channel 4 TC handler toggles it.
    let mut was_high = false;
    let mut toggled = false;
    for _ in 0..2_000_000 {
        machine.step().expect("uart_tx_dma instruction");
        let pd6 = machine.bus.read_u32(GPIOD_OUTDR).unwrap() & (1 << 6) != 0;
        was_high |= pd6;
        if was_high && !pd6 {
            toggled = true;
            break;
        }
    }
    assert!(toggled, "DMA1_Channel4_IRQHandler never ran");
    assert_eq!(
        machine.bus.read_u32(CH4_CNTR).unwrap(),
        0,
        "all 14 items moved"
    );
    // The handler wrote CTCIF4 only: HTIF4 and GIF4 stay, TCIF4 is clear.
    assert_eq!(machine.bus.read_u32(INTFR).unwrap() & 0xF000, 0x5000);
}
