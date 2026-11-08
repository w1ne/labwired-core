// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V203 DMA1 (`profile: ch32v20x`) on the COMMITTED ch32fun-smoke chip
//! profile: the CH32V003 channel semantics with the V20x PFIC numbers,
//! DMA1_ChannelN_IRQn = 26 + N (ch32fun ch32v20xhw.h), and ch32fun's
//! examples_v20x/spidmatest refilling its SPI2 buffer from the channel 5
//! HT/TC interrupt.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::{system::riscv::configure_riscv, Bus, Machine};
use std::path::PathBuf;
use std::sync::atomic::Ordering;

const RCC_AHBPCENR: u64 = 0x4002_1014;
const DMA1: u64 = 0x4002_0000;
const INTFR: u64 = DMA1;
const INTFCR: u64 = DMA1 + 0x04;
const CH5_CFGR: u64 = DMA1 + 0x08 + 4 * 20;
const CH5_CNTR: u64 = CH5_CFGR + 0x04;
const CH5_PADDR: u64 = CH5_CFGR + 0x08;
const CH5_MADDR: u64 = CH5_CFGR + 0x0C;
const DMA1_CHANNEL5_IRQ: u32 = 31;
/// spidmatest's `sendbuff` (nm): 256 halfwords at the start of SRAM.
const SENDBUFF: u64 = 0x2000_0000;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bus() -> SystemBus {
    let chip =
        ChipDescriptor::from_file(root().join("configs/chips/ch32v203c8t6-ch32fun-smoke.yaml"))
            .unwrap();
    let manifest: SystemManifest =
        serde_yaml::from_str("name: ch32v203\nchip: unused\nexternal_devices: []\n").unwrap();
    SystemBus::from_config(&chip, &manifest).unwrap()
}

#[test]
fn channel_5_flags_are_pfic_level_31_until_intfcr() {
    let mut bus = bus();
    let _cpu = configure_riscv(&mut bus);
    let pfic = bus
        .peripherals
        .iter()
        .find_map(|p| p.dev.qingke_pfic_state())
        .unwrap();
    let level =
        |irq: u32| pfic.level[(irq / 32) as usize].load(Ordering::SeqCst) & (1 << (irq % 32)) != 0;
    bus.write_u32(RCC_AHBPCENR, 1).unwrap(); // DMA1EN
                                             // A one-item mem-to-mem copy on channel 5 latches TCIF5.
    bus.write_u32(0x2000_0100, 0x5A).unwrap();
    bus.write_u32(CH5_PADDR, 0x2000_0200).unwrap();
    bus.write_u32(CH5_MADDR, 0x2000_0100).unwrap();
    bus.write_u32(CH5_CNTR, 1).unwrap();
    bus.write_u32(CH5_CFGR, (1 << 14) | (1 << 4) | (1 << 1) | 1)
        .unwrap();
    assert!(!level(DMA1_CHANNEL5_IRQ));
    let dma = bus.find_peripheral_index_by_name("dma1").unwrap();
    for _ in 0..4 {
        let res = bus.peripherals[dma].dev.tick_elapsed_forced(1);
        for req in res.dma_requests.unwrap_or_default() {
            bus.execute_dma(std::slice::from_ref(&req)).unwrap();
            bus.peripherals[dma].dev.dma_transfer_complete(&req);
        }
    }
    assert_eq!(bus.read_u32(0x2000_0200).unwrap() & 0xFF, 0x5A);
    assert_eq!(
        bus.read_u32(INTFR).unwrap() & 0xF_0000,
        0x3_0000,
        "TCIF5 | GIF5"
    );
    assert!(
        level(DMA1_CHANNEL5_IRQ),
        "TCIF5 & TCIE requests interrupt 31"
    );
    // Not the CH32V003 numbering: channel 5 there is 26.
    assert!(!level(26));
    bus.write_u32(INTFCR, 1 << 17).unwrap(); // CTCIF5
    assert_eq!(bus.read_u32(INTFR).unwrap() & 0xF_0000, 0);
    assert!(!level(DMA1_CHANNEL5_IRQ));
}

#[test]
fn ch32fun_spidmatest_refills_both_halves_from_the_channel_5_handler() {
    let mut bus = bus();
    let image = labwired_loader::load_elf(
        &root().join("examples/weact-bluepill-plus-ch32v203/ch32fun/spidmatest.elf"),
    )
    .expect("committed ch32fun spidmatest ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).unwrap();
    let half = |m: &Machine<_>, i: u64| m.bus.read_u16(SENDBUFF + 2 * i).unwrap();
    let mut refilled = false;
    for _ in 0..200_000 {
        machine.step().expect("spidmatest instruction");
        if half(&machine, 0) == 0x0AAF && half(&machine, 255) == 0x0AAF {
            refilled = true;
            break;
        }
    }
    assert!(
        refilled,
        "DMA1_Channel5_IRQHandler never refilled both halves (HT then TC)"
    );
    assert_eq!(half(&machine, 127), 0x0AAF);
    assert_eq!(half(&machine, 128), 0x0AAF);
}
