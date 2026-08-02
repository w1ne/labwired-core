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
use crate::{Bus, Cpu};
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

/// The classic-ESP32 boot ROM's per-character console sink. `esp-println`'s
/// `esp32` backend transmutes this exact address and calls it once per byte, so
/// it carries the console output of every bare-metal Rust firmware we run.
/// Sourced from Espressif's `esp32.rom.ld`, not invented here.
const ROM_UART_TX_ONE_CHAR: u32 = 0x4000_9200;

// The ROM console routine must be REAL CODE, not a BREAK dispatching a thunk.
//
// `RomThunkBank` pre-fills its whole range with `BREAK 1,14`, and an
// unregistered site falls back to `nop_return_zero`. So a thunk here — whether
// registered deliberately or inherited from the fallback — silently swallows
// every byte the firmware prints, and looks identical to a correct setup from
// the outside. This pins the bytes at the entry point as the ROM's own
// `entry a1, 32`, which is what `install_rom_console` puts there.
#[test]
fn esp32_rom_console_entry_is_real_code_not_a_break() {
    use crate::peripherals::esp_xtensa_common::rom_thunks::ROM_THUNK_BREAK_BYTES;
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let mut got = [0u8; 3];
    for (i, b) in got.iter_mut().enumerate() {
        *b = bus.read_u8(ROM_UART_TX_ONE_CHAR as u64 + i as u64).unwrap();
    }
    assert_ne!(
        got, ROM_THUNK_BREAK_BYTES,
        "uart_tx_one_char is a thunk dispatch site again — console output will be discarded"
    );
    // `entry a1, 32`, the first instruction of the real ROM routine.
    assert_eq!(got, [0x36, 0x41, 0x00], "not the real ROM entry sequence");
}

// End to end at the instruction level: CALL8 into the boot ROM's real
// `uart_tx_one_char` and watch the byte come out of the UART sink.
//
// Everything here is the genuine article — the CPU fetches and decodes ROM
// instructions, executes the windowed ABI (`entry` / `retw.n`), loads the
// literals via `l32r`, computes the UART base from the ROM's own descriptor,
// polls STATUS and stores to the FIFO through the ordinary bus. No thunk is
// dispatched at any point.
#[test]
fn esp32_rom_uart_tx_one_char_executes_and_reaches_the_sink() {
    use crate::{SimulationConfig, SimulationObserver};
    use std::sync::Arc;

    let mut bus = SystemBus::empty();
    let mut cpu = configure_xtensa_esp32(&mut bus);
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    // A one-instruction caller in IRAM: `call8 uart_tx_one_char`.
    // CALLn encoding: op0=0x5 in bits[3:0], n in bits[5:4], an 18-bit signed
    // word offset in bits[23:6], with
    // target = (PC & !3) + (offset << 2) + 4.
    const STUB: u32 = 0x4008_0000;
    let offset = ((ROM_UART_TX_ONE_CHAR as i64 - ((STUB & !3) as i64 + 4)) >> 2) as i32;
    let word = 0x5u32 | (2 << 4) | ((offset as u32 & 0x3_FFFF) << 6);
    for i in 0..3 {
        bus.write_u8(STUB as u64 + i, (word >> (8 * i)) as u8)
            .unwrap();
    }

    // CALL8 rotates the register window by 8, so the callee's a2 (its argument)
    // is the caller's a10. a1 is the stack pointer the ROM's `entry` will frame
    // from — point it at real DRAM.
    cpu.regs.write_logical(1, 0x3FFB_8000);
    cpu.regs.write_logical(10, b'Z' as u32);
    cpu.set_pc(STUB);

    let observers: Vec<Arc<dyn SimulationObserver>> = Vec::new();
    let config = SimulationConfig::default();
    let mut returned = false;
    for _ in 0..10_000 {
        cpu.step(&mut bus, &observers, &config)
            .expect("ROM console code must execute cleanly");
        if cpu.get_pc() == STUB + 3 {
            returned = true;
            break;
        }
    }
    assert!(returned, "uart_tx_one_char never returned to its caller");

    let uart0 = bus.find_peripheral_index_by_name("uart0").expect("uart0");
    for _ in 0..2_000_000 {
        let _ = bus.peripherals[uart0].dev.tick();
        if !sink.lock().unwrap().is_empty() {
            break;
        }
    }
    assert_eq!(
        sink.lock().unwrap().as_slice(),
        b"Z",
        "the byte the ROM stored must shift out of UART0"
    );
}

// The real `ets_printf`, formatting and all, out through UART0.
//
// This is the deepest of the console paths and it is now entirely real code:
// `ets_printf` parses the format string, `_cvt` converts the integer (calling
// the ROM's `__udivdi3` / `__umoddi3`), digits come from the ROM's own rodata
// table, each character goes to `ets_write_char`, which dispatches via `callx8`
// through the installed `putc1` to `ets_write_char_uart`, which expands `\n` to
// `\r\n` and calls `uart_tx_one_char`, which spins on STATUS and stores to the
// FIFO. A single Rust function stood in for that entire chain and wrote to
// `tracing::info!`, so nothing reached the UART.
//
// `%d` is deliberate: it exercises `_cvt`, the division thunks and the rodata
// digit table, which a plain string would leave untested. The `\r` proves the
// newline expansion, which a reimplementation would have missed.
#[test]
fn esp32_rom_ets_printf_formats_and_reaches_the_sink() {
    use crate::{SimulationConfig, SimulationObserver};
    use std::sync::Arc;

    const ROM_ETS_PRINTF: u32 = 0x4000_7d54;
    const STUB: u32 = 0x4008_0000;
    const FMT: u32 = 0x3FFB_1000;

    let mut bus = SystemBus::empty();
    let mut cpu = configure_xtensa_esp32(&mut bus);
    let sink = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(sink.clone(), false);

    for (i, b) in b"n=%d\n\0".iter().enumerate() {
        bus.write_u8(FMT as u64 + i as u64, *b).unwrap();
    }

    // `call8 ets_printf` — see the CALLn encoding note in the test above.
    let offset = ((ROM_ETS_PRINTF as i64 - ((STUB & !3) as i64 + 4)) >> 2) as i32;
    let word = 0x5u32 | (2 << 4) | ((offset as u32 & 0x3_FFFF) << 6);
    for i in 0..3 {
        bus.write_u8(STUB as u64 + i, (word >> (8 * i)) as u8)
            .unwrap();
    }

    // CALL8 rotates the window by 8: the callee's a2/a3 are the caller's
    // a10/a11 — here the format string and its one argument.
    cpu.regs.write_logical(1, 0x3FFB_8000);
    cpu.regs.write_logical(10, FMT);
    cpu.regs.write_logical(11, 42);
    cpu.set_pc(STUB);

    let observers: Vec<Arc<dyn SimulationObserver>> = Vec::new();
    let config = SimulationConfig::default();
    let uart0 = bus.find_peripheral_index_by_name("uart0").expect("uart0");
    let mut returned = false;
    for _ in 0..2_000_000 {
        cpu.step(&mut bus, &observers, &config)
            .expect("ROM ets_printf must execute cleanly");
        // Tick the UART alongside the CPU: the ROM spins on STATUS while the
        // FIFO drains, so a CPU-only loop would deadlock exactly as silicon
        // would if the shift register never advanced.
        let _ = bus.peripherals[uart0].dev.tick();
        if cpu.get_pc() == STUB + 3 {
            returned = true;
            break;
        }
    }
    assert!(returned, "ets_printf never returned to its caller");

    for _ in 0..4_000_000 {
        let _ = bus.peripherals[uart0].dev.tick();
        if sink.lock().unwrap().len() >= 6 {
            break;
        }
    }
    let got = String::from_utf8_lossy(&sink.lock().unwrap()).to_string();
    assert_eq!(
        got, "n=42\r\n",
        "real ets_printf must format through the ROM and reach UART0"
    );
}

// An Adafruit TFT FeatherWing puts the panel's D/C line on GPIO33 — the HIGH
// bank. The pin->output-register resolver capped at pad 31, so that pin was
// unresolvable, `set_dc_source` was never called, and the bus never latched
// D/C. The panel then framed every byte as a command: display on, zero pixels,
// no error anywhere. Both banks must resolve, to the register that actually
// holds the pad.
#[test]
fn esp32_high_bank_pads_resolve_to_the_out1_register() {
    let mut bus = SystemBus::empty();
    let _cpu = configure_xtensa_esp32(&mut bus);
    let gpio_base = {
        let idx = bus.find_peripheral_index_by_name("gpio").expect("gpio");
        bus.peripherals[idx].base
    };

    let low = SystemBus::resolve_pin_odr_pub(&bus, "GPIO5").expect("low bank resolves");
    assert_eq!(low, (gpio_base + 0x04, 5), "GPIO5 -> GPIO_OUT bit 5");

    let high = SystemBus::resolve_pin_odr_pub(&bus, "GPIO33").expect("high bank must resolve");
    assert_eq!(high, (gpio_base + 0x10, 1), "GPIO33 -> GPIO_OUT1 bit 1");

    // Pads stop at 39 on this part; 40 is not a pin.
    assert!(SystemBus::resolve_pin_odr_pub(&bus, "GPIO40").is_none());
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
    assert_eq!(
        (status >> 16) & 0xFF,
        4,
        "TXFIFO_CNT via APB after AHB push"
    );

    let uart0_idx = bus.find_peripheral_index_by_name("uart0").expect("uart0");
    for _ in 0..2_000_000 {
        let _ = bus.peripherals[uart0_idx].dev.tick();
        if sink.lock().unwrap().len() >= 4 {
            break;
        }
    }
    let bytes = sink.lock().unwrap();
    assert_eq!(
        bytes.as_slice(),
        b"AHB!",
        "AHB TX must reach sink, got {:?}",
        String::from_utf8_lossy(&bytes)
    );
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
