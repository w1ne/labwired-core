// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! RP2350 DMA paced by a peripheral DREQ (datasheet 12.6.4.1, DREQ numbers
//! from pico-sdk rp2350 `hardware/regs/dreq.h`): the pico-sdk full-duplex
//! SPI pattern — one channel feeds SSPDR on DREQ_SPI0_TX (24), another
//! drains it on DREQ_SPI0_RX (25) — must move every word, with the RP2350
//! CTRL layout (TREQ_SEL [22:17], CHAIN_TO [16:13], INCR_WRITE [6]) and the
//! RP2350 interrupt lines (DMA_IRQ_0 = NVIC 10).

use crate::common::root;

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;

const DMA: u64 = 0x5000_0000;
const SPI0: u64 = 0x4008_0000;
const SSPCR1: u64 = 0x04;
const SSPDR: u64 = 0x08;
const SSPDMACR: u64 = 0x24;
const NVIC_ISER0: u64 = 0xE000_E100;
const NVIC_ISPR0: u64 = 0xE000_E200;
const SRC: u64 = 0x2001_0000;
const DST: u64 = 0x2002_0000;
const WORDS: u32 = 320;

fn bus(chip_rel: &str, system_rel: &str) -> SystemBus {
    let chip_path = root(chip_rel);
    let chip = ChipDescriptor::from_file(&chip_path).expect("chip");
    let mut manifest = SystemManifest::from_file(root(system_rel)).expect("system");
    manifest.chip = chip_path.to_str().unwrap().to_string();
    let mut b = SystemBus::from_config(&chip, &manifest).expect("bus");
    let _ = configure_cortex_m(&mut b);
    b
}

/// RP2350 CTRL: EN, DATA_SIZE byte, INCR_READ/INCR_WRITE, CHAIN_TO self,
/// TREQ_SEL.
fn ctrl(ch: u32, incr_read: bool, incr_write: bool, treq: u32) -> u32 {
    1 | if incr_read { 1 << 4 } else { 0 }
        | if incr_write { 1 << 6 } else { 0 }
        | (ch << 13)
        | (treq << 17)
}

fn full_duplex(chip: &str, system: &str) {
    let mut b = bus(chip, system);
    // SPI0 enabled in loopback (LBM | SSE): every TX word returns on RX.
    b.write_u32(SPI0 + SSPCR1, 0b11).unwrap();
    b.write_u32(SPI0 + SSPDMACR, 0b11).unwrap();
    for i in 0..WORDS as u64 {
        b.write_u8(SRC + i, (i * 13 + 5) as u8).unwrap();
    }
    // Channel 1: SSPDR -> DST on DREQ_SPI0_RX (25).
    b.write_u32(DMA + 0x40, (SPI0 + SSPDR) as u32).unwrap();
    b.write_u32(DMA + 0x44, DST as u32).unwrap();
    b.write_u32(DMA + 0x48, WORDS).unwrap();
    b.write_u32(DMA + 0x404, 1 << 1).unwrap(); // INTE0: channel 1
    b.write_u32(NVIC_ISER0, 1 << 10).unwrap();
    b.write_u32(DMA + 0x4C, ctrl(1, false, true, 25)).unwrap();
    // Channel 0: SRC -> SSPDR on DREQ_SPI0_TX (24).
    b.write_u32(DMA, SRC as u32).unwrap();
    b.write_u32(DMA + 0x04, (SPI0 + SSPDR) as u32).unwrap();
    b.write_u32(DMA + 0x08, WORDS).unwrap();
    b.write_u32(DMA + 0x0C, ctrl(0, true, false, 24)).unwrap();
    for _ in 0..20_000 {
        let _ = b.tick_peripherals_fully();
        let busy = (b.read_u32(DMA + 0x0C).unwrap() | b.read_u32(DMA + 0x4C).unwrap()) & (1 << 26);
        if busy == 0 {
            break;
        }
    }
    let _ = b.tick_peripherals_fully();
    assert_eq!(
        b.read_u32(DMA + 0x0C).unwrap() & (1 << 26),
        0,
        "TX channel done"
    );
    assert_eq!(
        b.read_u32(DMA + 0x4C).unwrap() & (1 << 26),
        0,
        "RX channel done"
    );
    for i in 0..WORDS as u64 {
        assert_eq!(
            b.read_u8(DST + i).unwrap(),
            (i * 13 + 5) as u8,
            "word {i} lost between SPI RX and DMA"
        );
    }
    assert_eq!(
        b.read_u32(DMA + 0x400).unwrap() & 0b11,
        0b11,
        "INTR: both channels"
    );
    assert_ne!(
        b.read_u32(NVIC_ISPR0).unwrap() & (1 << 10),
        0,
        "DMA_IRQ_0 is NVIC 10 on the RP2350"
    );
}

#[test]
fn rp2350_spi_full_duplex_dma_moves_every_word() {
    full_duplex(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
}

#[test]
fn rp2354b_spi_full_duplex_dma_moves_every_word() {
    full_duplex("configs/chips/rp2354b.yaml", "configs/systems/rp2354b.yaml");
}

#[test]
fn a_dreq_aimed_at_the_wrong_peripheral_is_refused() {
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    b.write_u32(SPI0 + SSPCR1, 0b10).unwrap();
    b.write_u32(SPI0 + SSPDMACR, 0b10).unwrap();
    // DREQ_SPI1_TX (26) pacing a write into SPI0.
    b.write_u32(DMA, SRC as u32).unwrap();
    b.write_u32(DMA + 0x04, (SPI0 + SSPDR) as u32).unwrap();
    b.write_u32(DMA + 0x08, 4).unwrap();
    b.write_u32(DMA + 0x0C, ctrl(0, true, false, 26)).unwrap();
    let _ = b.tick_peripherals_fully();
    let err = b.read_u32(DMA + 0x0C).unwrap_err().to_string();
    assert!(err.contains("spi1") && err.contains("spi0"), "{err}");
}

/// DREQ_ADC (48): pico-sdk `adc_fifo_setup(true, true, 1, ...)` +
/// `adc_run(true)` with a DMA channel draining ADC FIFO into RAM, the way
/// Betaflight's RP2350 ADC driver reads its channels.
#[test]
fn adc_free_running_conversions_reach_ram_through_dreq_48() {
    const ADC: u64 = 0x400A_0000;
    const N: u32 = 16;
    let mut b = bus(
        "configs/chips/rp2350.yaml",
        "configs/systems/rp2350-zero.yaml",
    );
    // FCS: EN | DREQ_EN | THRESH 1.
    b.write_u32(ADC + 0x08, 1 | (1 << 3) | (1 << 24)).unwrap();
    // Channel 0: ADC FIFO (16-bit) -> DST, DATA_SIZE half-word.
    b.write_u32(DMA, (ADC + 0x0C) as u32).unwrap();
    b.write_u32(DMA + 0x04, DST as u32).unwrap();
    b.write_u32(DMA + 0x08, N).unwrap();
    b.write_u32(DMA + 0x0C, ctrl(0, false, true, 48) | (1 << 2))
        .unwrap();
    // CS: EN | START_MANY, input 4 (temperature sensor) with TS_EN.
    b.write_u32(ADC, 1 | (1 << 1) | (1 << 3) | (4 << 12))
        .unwrap();
    for _ in 0..200_000 {
        let _ = b.tick_peripherals_fully();
        if b.read_u32(DMA + 0x0C).unwrap() & (1 << 26) == 0 {
            break;
        }
    }
    assert_eq!(
        b.read_u32(DMA + 0x0C).unwrap() & (1 << 26),
        0,
        "ADC channel done"
    );
    for i in 0..N as u64 {
        let s = b.read_u16(DST + 2 * i).unwrap();
        assert!(
            s != 0 && s < 4096,
            "sample {i} = {s}: not a 12-bit conversion"
        );
    }
}
