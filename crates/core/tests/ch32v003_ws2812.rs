// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! CH32V003 WS2812 over SPI1 + DMA1 on the COMMITTED ch32fun-smoke chip
//! profile, the path ch32fun's ws2812b_dma_spi_led_driver.h takes:
//!
//! * SPI1 MOSI is published on PC6 (`pad_map: ch32v003`), and a GPIO
//!   observer on `gpioc` hears its edges at the cycles the bit engine
//!   stamps them (the pad-level half is `gpio.rs`'s
//!   `wch_pc6_observer_hears_spi_mosi_only_in_af_mode`).
//! * DMA1 channel 3 serves SPI1 TX (`spi1_tx: 3`). The driver enables the
//!   channel once with CNTR = 0 and starts every frame by storing CNTR with
//!   EN still set, so that store has to re-arm the channel and pick up the
//!   SPI's standing TXE request.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::{system::riscv::configure_riscv, Bus};
use std::path::PathBuf;

const RCC_AHBPCENR: u64 = 0x4002_1014;
const RCC_APB2PCENR: u64 = 0x4002_1018;
const SPI1: u64 = 0x4001_3000;
const SPI1_CTLR1: u64 = SPI1;
const SPI1_CTLR2: u64 = SPI1 + 0x04;
const SPI1_DATAR: u64 = SPI1 + 0x0C;
const DMA1: u64 = 0x4002_0000;
const CH3_CFGR: u64 = DMA1 + 0x08 + 2 * 20;
const CH3_CNTR: u64 = CH3_CFGR + 0x04;
const CH3_PADDR: u64 = CH3_CFGR + 0x08;
const CH3_MADDR: u64 = CH3_CFGR + 0x0C;
/// WS2812BDMAInit's CFGR: PL very high, 16-bit both sides, MINC, memory to
/// peripheral, TCIE | HTIE.
const CFGR_WS: u32 = (3 << 12) | (1 << 10) | (1 << 8) | (1 << 7) | (1 << 4) | (1 << 2) | (1 << 1);
/// WS2812BDMAInit's CTLR1: SSM | SSI, 16-bit, master, 1-line TX
/// (BIDIMODE | BIDIOE), BR = 3.
const CTLR1_WS: u32 = (1 << 9) | (1 << 8) | (1 << 11) | (1 << 2) | (1 << 15) | (1 << 14) | (3 << 3);

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn chip() -> ChipDescriptor {
    ChipDescriptor::from_file(root().join("configs/chips/ch32v003f4p6-ch32fun-smoke.yaml")).unwrap()
}

fn manifest() -> SystemManifest {
    serde_yaml::from_str("name: ch32\nchip: unused\nexternal_devices: []\n").unwrap()
}

fn bus() -> SystemBus {
    let mut bus = SystemBus::from_config(&chip(), &manifest()).unwrap();
    let _cpu = configure_riscv(&mut bus);
    bus.write_u32(RCC_AHBPCENR, 1).unwrap(); // DMA1EN
    bus.write_u32(RCC_APB2PCENR, (1 << 12) | (1 << 4)).unwrap(); // SPI1EN, IOPCEN
    bus
}

/// Serve what channel 3 has latched, one bus copy and commit per item, the
/// way the bus does; with `paced`, raise channel 3's request again after
/// each item (SPI1 TXE coming back as the frame shifts out, which this
/// test does not clock). Stops after `max` items; returns the items moved.
fn serve_ch3(bus: &mut SystemBus, max: u32, paced: bool) -> u32 {
    let dma = bus.find_peripheral_index_by_name("dma1").unwrap();
    let mut moved = 0;
    for _ in 0..max * 4 {
        if moved == max {
            break;
        }
        let res = bus.peripherals[dma].dev.tick_elapsed_forced(1);
        for req in res.dma_requests.unwrap_or_default() {
            bus.execute_dma(std::slice::from_ref(&req)).unwrap();
            bus.peripherals[dma].dev.dma_transfer_complete(&req);
            moved += 1;
            if paced {
                bus.peripherals[dma].dev.dma_request(3);
            }
        }
    }
    moved
}

/// WS2812BDMAInit, register for register: SPI1 TX with TXDMAEN, then
/// channel 3 enabled with nothing to move.
fn ws2812_init(bus: &mut SystemBus) {
    bus.write_u16(SPI1_CTLR1, CTLR1_WS as u16).unwrap();
    bus.write_u16(SPI1_CTLR2, 1 << 1).unwrap(); // TXDMAEN
    bus.write_u16(SPI1_CTLR1, (CTLR1_WS | (1 << 6)) as u16)
        .unwrap(); // SPE
    bus.write_u32(CH3_PADDR, SPI1_DATAR as u32).unwrap();
    bus.write_u32(CH3_MADDR, 0x2000_0100).unwrap();
    bus.write_u32(CH3_CNTR, 0).unwrap();
    bus.write_u32(CH3_CFGR, CFGR_WS).unwrap();
    bus.write_u32(CH3_CFGR, CFGR_WS | 1).unwrap();
}

#[test]
fn cntr_store_on_enabled_channel_3_rearms_it_for_a_circular_frame() {
    let mut b = bus();
    ws2812_init(&mut b);
    assert_eq!(serve_ch3(&mut b, 4, false), 0, "CNTR = 0: nothing to move");
    // WS2812BDMAStart: CNTR = 48 then CIRC, EN never dropped.
    b.write_u32(CH3_CNTR, 48).unwrap();
    b.write_u32(CH3_CFGR, CFGR_WS | (1 << 5) | 1).unwrap();
    assert_eq!(
        serve_ch3(&mut b, 1, false),
        1,
        "the store picks up SPI1's standing TXE & TXDMAEN request"
    );
    // TXE comes back once the frame moves into the shift register.
    let dma = b.find_peripheral_index_by_name("dma1").unwrap();
    b.peripherals[dma].dev.dma_request(3);
    assert_eq!(serve_ch3(&mut b, 23, true), 23);
    assert_eq!(b.read_u32(CH3_CNTR).unwrap(), 24);
    assert_eq!(
        b.read_u32(DMA1).unwrap() & 0xF00,
        0x500,
        "HTIF3 | GIF3 at half of the CNTR the store loaded"
    );
    assert_eq!(serve_ch3(&mut b, 24, true), 24);
    assert_eq!(
        b.read_u32(CH3_CNTR).unwrap(),
        48,
        "TC with CIRC reloads the CNTR the store loaded"
    );
    assert_eq!(b.read_u32(DMA1).unwrap() & 0xF00, 0x700, "TCIF3 too");
}

#[test]
fn cntr_store_of_zero_on_an_enabled_channel_moves_nothing() {
    let mut b = bus();
    ws2812_init(&mut b);
    b.write_u32(CH3_CNTR, 0).unwrap();
    assert_eq!(serve_ch3(&mut b, 4, false), 0);
}

/// The committed ws2812bdemo.elf on a Machine: the strip on PC6 decodes the
/// LEDs the driver clocks out. 190 of NR_LEDS = 191 reach the wire (see
/// check-ws2812.py's replay of the driver's buffer protocol); LED 0 of
/// frame 1 is the demo's "fire" colour for phase 0x0100 + one step.
///
/// The edge times come from the SPI bit engine's cycle stamps, which only
/// the event-scheduler path has. On the legacy walk the strip must stay
/// empty: an edge with no known cycle is not reported, rather than decoded
/// at a made-up time.
#[test]
fn ch32fun_ws2812bdemo_lights_190_leds_on_pc6() {
    use labwired_core::Machine;
    let mut bus = SystemBus::from_config(&chip(), &manifest()).unwrap();
    let strip = std::sync::Arc::new(labwired_core::peripherals::components::ws2812(
        6, 191, 48_000_000,
    ));
    assert!(SystemBus::install_port_gpio_observer(
        &mut bus,
        "gpioc",
        strip.clone()
    ));
    let image =
        labwired_loader::load_elf(&root().join("examples/ch32v003-evt/ch32fun/ws2812bdemo.elf"))
            .expect("committed ch32fun ws2812bdemo ELF");
    let cpu = configure_riscv(&mut bus);
    let mut machine = Machine::new(cpu, bus);
    machine.load_firmware(&image).unwrap();
    // Frame 1 goes out from ~12 ms and frame 2's first edge latches it at
    // ~25 ms; before that `pixels()` shows the frame still arriving.
    while machine.total_cycles < 1_400_000 {
        labwired_core::DebugControl::run(&mut machine, Some(10_000)).expect("ws2812bdemo runs");
    }
    let lit = strip.pixels().len();
    if !labwired_core::cycle_clock::event_scheduler_enabled() {
        assert_eq!(lit, 0, "legacy walk: no stamped edges, no LEDs");
        return;
    }
    assert_eq!(lit, 190, "LEDs latched off PC6");
    let px = strip.pixels();
    // Frame 1, LED 0 (WSGRB wire order), from the demo's tables.
    assert_eq!(px[0].wire, [48, 255, 0]);
}
