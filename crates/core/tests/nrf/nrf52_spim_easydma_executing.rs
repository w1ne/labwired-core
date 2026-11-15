// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Executing EasyDMA oracle for the nRF52840 SPIM (`peripherals/spi.rs`,
//! `SpiRegisterLayout::Nrf52Spim`).
//!
//! The shipped nRF52 SPIM/TWIM EasyDMA conformance suites are hardware-oracle
//! (SWD) and register-surface diffs — they do not observe DMA byte movement in
//! the sim. The model IS fully executing (it reads the TX buffer from RAM and
//! writes the RX buffer back to RAM inside `tick_with_bus`), so this adds a
//! ratchet-discoverable integration test that proves REAL data movement:
//! program `TXD.PTR/MAXCNT` + `RXD.PTR/MAXCNT`, trigger `TASKS_START`, run the
//! DMA on a real `SystemBus`-backed RAM region, and assert the destination RAM
//! bytes — plus the `AMOUNT` counters and `EVENTS_END*` — reflect the transfer.
//!
//! Named `nrf52_*` so the board-coverage ratchet discovers it.

use labwired_core::bus::SystemBus;
use labwired_core::peripherals::spi::{Spi, SpiRegisterLayout};
use labwired_core::system::xtensa::RamPeripheral;
use labwired_core::{Bus, Peripheral};

// nRF52 SPIM register offsets (base-relative).
const TASKS_START: u64 = 0x010;
const EVENTS_ENDRX: u64 = 0x110;
const EVENTS_END: u64 = 0x118;
const EVENTS_ENDTX: u64 = 0x120;
const ENABLE: u64 = 0x500;
const RXD_PTR: u64 = 0x534;
const RXD_MAXCNT: u64 = 0x538;
const RXD_AMOUNT: u64 = 0x53C;
const TXD_PTR: u64 = 0x544;
const TXD_MAXCNT: u64 = 0x548;
const TXD_AMOUNT: u64 = 0x54C;

const ENABLE_SPIM: u32 = 7;

// nRF52 RAM window.
const RAM_BASE: u64 = 0x2000_0000;
const RAM_SIZE: usize = 64 * 1024;

fn bus_with_ram() -> SystemBus {
    let mut bus = SystemBus::new();
    bus.add_peripheral(
        "ram",
        RAM_BASE,
        RAM_SIZE as u64,
        None,
        Box::new(RamPeripheral::new(RAM_SIZE)),
    );
    bus
}

/// EasyDMA with loopback: the RX buffer in RAM must end up holding exactly the
/// bytes that were fetched from the TX buffer, and the completion events /
/// AMOUNT counters must reflect the full-length transfer.
#[test]
fn nrf52_spim_easydma_loopback_moves_bytes_through_ram() {
    let mut bus = bus_with_ram();
    let mut spim = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spim.set_loopback(true); // MISO mirrors MOSI, so RX == TX deterministically.

    let tx_ptr = RAM_BASE;
    let rx_ptr = RAM_BASE + 0x100;
    let tx_data: [u8; 6] = [0xDE, 0xAD, 0xBE, 0xEF, 0xA5, 0x5A];
    for (i, &b) in tx_data.iter().enumerate() {
        bus.write_u8(tx_ptr + i as u64, b).unwrap();
        // Poison the RX region so a no-op cannot pass.
        bus.write_u8(rx_ptr + i as u64, 0x00).unwrap();
    }

    spim.write_u32(ENABLE, ENABLE_SPIM).unwrap();
    spim.write_u32(TXD_PTR, tx_ptr as u32).unwrap();
    spim.write_u32(TXD_MAXCNT, tx_data.len() as u32).unwrap();
    spim.write_u32(RXD_PTR, rx_ptr as u32).unwrap();
    spim.write_u32(RXD_MAXCNT, tx_data.len() as u32).unwrap();

    spim.write_u32(TASKS_START, 1).unwrap();
    assert_eq!(
        spim.read_u32(EVENTS_END).unwrap(),
        0,
        "EVENTS_END must not fire before the DMA runs"
    );
    assert!(spim.needs_bus_tick(), "START must arm a pending bus tick");

    // Run the EasyDMA engine (SPIM completes in a single bus tick).
    spim.tick_with_bus(&mut bus);

    assert_eq!(spim.read_u32(EVENTS_END).unwrap(), 1, "EVENTS_END");
    assert_eq!(spim.read_u32(EVENTS_ENDTX).unwrap(), 1, "EVENTS_ENDTX");
    assert_eq!(spim.read_u32(EVENTS_ENDRX).unwrap(), 1, "EVENTS_ENDRX");
    assert_eq!(spim.read_u32(TXD_AMOUNT).unwrap(), tx_data.len() as u32);
    assert_eq!(spim.read_u32(RXD_AMOUNT).unwrap(), tx_data.len() as u32);
    assert!(
        !spim.needs_bus_tick(),
        "pending START must clear after the DMA"
    );

    let rx: Vec<u8> = (0..tx_data.len())
        .map(|i| bus.read_u8(rx_ptr + i as u64).unwrap())
        .collect();
    assert_eq!(
        rx, tx_data,
        "loopback EasyDMA must land the TX bytes verbatim in the RX buffer"
    );
}

/// RXD.MAXCNT shorter than TXD.MAXCNT bounds how many bytes are written back to
/// RAM: the RX buffer past MAXCNT stays untouched and RXD.AMOUNT reflects the
/// clamp, while TXD.AMOUNT still reports the full clocked length.
#[test]
fn nrf52_spim_easydma_rxd_maxcnt_clamps_written_bytes() {
    let mut bus = bus_with_ram();
    let mut spim = Spi::new_with_layout(SpiRegisterLayout::Nrf52Spim);
    spim.set_loopback(true);

    let tx_ptr = RAM_BASE;
    let rx_ptr = RAM_BASE + 0x100;
    let tx_data: [u8; 4] = [0x11, 0x22, 0x33, 0x44];
    for (i, &b) in tx_data.iter().enumerate() {
        bus.write_u8(tx_ptr + i as u64, b).unwrap();
    }
    // Sentinel across the whole RX region.
    for i in 0..tx_data.len() {
        bus.write_u8(rx_ptr + i as u64, 0x99).unwrap();
    }

    spim.write_u32(ENABLE, ENABLE_SPIM).unwrap();
    spim.write_u32(TXD_PTR, tx_ptr as u32).unwrap();
    spim.write_u32(TXD_MAXCNT, 4).unwrap();
    spim.write_u32(RXD_PTR, rx_ptr as u32).unwrap();
    spim.write_u32(RXD_MAXCNT, 2).unwrap(); // only 2 bytes captured

    spim.write_u32(TASKS_START, 1).unwrap();
    spim.tick_with_bus(&mut bus);

    assert_eq!(spim.read_u32(TXD_AMOUNT).unwrap(), 4, "all 4 bytes clocked");
    assert_eq!(
        spim.read_u32(RXD_AMOUNT).unwrap(),
        2,
        "only 2 bytes captured"
    );
    assert_eq!(bus.read_u8(rx_ptr).unwrap(), 0x11);
    assert_eq!(bus.read_u8(rx_ptr + 1).unwrap(), 0x22);
    assert_eq!(
        bus.read_u8(rx_ptr + 2).unwrap(),
        0x99,
        "RX bytes past RXD.MAXCNT must remain untouched"
    );
    assert_eq!(bus.read_u8(rx_ptr + 3).unwrap(), 0x99);
}
