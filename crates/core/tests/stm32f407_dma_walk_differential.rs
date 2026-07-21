// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Executing-fidelity gate for the STM32F407 stream-based DMA controller
//! (RM0090 §10), issue #578. Builds the F407 bus from the shipped
//! `configs/chips/stm32f407.yaml` descriptor — the SAME `from_config` path
//! production uses — and drives a real memory-to-memory transfer on DMA2,
//! asserting every RM0090-mandated observable against the actual copied bytes:
//!
//!   * stream enable (`SxCR.EN` 0→1) starts the transfer;
//!   * NDTR counts down to 0;
//!   * MINC / PINC advance the internal pointers so the destination buffer is a
//!     byte-exact copy of the source (the increment is observable in the data);
//!   * TCIF latches in LISR and LIFCR (write-1-to-clear) clears it;
//!   * the stream's own (non-contiguous) NVIC vector is pended through the bus;
//!   * memory-to-memory is DMA2-only (RM0090 §10.3.3) — the same program on
//!     DMA1 must not start.
//!
//! The byte-identical walk-vs-scheduler differential for this model (full
//! register snapshot + request stream + IRQ pend set, every cycle, under the
//! `event-scheduler` feature) lives in the model's own unit tests
//! (`peripherals::stm32f4_dma::tests::scheduler_mode`), mirroring the F1
//! `Dma1` migration. This file pins the end-to-end `from_config` wiring: base
//! addresses, the RCC clock gate, and per-stream NVIC routing.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::stm32f4_dma::StreamDma;
use labwired_core::Bus;

// STM32F407 memory map (RM0090 §2.3 / configs/chips/stm32f407.yaml).
const RCC_BASE: u64 = 0x4002_3800;
const RCC_AHB1ENR: u64 = RCC_BASE + 0x30;
const DMA1_BASE: u64 = 0x4002_6000;
const DMA2_BASE: u64 = 0x4002_6400;

// Per-stream register offsets (RM0090 §10.5).
const LISR: u64 = 0x00;
const LIFCR: u64 = 0x08;
fn s_cr(base: u64, s: u64) -> u64 {
    base + 0x10 + s * 0x18
}
fn s_ndtr(base: u64, s: u64) -> u64 {
    s_cr(base, s) + 0x04
}
fn s_par(base: u64, s: u64) -> u64 {
    s_cr(base, s) + 0x08
}
fn s_m0ar(base: u64, s: u64) -> u64 {
    s_cr(base, s) + 0x0C
}

// SxCR bits.
const CR_EN: u32 = 1 << 0;
const CR_TCIE: u32 = 1 << 4;
const DIR_M2M: u32 = 0b10 << 6;
const CR_PINC: u32 = 1 << 9;
const CR_MINC: u32 = 1 << 10;

// LISR stream-0 TCIF bit (RM0090 §10.5.1: stream-0 flags at base bit 0, TCIF +5).
const LISR_TCIF0: u32 = 1 << 5;

// DMA1EN / DMA2EN (RCC_AHB1ENR bits 21 / 22).
const DMA1EN: u32 = 1 << 21;
const DMA2EN: u32 = 1 << 22;

const SRC: u64 = 0x2000_0100;
const DST: u64 = 0x2000_0200;
const LEN: u32 = 16;

/// DMA2_Stream0 NVIC vector (stm32f407xx.h; configs/chips/stm32f407.yaml).
const DMA2_STREAM0_IRQ: u32 = 56;

fn f407_bus() -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32f407.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load stm32f407.yaml");
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "f407-dma".to_string(),
        chip: path.to_string_lossy().to_string(),
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f407 bus");
    // Pin both DMA controllers onto the per-cycle walk so `tick_peripherals_fully`
    // drives them directly. Under the `event-scheduler` feature `from_config`
    // attaches a cycle clock (flipping the model to scheduler pacing, which the
    // walk skips); detaching it makes this end-to-end test feature-agnostic. The
    // byte-identical walk-vs-scheduler equivalence is proven separately in the
    // model's own scheduler_mode unit tests.
    for name in ["dma1", "dma2"] {
        let idx = bus.find_peripheral_index_by_name(name).unwrap();
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<StreamDma>()
            .unwrap()
            .force_legacy_walk();
    }
    bus
}

/// Fill the source buffer with a recognisable ramp and clear the destination.
fn seed_buffers(bus: &mut SystemBus) {
    for i in 0..LEN as u64 {
        bus.write_u8(SRC + i, 0xA0u8.wrapping_add(i as u8)).unwrap();
        bus.write_u8(DST + i, 0x00).unwrap();
    }
}

/// Program stream 0 of `base` for an LEN-byte memory-to-memory transfer.
fn program_m2m(bus: &mut SystemBus, base: u64, extra_cr: u32) {
    bus.write_u32(s_par(base, 0), SRC as u32).unwrap(); // peripheral port = source
    bus.write_u32(s_m0ar(base, 0), DST as u32).unwrap(); // memory port = dest
    bus.write_u32(s_ndtr(base, 0), LEN).unwrap();
    bus.write_u32(
        s_cr(base, 0),
        CR_EN | DIR_M2M | CR_PINC | CR_MINC | extra_cr,
    )
    .unwrap();
}

#[test]
fn stm32f407_dma2_m2m_transfer_copies_buffer_and_latches_tcif() {
    let mut bus = f407_bus();
    // RM0090 §6.3.12: DMA is unclocked out of reset — enable RCC_AHB1ENR.DMA2EN
    // before the block responds to register writes.
    bus.write_u32(RCC_AHB1ENR, DMA1EN | DMA2EN).unwrap();
    seed_buffers(&mut bus);
    program_m2m(&mut bus, DMA2_BASE, CR_TCIE);

    // Drive the transfer to completion (one item per tick; a few extra ticks
    // are harmless once the stream goes idle).
    for _ in 0..LEN + 4 {
        bus.tick_peripherals_fully();
    }

    // NDTR counted down to zero (RM0090 §10.3.15).
    assert_eq!(bus.read_u32(s_ndtr(DMA2_BASE, 0)).unwrap(), 0, "NDTR → 0");

    // MINC/PINC increment is observable: destination is a byte-exact copy.
    for i in 0..LEN as u64 {
        assert_eq!(
            bus.read_u8(DST + i).unwrap(),
            bus.read_u8(SRC + i).unwrap(),
            "destination byte {i} must match source (PINC/MINC advance)"
        );
    }

    // User-visible SxPAR / SxM0AR stay at the programmed base (RM0090: internal
    // counters advance, not the address registers).
    assert_eq!(bus.read_u32(s_par(DMA2_BASE, 0)).unwrap(), SRC as u32);
    assert_eq!(bus.read_u32(s_m0ar(DMA2_BASE, 0)).unwrap(), DST as u32);

    // TCIF latched in LISR; LIFCR write-1-to-clear clears it (RM0090 §10.5.3).
    assert_ne!(
        bus.read_u32(DMA2_BASE + LISR).unwrap() & LISR_TCIF0,
        0,
        "TCIF must latch on completion"
    );
    bus.write_u32(DMA2_BASE + LIFCR, LISR_TCIF0).unwrap();
    assert_eq!(
        bus.read_u32(DMA2_BASE + LISR).unwrap() & LISR_TCIF0,
        0,
        "LIFCR must clear TCIF"
    );
}

#[test]
fn stm32f407_dma2_stream_irq_routes_through_nvic() {
    let mut bus = f407_bus();
    bus.write_u32(RCC_AHB1ENR, DMA2EN).unwrap();
    seed_buffers(&mut bus);
    // Two items so the completion (and its TCIE pend) lands on the last tick.
    bus.write_u32(s_par(DMA2_BASE, 0), SRC as u32).unwrap();
    bus.write_u32(s_m0ar(DMA2_BASE, 0), DST as u32).unwrap();
    bus.write_u32(s_ndtr(DMA2_BASE, 0), 2).unwrap();
    bus.write_u32(s_cr(DMA2_BASE, 0), CR_EN | DIR_M2M | CR_MINC | CR_TCIE)
        .unwrap();

    let (irqs, _) = bus.tick_peripherals_fully();
    assert!(
        !irqs.contains(&DMA2_STREAM0_IRQ),
        "no IRQ before transfer-complete"
    );
    let (irqs, _) = bus.tick_peripherals_fully();
    assert!(
        irqs.contains(&DMA2_STREAM0_IRQ),
        "TCIE must pend DMA2_Stream0 NVIC vector {DMA2_STREAM0_IRQ}"
    );
}

#[test]
fn stm32f407_dma1_memory_to_memory_is_blocked() {
    // RM0090 §10.3.3: memory-to-memory mode is available on DMA2 only.
    let mut bus = f407_bus();
    bus.write_u32(RCC_AHB1ENR, DMA1EN).unwrap();
    seed_buffers(&mut bus);
    program_m2m(&mut bus, DMA1_BASE, 0);

    for _ in 0..LEN + 4 {
        bus.tick_peripherals_fully();
    }

    // The stream never started: NDTR unchanged, destination untouched, no TCIF.
    assert_eq!(
        bus.read_u32(s_ndtr(DMA1_BASE, 0)).unwrap(),
        LEN,
        "DMA1 M2M must not start (NDTR unchanged)"
    );
    assert_eq!(
        bus.read_u8(DST).unwrap(),
        0x00,
        "DMA1 M2M must not touch memory"
    );
    assert_eq!(bus.read_u32(DMA1_BASE + LISR).unwrap() & LISR_TCIF0, 0);
}
