// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Executing oracle for the ESP32-S3 GDMA (general DMA) mem-to-mem engine.
//!
//! `esp32s3/gdma.rs` is a ~4000-line behavioural model that had rich in-module
//! unit tests but no ratchet-discoverable integration coverage. This file
//! drives the controller the same way firmware does — build linked-list DMA
//! descriptors in real DRAM, arm `MEM_TRANS_EN`, kick `OUTLINK_START` /
//! `INLINK_START`, then `tick_with_bus` — and asserts that bytes were ACTUALLY
//! moved through the bus router (not merely that registers latched). Everything
//! goes through the public `Peripheral` MMIO API (`read_u32`/`write_u32`) and a
//! real `SystemBus`, so a register or descriptor-decode regression fails here.
//!
//! Named `esp32s3_*` so the board-coverage ratchet discovers it.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32s3::gdma::Esp32s3Gdma;
use labwired_core::system::xtensa::RamPeripheral;
use labwired_core::{Bus, Peripheral};

// Per-channel register block stride and offsets (base-relative, what the
// `Peripheral` MMIO API expects). Mirrors the S3 GDMA register map.
const CHANNEL_STRIDE: u64 = 0xC0;
const IN_CONF0: u64 = 0x00;
const IN_INT_RAW: u64 = 0x08;
const IN_INT_CLR: u64 = 0x14;
const IN_LINK: u64 = 0x20;
const OUT_INT_RAW: u64 = 0x68;
const OUT_INT_CLR: u64 = 0x74;
const OUT_LINK: u64 = 0x80;

const MEM_TRANS_EN: u32 = 1 << 4; // IN_CONF0.mem_trans_en
const IN_LINK_START: u32 = 1 << 22;
const OUT_LINK_START: u32 = 1 << 21;
const LINK_ADDR_MASK: u32 = 0x000F_FFFF;
const IN_SUC_EOF: u32 = 1 << 1; // IN_INT_RAW.in_suc_eof
const OUT_TOTAL_EOF: u32 = 1 << 3; // OUT_INT_RAW.out_total_eof

// Real ESP32-S3 RX-channel-0 interrupt source id.
const IN_CH0_SRC: u32 = 66;

// The S3 DRAM window the model reconstructs descriptor addresses into
// (`DRAM_ADDR_PREFIX | link_addr[19:0]`). Descriptors and buffers must live
// here so the descriptor walk resolves through the bus router.
const DRAM_BASE: u64 = 0x3FC8_8000;
const DRAM_SIZE: usize = 256 * 1024;

fn ch_base(n: u64) -> u64 {
    n * CHANNEL_STRIDE
}

fn bus_with_dram() -> SystemBus {
    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "dram_test",
        DRAM_BASE,
        DRAM_SIZE as u64,
        None,
        Box::new(RamPeripheral::new(DRAM_SIZE)),
    );
    bus
}

fn write_desc(bus: &mut SystemBus, addr: u64, dw0: u32, buffer: u64, next: u64) {
    bus.write_u32(addr, dw0).unwrap();
    bus.write_u32(addr + 4, buffer as u32).unwrap();
    bus.write_u32(addr + 8, next as u32).unwrap();
}

// TX descriptor dw0: owner=DMA, suc_eof, length=size=len.
fn tx_dw0(len: u32) -> u32 {
    (1 << 31) | (1 << 30) | (len << 12) | len
}

// RX descriptor dw0: owner=DMA, size (length filled by the engine).
fn rx_dw0(size: u32) -> u32 {
    (1 << 31) | size
}

/// A single-descriptor mem-to-mem transfer moves the exact source bytes to the
/// destination and latches IN_SUC_EOF / OUT_TOTAL_EOF.
#[test]
fn gdma_mem2mem_single_descriptor_moves_bytes() {
    let mut bus = bus_with_dram();

    let src: u64 = DRAM_BASE;
    let dst: u64 = DRAM_BASE + 0x1000;
    let payload: &[u8] = b"LABWIRED-S3-GDMA-ORACLE!\x00";
    let len = payload.len() as u32;

    for (i, &b) in payload.iter().enumerate() {
        bus.write_u8(src + i as u64, b).unwrap();
        // Poison the destination so a no-op "transfer" cannot pass.
        bus.write_u8(dst + i as u64, 0x5A).unwrap();
    }

    let tx_desc: u64 = DRAM_BASE + 0x2000;
    let rx_desc: u64 = DRAM_BASE + 0x3000;
    write_desc(&mut bus, tx_desc, tx_dw0(len), src, 0);
    write_desc(&mut bus, rx_desc, rx_dw0(len), dst, 0);

    let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
    let b = ch_base(0);

    g.write_u32(b + IN_CONF0, MEM_TRANS_EN).unwrap();
    g.write_u32(b + IN_INT_CLR, 0xFFFF_FFFF).unwrap();
    g.write_u32(b + OUT_INT_CLR, 0xFFFF_FFFF).unwrap();
    g.write_u32(
        b + IN_LINK,
        ((rx_desc as u32) & LINK_ADDR_MASK) | IN_LINK_START,
    )
    .unwrap();
    g.write_u32(
        b + OUT_LINK,
        ((tx_desc as u32) & LINK_ADDR_MASK) | OUT_LINK_START,
    )
    .unwrap();

    // The copy is deferred to the bus-tick, not done inside the register write.
    assert!(
        g.needs_bus_tick(),
        "mem2mem must be pending before bus tick"
    );
    assert_eq!(
        g.read_u32(b + IN_INT_RAW).unwrap() & IN_SUC_EOF,
        0,
        "IN_SUC_EOF must not latch before the descriptor walk"
    );

    g.tick_with_bus(&mut bus);

    assert_ne!(
        g.read_u32(b + IN_INT_RAW).unwrap() & IN_SUC_EOF,
        0,
        "IN_SUC_EOF must latch after the walk"
    );
    assert_ne!(
        g.read_u32(b + OUT_INT_RAW).unwrap() & OUT_TOTAL_EOF,
        0,
        "OUT_TOTAL_EOF must latch after the walk"
    );

    let moved: Vec<u8> = (0..payload.len())
        .map(|i| bus.read_u8(dst + i as u64).unwrap())
        .collect();
    assert_eq!(moved, payload, "GDMA must copy the exact source bytes");
    assert!(!g.needs_bus_tick(), "pending_m2m must clear after the walk");
}

/// A two-link scatter/gather chain concatenates both source fragments into a
/// single contiguous destination — exercising the descriptor `next` walk.
#[test]
fn gdma_mem2mem_two_link_chain_concatenates() {
    let mut bus = bus_with_dram();

    let src_a: u64 = DRAM_BASE;
    let src_b: u64 = DRAM_BASE + 0x400;
    let dst: u64 = DRAM_BASE + 0x1000;
    let frag_a: &[u8] = b"HELLO-";
    let frag_b: &[u8] = b"WORLD!";
    let expected: Vec<u8> = frag_a.iter().chain(frag_b.iter()).copied().collect();

    for (i, &b) in frag_a.iter().enumerate() {
        bus.write_u8(src_a + i as u64, b).unwrap();
    }
    for (i, &b) in frag_b.iter().enumerate() {
        bus.write_u8(src_b + i as u64, b).unwrap();
    }
    for i in 0..expected.len() {
        bus.write_u8(dst + i as u64, 0x00).unwrap();
    }

    // Two TX descriptors chained via `next`; single RX sink big enough for both.
    let tx0: u64 = DRAM_BASE + 0x2000;
    let tx1: u64 = DRAM_BASE + 0x2010;
    let rx: u64 = DRAM_BASE + 0x3000;
    // First TX fragment: owner, length, NOT suc_eof (more to come), next=tx1.
    write_desc(
        &mut bus,
        tx0,
        (1 << 31) | ((frag_a.len() as u32) << 12) | frag_a.len() as u32,
        src_a,
        tx1,
    );
    write_desc(&mut bus, tx1, tx_dw0(frag_b.len() as u32), src_b, 0);
    write_desc(&mut bus, rx, rx_dw0(expected.len() as u32), dst, 0);

    let mut g = Esp32s3Gdma::new(IN_CH0_SRC);
    let b = ch_base(0);
    g.write_u32(b + IN_CONF0, MEM_TRANS_EN).unwrap();
    g.write_u32(b + IN_INT_CLR, 0xFFFF_FFFF).unwrap();
    g.write_u32(b + OUT_INT_CLR, 0xFFFF_FFFF).unwrap();
    g.write_u32(b + IN_LINK, ((rx as u32) & LINK_ADDR_MASK) | IN_LINK_START)
        .unwrap();
    g.write_u32(
        b + OUT_LINK,
        ((tx0 as u32) & LINK_ADDR_MASK) | OUT_LINK_START,
    )
    .unwrap();

    // Drain the transfer (walk may span more than one bus tick for a chain).
    for _ in 0..8 {
        if !g.needs_bus_tick() {
            break;
        }
        g.tick_with_bus(&mut bus);
    }

    assert!(!g.needs_bus_tick(), "chained walk must complete");
    assert_ne!(
        g.read_u32(b + IN_INT_RAW).unwrap() & IN_SUC_EOF,
        0,
        "IN_SUC_EOF must latch once the RX sink sees suc_eof"
    );

    let moved: Vec<u8> = (0..expected.len())
        .map(|i| bus.read_u8(dst + i as u64).unwrap())
        .collect();
    assert_eq!(
        moved, expected,
        "chained TX fragments must land contiguously in the RX buffer"
    );
}
