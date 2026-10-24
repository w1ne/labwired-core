// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32G071 DMAMUX request generators and request synchronisation (RM0444
//! DMAMUX chapter) end to end on the bus the shipped
//! `configs/chips/stm32g071.yaml` builds: a pad edge on PA0 / PA1 reaches the
//! DMAMUX as EXTI line 0 / 1 (port selected by `EXTI_EXTICR1`), a request
//! generator turns one edge into `GNBREQ + 1` DMA requests, synchronisation
//! lets `NBREQ + 1` requests through per edge, and DMA1 channel 1 moves one
//! word to `GPIOB_ODR` per request — the waveform-on-a-trigger use the
//! generators exist for.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;

const RCC_IOPENR: u64 = 0x4002_1034;
const RCC_AHBENR: u64 = 0x4002_1038;
const GPIOA_MODER: u64 = 0x5000_0000;
const GPIOB_ODR: u64 = 0x5000_0414;
const DMA1: u64 = 0x4002_0000;
const DMA1_CCR1: u64 = DMA1 + 0x08;
const DMA1_CNDTR1: u64 = DMA1 + 0x0C;
const DMA1_CPAR1: u64 = DMA1 + 0x10;
const DMA1_CMAR1: u64 = DMA1 + 0x14;
const DMAMUX: u64 = 0x4002_0800;
const DMAMUX_C0CR: u64 = DMAMUX;
const DMAMUX_RG0CR: u64 = DMAMUX + 0x100;
const DMAMUX_CSR: u64 = DMAMUX + 0x80;
const DMAMUX_RGSR: u64 = DMAMUX + 0x140;
const TABLE: u64 = 0x2000_0400;

fn g071_bus() -> SystemBus {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips/stm32g071.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load stm32g071.yaml");
    let manifest = SystemManifest {
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "g071-dmamux".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build g071 bus");
    bus.write_u32(RCC_IOPENR, 0b11).unwrap(); // GPIOA, GPIOB
    bus.write_u32(RCC_AHBENR, 1).unwrap(); // DMA1 + DMAMUX
    bus.write_u32(GPIOA_MODER, 0xEBFF_FFF0).unwrap(); // PA0, PA1 inputs
    for i in 0..8u64 {
        bus.write_u32(TABLE + 4 * i, 0x100 + i as u32).unwrap();
    }
    // DMA1 channel 1: TABLE -> GPIOB_ODR, words, 8 items, memory increment.
    bus.write_u32(DMA1_CPAR1, GPIOB_ODR as u32).unwrap();
    bus.write_u32(DMA1_CMAR1, TABLE as u32).unwrap();
    bus.write_u32(DMA1_CNDTR1, 8).unwrap();
    // EN | DIR | MINC | PSIZE = 32 | MSIZE = 32.
    bus.write_u32(DMA1_CCR1, 1 | (1 << 4) | (1 << 7) | (2 << 8) | (2 << 10))
        .unwrap();
    bus
}

/// Drive pad `pin` of GPIOA to `level`.
fn pad(bus: &mut SystemBus, pin: u8, level: bool) {
    let idx = bus.find_peripheral_index_by_name("gpioa").unwrap();
    bus.set_peripheral_gpio_input(idx, pin, level);
}

fn run(bus: &mut SystemBus, cycles: usize) {
    for _ in 0..cycles {
        bus.tick_peripherals_fully_forced();
    }
}

#[test]
fn an_exti_edge_makes_a_generator_issue_gnbreq_plus_one_dma_requests() {
    let mut bus = g071_bus();
    // C0CR: DMAREQ_ID 1 = dmamux_req_gen0 -> DMA1 channel 1.
    bus.write_u32(DMAMUX_C0CR, 1).unwrap();
    // RG0CR: SIG_ID 0 (EXTI line 0), GPOL rising, GNBREQ 3, GE.
    bus.write_u32(DMAMUX_RG0CR, (3 << 19) | (1 << 17) | (1 << 16))
        .unwrap();
    run(&mut bus, 10);
    assert_eq!(bus.read_u32(DMA1_CNDTR1).unwrap(), 8, "no trigger yet");

    pad(&mut bus, 0, true);
    run(&mut bus, 10);
    assert_eq!(
        bus.read_u32(DMA1_CNDTR1).unwrap(),
        4,
        "one edge, 4 requests"
    );
    assert_eq!(bus.read_u32(GPIOB_ODR).unwrap() & 0xFFFF, 0x103);
    pad(&mut bus, 0, false); // falling: not selected
    run(&mut bus, 10);
    assert_eq!(bus.read_u32(DMA1_CNDTR1).unwrap(), 4);
    pad(&mut bus, 0, true);
    run(&mut bus, 10);
    assert_eq!(bus.read_u32(DMA1_CNDTR1).unwrap(), 0);
    assert_eq!(bus.read_u32(GPIOB_ODR).unwrap() & 0xFFFF, 0x107);
    assert_eq!(bus.read_u32(DMA1).unwrap() & 0b10, 0b10, "TCIF1");
    assert_eq!(bus.read_u32(DMAMUX_RGSR).unwrap(), 0, "no overrun");
}

#[test]
fn synchronisation_lets_nbreq_plus_one_requests_through_per_edge() {
    let mut bus = g071_bus();
    // Generator 0 on EXTI line 0 issues 6 requests; channel 0 takes them but
    // forwards only 2 per rising edge of EXTI line 1 (SE, SYNC_ID 1, SPOL
    // rising, NBREQ 1).
    bus.write_u32(DMAMUX_RG0CR, (5 << 19) | (1 << 17) | (1 << 16))
        .unwrap();
    bus.write_u32(
        DMAMUX_C0CR,
        1 | (1 << 16) | (1 << 17) | (1 << 19) | (1 << 24),
    )
    .unwrap();
    pad(&mut bus, 0, true);
    run(&mut bus, 20);
    assert_eq!(bus.read_u32(DMA1_CNDTR1).unwrap(), 8, "held: no sync edge");
    for want in [6, 4, 2] {
        pad(&mut bus, 1, true);
        run(&mut bus, 10);
        assert_eq!(bus.read_u32(DMA1_CNDTR1).unwrap(), want);
        pad(&mut bus, 1, false);
        run(&mut bus, 2);
    }
    assert_eq!(bus.read_u32(GPIOB_ODR).unwrap() & 0xFFFF, 0x105);
    assert_eq!(bus.read_u32(DMAMUX_CSR).unwrap(), 0, "no sync overrun");
    // A sync edge with nothing held opens a window nobody uses; the next
    // edge overruns it (SOF0).
    pad(&mut bus, 1, true);
    run(&mut bus, 2);
    pad(&mut bus, 1, false);
    pad(&mut bus, 1, true);
    run(&mut bus, 2);
    assert_eq!(bus.read_u32(DMAMUX_CSR).unwrap(), 1, "SOF0");
}
