// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// ESP32 (classic, Xtensa LX6 dual-core) modeling smoke tests.
//
// These verify three things:
//   1. The chip yaml `configs/chips/esp32.yaml` loads cleanly.
//   2. The system builder `configure_xtensa_esp32` registers every
//      declared peripheral (IRAM, DRAM, ROM, flash I-cache, flash
//      D-cache, UART0) at the documented addresses.
//   3. Writes to UART0's DR (STM32F1 layout offset 0x04 on top of the
//      ESP32 UART0 base 0x3FF4_0000) propagate to the bus's TX sink —
//      the same UART pipe every other LabWired chip uses.
//
// A full Xtensa LX6 firmware demo (hand-rolled vector table + UART
// init in `.S`) is the follow-up; for the half-day modeling slice
// the goal is just to prove the simulator's memory map and UART path
// match real ESP32 silicon's documented layout, with the chip yaml
// + system builder as the contract.

use crate::bus::SystemBus;
use crate::system::xtensa::configure_xtensa_esp32;
use crate::Bus;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[test]
fn esp32_chip_yaml_loads() {
    let mut chip_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    chip_path.push("../../configs/chips/esp32.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)
        .unwrap_or_else(|_| panic!("Failed to load chip config at {:?}", chip_path));
    assert_eq!(chip.name, "esp32");
    // The Arch enum collapses both LX6 and LX7 to `Xtensa` per
    // labwired_config::lib's `FromStr` map (XTENSA/LX7/LX6 → Xtensa).
    assert!(matches!(chip.arch, labwired_config::Arch::Xtensa));
}

#[test]
fn esp32_wroom_system_yaml_loads() {
    let mut system_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    system_path.push("../../configs/systems/esp32-wroom-32.yaml");
    let manifest = SystemManifest::from_file(&system_path)
        .unwrap_or_else(|_| panic!("Failed to load system manifest at {:?}", system_path));
    assert_eq!(manifest.name, "esp32-wroom-32");
}

#[test]
fn esp32_system_builder_wires_documented_regions() {
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    // Every region declared in esp32.yaml must respond to at least one
    // read.  Bus::read_u8 returns Ok for any address inside a registered
    // peripheral and an error for unmapped addresses.
    for (name, addr) in [
        ("IRAM", 0x4008_0000),
        ("DRAM", 0x3FFB_0000),
        ("ROM", 0x4000_0000),
        ("flash_icache", 0x400D_0000),
        ("flash_dcache", 0x3F40_0000),
        ("UART0", 0x3FF4_0000),
    ] {
        bus.read_u8(addr)
            .unwrap_or_else(|e| panic!("{name} @ 0x{addr:08X} unreachable: {e:?}"));
    }
}

#[test]
fn esp32_uart0_emits_to_sink() {
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // UART0 base 0x3FF4_0000, real ESP32 layout: TX is the FIFO at offset 0x00.
    // Bytes shift out at the baud rate. Drain by ticking the uart0 peripheral
    // directly — independent of the bus's scheduler cadence (uart0 is
    // event-scheduler-managed, so a tight tick_peripherals loop wouldn't
    // reliably advance it across build configs).
    for &b in b"ESP32" {
        bus.write_u8(0x3FF4_0000, b).unwrap();
    }
    let uart0_idx = bus
        .find_peripheral_index_by_name("uart0")
        .expect("uart0 mapped");
    for _ in 0..2_000_000 {
        let _ = bus.peripherals[uart0_idx].dev.tick();
        if sink.lock().unwrap().len() >= 5 {
            break;
        }
    }

    let bytes = sink.lock().unwrap();
    assert_eq!(
        bytes.as_slice(),
        b"ESP32",
        "UART0 sink should have received 'ESP32', got {:?}",
        std::str::from_utf8(&bytes).unwrap_or("<non-utf8>")
    );
}


#[test]
fn esp32_uart0_ahb_fifo_emits_to_sink() {
    // Classic ESP32 IDF writes TX via UART_FIFO_AHB_REG(0)=0x6000_0000, not APB.
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    assert_eq!(
        bus.resolve_window(0x6000_0000),
        Some((0x6000_0000, 4)),
        "AHB FIFO window must win over wifi_mac_phy stub"
    );

    for &b in b"AHB!" {
        bus.write_u32(0x6000_0000, b as u32).unwrap();
    }
    // STATUS on APB must see the shared TX FIFO.
    let status = bus.read_u32(0x3FF4_001C).unwrap();
    assert_eq!((status >> 16) & 0xFF, 4, "TXFIFO_CNT via APB after AHB push");

    let uart0_idx = bus.find_peripheral_index_by_name("uart0").expect("uart0");
    for _ in 0..2_000_000 {
        let _ = bus.peripherals[uart0_idx].dev.tick();
        if sink.lock().unwrap().len() >= 4 {
            break;
        }
    }
    let bytes = sink.lock().unwrap();
    assert_eq!(bytes.as_slice(), b"AHB!", "AHB TX must reach sink, got {:?}", String::from_utf8_lossy(&bytes));
}

#[test]
fn esp32_sar_adc_oneshot_is_channel_dependent() {
    // The SENS SAR-ADC model must win the overlapping rtcio-stub window at
    // 0x3FF4_8800 and produce a genuine channel-dependent one-shot result.
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    const READ_CTRL: u64 = 0x3FF4_8800; // SAR_READ_CTRL
    const MEAS_START1: u64 = 0x3FF4_8800 + 0x54;
    const START: u32 = (1 << 18) | (1 << 17); // START_FORCE | START_SAR
    const DONE: u32 = 1 << 16;

    let convert = |bus: &mut SystemBus, channel: u32, sample_bit: u32| -> u32 {
        bus.write_u32(READ_CTRL, sample_bit << 16).unwrap();
        bus.write_u32(MEAS_START1, ((1u32 << channel) << 19) | START)
            .unwrap();
        let v = bus.read_u32(MEAS_START1).unwrap();
        assert_ne!(v & DONE, 0, "DONE must latch after START");
        v & 0xFFFF
    };

    let d3 = convert(&mut bus, 3, 3);
    let d5 = convert(&mut bus, 5, 3);
    assert_ne!(d3, 0);
    assert_ne!(d3, d5, "distinct channels must give distinct results");
    // 9-bit conversion of channel 5 is the 12-bit value >> 3.
    let d5_9 = convert(&mut bus, 5, 0);
    assert_eq!(d5_9, d5 >> 3, "result must scale with configured width");
}

#[test]
fn esp32_iram_round_trip() {
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    // Write a sentinel word to IRAM, read it back from both the
    // instruction-fetch view (IRAM at 0x4008_0000) — exercises the
    // SRAM0 backing the way real Xtensa code-load would.
    let addr = 0x4008_0100;
    bus.write_u32(addr, 0xDEAD_BEEF).unwrap();
    let v = bus.read_u32(addr).unwrap();
    assert_eq!(v, 0xDEAD_BEEF, "IRAM round-trip failed at 0x{:08X}", addr);
}
