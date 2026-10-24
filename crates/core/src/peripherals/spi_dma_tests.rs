// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32 SPI DMA requests (CR2.TXDMAEN / RXDMAEN, LDMA_TX / LDMA_RX) — the
//! request/acknowledge handshake on the classic and FIFO ports, and one full
//! G071 bus (SPI1 → DMAMUX → DMA1) moving a buffer both ways.

use super::{
    Spi, SpiDevice, SpiRegisterLayout, FIFO_CR2_FRXTH, FIFO_CR2_LDMA_RX, FIFO_CR2_LDMA_TX,
};
use crate::{Peripheral, DMA_SIGNAL_RX, DMA_SIGNAL_TX};

const CR1_MSTR: u16 = 1 << 2;
const CR1_SPE: u16 = 1 << 6;
const CR2_RXDMAEN: u16 = 1 << 0;
const CR2_TXDMAEN: u16 = 1 << 1;
/// CR2 reset value on the FIFO port: DS = 8 bit.
const CR2_DS8: u16 = 0x0700;

/// Answers a fixed sequence, one byte per frame.
struct Answers(std::collections::VecDeque<u8>);
impl SpiDevice for Answers {
    fn transfer(&mut self, _mosi: u8) -> u8 {
        self.0.pop_front().unwrap_or(0)
    }
    fn cs_pin(&self) -> &str {
        ""
    }
}

fn master(layout: SpiRegisterLayout, answers: &[u8]) -> Spi {
    let mut spi = Spi::new_with_layout(layout);
    spi.push_device(Box::new(Answers(answers.iter().copied().collect())));
    spi.write_u16(0x00, CR1_MSTR | CR1_SPE).unwrap();
    spi
}

fn signals(spi: &mut Spi) -> Vec<u32> {
    spi.stm32_take_dma_signals()
}

/// Clock the engine until the wire is idle; the DMA requests the walk
/// raised on the way, in order.
fn finish(spi: &mut Spi) -> Vec<u32> {
    let mut raised = Vec::new();
    for _ in 0..10_000 {
        if !spi.transfer_active() {
            return raised;
        }
        raised.extend(spi.tick_elapsed(1).dma_signals.unwrap_or_default());
    }
    panic!("SPI frame did not complete");
}

#[test]
fn no_request_line_is_raised_without_its_enable_bit() {
    let mut spi = master(SpiRegisterLayout::Stm32Fifo, &[]);
    assert!(signals(&mut spi).is_empty(), "TXE is set, but TXDMAEN is 0");
    assert!(spi.tick_elapsed(1).dma_signals.is_none());
}

#[test]
fn fifo_tx_request_is_held_until_the_dr_write_and_raised_while_the_fifo_has_room() {
    let mut spi = master(SpiRegisterLayout::Stm32Fifo, &[]);
    spi.write_u16(0x04, CR2_DS8 | FIFO_CR2_FRXTH | CR2_TXDMAEN)
        .unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX], "TXE: one request");
    assert!(
        signals(&mut spi).is_empty(),
        "outstanding until the DMA writes DR: not raised twice"
    );
    // The DMA's item: the first frame goes to the shifter, the FIFO is empty.
    spi.write(0x0C, 0x11).unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX]);
    spi.write(0x0C, 0x22).unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX], "1 byte queued");
    spi.write(0x0C, 0x33).unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX], "2 bytes: half full");
    spi.write(0x0C, 0x44).unwrap();
    assert!(
        signals(&mut spi).is_empty(),
        "3 bytes: above half full, TXE is clear"
    );
    // Clearing TXDMAEN drops the line.
    spi.write_u16(0x04, CR2_DS8 | FIFO_CR2_FRXTH).unwrap();
    assert!(finish(&mut spi).is_empty());
    assert!(signals(&mut spi).is_empty());
}

#[test]
fn a_disabled_master_raises_no_tx_request() {
    let mut spi = Spi::new_with_layout(SpiRegisterLayout::Stm32Fifo);
    spi.write_u16(0x00, CR1_MSTR).unwrap();
    spi.write_u16(0x04, CR2_DS8 | CR2_TXDMAEN).unwrap();
    assert!(signals(&mut spi).is_empty(), "SPE=0");
    spi.write_u16(0x00, CR1_MSTR | CR1_SPE).unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX]);
}

#[test]
fn fifo_rx_request_follows_rxne_and_is_answered_by_a_dr_read() {
    let mut spi = master(SpiRegisterLayout::Stm32Fifo, &[0xA1, 0xA2]);
    spi.write_u16(0x04, CR2_DS8 | FIFO_CR2_FRXTH | CR2_RXDMAEN)
        .unwrap();
    assert!(signals(&mut spi).is_empty(), "RX FIFO empty");
    spi.write(0x0C, 0x01).unwrap();
    spi.write(0x0C, 0x02).unwrap();
    // Frame 1 lands: RXNE (FRXTH=1) raises the request; the walk reports it.
    assert_eq!(
        finish(&mut spi),
        vec![DMA_SIGNAL_RX],
        "raised once, at the first frame"
    );
    assert!(
        signals(&mut spi).is_empty(),
        "two frames in the FIFO, but the request is still outstanding"
    );
    assert_eq!(spi.read(0x0C).unwrap(), 0xA1, "the DMA's read");
    assert_eq!(
        signals(&mut spi),
        vec![DMA_SIGNAL_RX],
        "RXNE still set: raised again"
    );
    assert_eq!(spi.read(0x0C).unwrap(), 0xA2);
    assert!(signals(&mut spi).is_empty(), "RX FIFO empty");
}

#[test]
fn classic_port_requests_once_per_frame() {
    let mut spi = master(SpiRegisterLayout::Stm32, &[0x5A]);
    spi.write_u16(0x04, CR2_TXDMAEN | CR2_RXDMAEN).unwrap();
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_TX], "TXE");
    spi.write(0x0C, 0xC3).unwrap();
    assert!(signals(&mut spi).is_empty(), "frame on the wire: TXE clear");
    // The frame is done: RXNE and TXE both request, RX first.
    assert_eq!(finish(&mut spi), vec![DMA_SIGNAL_RX, DMA_SIGNAL_TX]);
    assert_eq!(spi.read(0x0C).unwrap(), 0x5A);
    assert!(
        signals(&mut spi).is_empty(),
        "both still outstanding / empty"
    );
}

#[test]
fn ldma_tx_sends_one_frame_from_the_last_packed_dma_item() {
    let mut spi = master(SpiRegisterLayout::Stm32Fifo, &[]);
    spi.write_u16(0x04, CR2_DS8 | CR2_TXDMAEN | FIFO_CR2_LDMA_TX)
        .unwrap();
    // Two packed items of an odd (3-byte) transfer: NDTR 2 → 1 → 0.
    spi.dma_access_notice(0x0C, false, 1);
    spi.write_u16(0x0C, 0x2211).unwrap();
    assert_eq!(spi.fifo_tx_bytes(), 1, "two frames, one already shifting");
    spi.dma_access_notice(0x0C, false, 0);
    spi.write_u16(0x0C, 0xEE33).unwrap();
    assert_eq!(
        spi.fifo_tx_bytes(),
        2,
        "the last item carries one frame: 0xEE is not data"
    );
    // Without LDMA_TX the same item packs two.
    let mut plain = master(SpiRegisterLayout::Stm32Fifo, &[]);
    plain.write_u16(0x04, CR2_DS8 | CR2_TXDMAEN).unwrap();
    plain.dma_access_notice(0x0C, false, 0);
    plain.write_u16(0x0C, 0xEE33).unwrap();
    assert_eq!(plain.fifo_tx_bytes(), 1, "two frames, one shifting");
}

#[test]
fn ldma_rx_requests_the_odd_last_byte_below_the_16_bit_threshold() {
    let mut spi = master(SpiRegisterLayout::Stm32Fifo, &[0x10, 0x20, 0x30]);
    // FRXTH = 0: RXNE wants 16 bits; halfword DMA reads two frames a time.
    spi.write_u16(0x04, CR2_DS8 | CR2_RXDMAEN | FIFO_CR2_LDMA_RX)
        .unwrap();
    let mut raised = Vec::new();
    for b in [1, 2, 3] {
        spi.write(0x0C, b).unwrap();
        raised.extend(finish(&mut spi));
    }
    // Two bytes reached the 16-bit threshold: one request (still
    // outstanding at the third frame).
    assert_eq!(raised, vec![DMA_SIGNAL_RX]);
    // Item 1 of 2 (NDTR 2 → 1): the next one is the last.
    spi.dma_access_notice(0x0C, true, 1);
    assert_eq!(spi.read_u16(0x0C).unwrap(), 0x2010);
    // One byte left, below the threshold: requested anyway (LDMA_RX).
    assert_eq!(signals(&mut spi), vec![DMA_SIGNAL_RX]);
    spi.dma_access_notice(0x0C, true, 0);
    assert_eq!(spi.read_u16(0x0C).unwrap(), 0x0030);
    assert!(signals(&mut spi).is_empty());

    // Without LDMA_RX the odd byte never reaches the threshold.
    let mut plain = master(SpiRegisterLayout::Stm32Fifo, &[0x10]);
    plain.write_u16(0x04, CR2_DS8 | CR2_RXDMAEN).unwrap();
    plain.write(0x0C, 1).unwrap();
    assert!(finish(&mut plain).is_empty());
    plain.dma_access_notice(0x0C, true, 1);
    assert!(signals(&mut plain).is_empty());
}

/// The whole path on a G071 bus built from its chip yaml: SPI1 (an echo
/// device, so MISO echoes MOSI) raises its requests, DMAMUX routes them (C0CR =
/// SPI1_RX → DMA1 channel 1, C1CR = SPI1_TX → channel 2), DMA1 moves one
/// byte per request, and the receive buffer ends up holding the transmit
/// buffer — on the legacy walk.
#[test]
fn g071_spi1_moves_a_buffer_through_dmamux_and_dma1() {
    use crate::bus::SystemBus;
    use crate::Bus;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("configs/chips/stm32g071.yaml");
    let chip = labwired_config::ChipDescriptor::from_file(&path).unwrap();
    let manifest = labwired_config::SystemManifest {
        parts: Vec::new(),
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "spi-dma".to_string(),
        chip: path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    // A device answering each byte with itself: MISO echoes MOSI.
    struct Echo;
    impl SpiDevice for Echo {
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi
        }
        fn cs_pin(&self) -> &str {
            ""
        }
    }
    bus.attach_spi_device("spi1", Box::new(Echo)).unwrap();
    // Clocks: RCC_AHBENR.DMA1EN, RCC_APBENR2.SPI1EN.
    bus.write_u32(0x4002_1038, 1).unwrap();
    bus.write_u32(0x4002_1040, 1 << 12).unwrap();
    let tx = [0xA5u8, 0x3C, 0x5A, 0xC3, 0x0F];
    for (i, b) in tx.iter().enumerate() {
        bus.write_u8(0x2000_0200 + i as u64, *b).unwrap();
    }
    const SPI1_DR: u32 = 0x4001_300C;
    // DMAMUX: channel 0 ← SPI1_RX (16), channel 1 ← SPI1_TX (17).
    bus.write_u32(0x4002_0800, 16).unwrap();
    bus.write_u32(0x4002_0804, 17).unwrap();
    // DMA1 ch1 (CCR1 0x08): SPI1 DR → RAM, MINC. ch2 (CCR2 0x1C): RAM → DR,
    // DIR | MINC.
    bus.write_u32(0x4002_000C, tx.len() as u32).unwrap();
    bus.write_u32(0x4002_0010, SPI1_DR).unwrap();
    bus.write_u32(0x4002_0014, 0x2000_0300).unwrap();
    bus.write_u32(0x4002_0008, (1 << 7) | 1).unwrap();
    bus.write_u32(0x4002_0020, tx.len() as u32).unwrap();
    bus.write_u32(0x4002_0024, SPI1_DR).unwrap();
    bus.write_u32(0x4002_0028, 0x2000_0200).unwrap();
    bus.write_u32(0x4002_001C, (1 << 7) | (1 << 4) | 1).unwrap();
    // SPI1: master, BR = /8, FRXTH, RXDMAEN, SPE, then TXDMAEN (HAL order).
    bus.write_u32(0x4001_3004, 0x0700 | (1 << 12) | 1).unwrap();
    bus.write_u32(
        0x4001_3000,
        (2 << 3) | (1 << 2) | (1 << 8) | (1 << 9) | (1 << 6),
    )
    .unwrap();
    bus.write_u32(0x4001_3004, 0x0700 | (1 << 12) | 3).unwrap();
    for _ in 0..2_000 {
        bus.tick_peripherals_fully_forced();
    }
    let mut rx = [0u8; 5];
    for (i, b) in rx.iter_mut().enumerate() {
        *b = bus.read_u8(0x2000_0300 + i as u64).unwrap();
    }
    assert_eq!(rx, tx, "the loopback echo, item by item");
    // CNDTR 0 on both channels, TCIF1 | TCIF2 latched.
    assert_eq!(bus.read_u32(0x4002_000C).unwrap(), 0);
    assert_eq!(bus.read_u32(0x4002_0020).unwrap(), 0);
    let isr = bus.read_u32(0x4002_0000).unwrap();
    assert_ne!(isr & (1 << 1), 0, "TCIF1");
    assert_ne!(isr & (1 << 5), 0, "TCIF2");
}
