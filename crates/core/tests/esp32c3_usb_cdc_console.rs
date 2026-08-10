// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ESP32-C3 USB CDC console gate: an **interrupt-driven** `HWCDC` build must
//! emit its `Serial` output through the USB_SERIAL_JTAG sink.
//!
//! # Why this gate exists
//!
//! An ESP32-C3 SuperMini wires its USB-C socket to the chip's own
//! USB-Serial-JTAG block, not to UART0. Firmware for such a board is built with
//! `-DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1`, which makes Arduino's
//! `Serial` an `HWCDC` instead of a `HardwareSerial`. `HardwareSerial` is not
//! even linked into that image — there is no UART0 console to fall back to.
//!
//! `HWCDC` is entirely interrupt-driven. The assertion below is derived from
//! the DRIVER, not from this simulator's model
//! (`framework-arduinoespressif32@3.20017` `cores/esp32/HWCDC.cpp`):
//!
//! * `HWCDC::write` (line 419) forwards to the FIFO only `if(isCDC_Connected())`.
//! * `isCDC_Connected` (line 192) returns false whenever `isPlugged()` is false,
//!   and `write` then calls `flushTXBuffer`, which **discards** the bytes by
//!   cycling them through the ring buffer. Output is silently dropped.
//! * `isPlugged()` (line 184) returns `s_usb_serial_jtag_conn_status`, which the
//!   FreeRTOS tick hook `usb_serial_jtag_sof_tick_hook` (line 47) drives from
//!   `USB_SERIAL_JTAG.int_raw.sof_int_raw`. It starts `true`, and is latched
//!   `false` for good once SOF has been absent for `ALLOWED_NO_SOF_TICKS`
//!   (`pdMS_TO_TICKS(5)`) consecutive ticks. **A twin that never raises SOF is
//!   modelling an UNPLUGGED board.**
//! * Bytes reach the FIFO only from `hw_cdc_isr_handler` (line 111), which the
//!   interrupt matrix must deliver on `SERIAL_IN_EMPTY`. `HWCDC::begin`
//!   (line 346) binds it with `esp_intr_alloc(ETS_USB_SERIAL_JTAG_INTR_SOURCE,
//!   ...)`; on the C3 that source id is 26 — see the compiled image, where
//!   `HWCDC::begin` does `li a0,26` immediately before `jal esp_intr_alloc`.
//!
//! So three separate things must be true of the model for ONE `Serial.println`
//! to leave the chip: SOF must tick, INT_ENA must be real storage that INT_ST
//! honours, and the source must reach the CPU through the matrix. This test
//! asserts only the observable end of that chain.
//!
//! # What the fixture is
//!
//! `crates/core/tests/fixtures/esp32c3-usb-cdc-console-flash.bin` is a real
//! Arduino image for `board = esp32-c3-supermini`, built by PlatformIO with
//! `platform = espressif32@7.0.1` and
//! `-DARDUINO_USB_CDC_ON_BOOT=1 -DARDUINO_USB_MODE=1`. Its sketch prints
//! `LW_CDC_SETUP` once from `setup()` and then `LW_CDC_LOOP <n>` (CRLF-terminated, as Arduino `println` emits) from `loop()`
//! every 50 ms.
//!
//! The gate asserts on a LOOP line, not the setup line, and specifically on an
//! iteration past the first: a sketch whose `loop()` is dead can still print
//! from `setup()`, so `LW_CDC_SETUP` alone would be a vacuous pass.
//!
//! # The control
//!
//! "The CDC sink is empty" on its own proves nothing — it is equally consistent
//! with a firmware that never booted. So the SAME sketch is also built with
//! `-DARDUINO_USB_CDC_ON_BOOT=0` (`esp32c3-uart0-console-control-flash.bin`),
//! which puts `Serial` on `HardwareSerial`/UART0 instead. The two images differ
//! in the console and nothing else: the CDC image links `HWCDC` and NO
//! `HardwareSerial`; the control links `HardwareSerial` and NO `HWCDC`.
//!
//! `uart0_control_build_emits_loop_output` boots the control through the very
//! same harness. It passes both before and after the interrupt model exists, so
//! it pins down that the ROM boot, the bootloader, the Arduino startup and
//! `loop()` all work — and therefore that a silent CDC image is the CDC model's
//! fault and not the twin's.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::peripherals::esp32s3::usb_serial_jtag::UsbSerialJtag;
use labwired_core::{Arch, Bus, Cpu, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

fn esp32c3_bootloader_image(flash: &[u8]) -> ProgramImage {
    assert!(flash.len() > ESP_IMAGE_HEADER_LEN, "flash image truncated");
    assert_eq!(flash[0], ESP_IMAGE_MAGIC, "bad bootloader image magic");
    let segment_count = flash[1] as usize;
    let entry = u32::from_le_bytes(flash[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, Arch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;
    for _ in 0..segment_count {
        let load_addr = u32::from_le_bytes(flash[cursor..cursor + 4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(flash[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        program.add_segment(load_addr, flash[cursor..cursor + len].to_vec());
        cursor += len;
    }
    program
}

struct CdcLab {
    machine: Machine<RiscV>,
    /// Bytes the firmware pushed through USB_SERIAL_JTAG EP1 — the CDC console.
    cdc: Arc<Mutex<Vec<u8>>>,
    /// Bytes the firmware pushed through UART0. Captured to prove the CDC
    /// output is not merely UART0 output arriving by another name.
    uart: Arc<Mutex<Vec<u8>>>,
}

fn build_cdc_lab() -> CdcLab {
    build_lab("tests/fixtures/esp32c3-usb-cdc-console-flash.bin")
}

fn build_lab(fixture: &str) -> CdcLab {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-oled-demo.yaml"))
            .expect("load esp32c3 system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build C3 bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    let flash_path = root().join(fixture);
    let flash = std::fs::read(&flash_path)
        .unwrap_or_else(|e| panic!("read flash image {}: {e}", flash_path.display()));

    assert!(
        inject_rom_regions(
            &mut bus,
            &RomImages {
                irom: irom.clone(),
                drom,
            }
        ),
        "chip yaml must declare the C3 IROM region"
    );
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let uart = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(uart.clone(), false);

    let cdc = Arc::new(Mutex::new(Vec::new()));
    let bootloader = esp32c3_bootloader_image(&flash);
    let mut machine = build_rom_boot_machine(
        bus,
        flash,
        RomBootOpts {
            pinned_efuse_mac: Some(labwired_core::system::efuse::FIRST_FACTORY_MAC),
            // The CDC console tap. core#831 wires this up from the board
            // descriptor; this gate attaches it directly so it stands alone.
            usb_serial_sink: Some(cdc.clone()),
        },
        |c| c,
    );

    for segment in &bootloader.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }
        for (i, byte) in segment.data.iter().enumerate() {
            machine
                .bus
                .write_u8(segment.start_addr + i as u64, *byte)
                .expect("load bootloader segment");
        }
    }
    let sp_top = (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(bootloader.entry_point as u32);

    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled = true;

    CdcLab { machine, cdc, uart }
}

fn budget() -> u64 {
    std::env::var("LABWIRED_C3_CDC_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120_000_000)
}

/// A CDC-on-boot Arduino build must get its `loop()` output out of the chip.
///
/// Asserts on the SECOND loop iteration (`LW_CDC_LOOP 1`), so a firmware that
/// printed once from `setup()` and then wedged cannot pass.
#[test]
fn cdc_on_boot_build_emits_loop_output_through_usb_serial_jtag() {
    let mut lab = build_cdc_lab();
    let budget = budget();
    let mut steps = 0u64;
    let mut found_at = None;

    while steps < budget {
        let chunk = 200_000u64;
        for _ in 0..chunk {
            if lab.machine.step().is_err() {
                break;
            }
        }
        steps += chunk;
        let got = lab.cdc.lock().unwrap().clone();
        if twoway_contains(&got, b"LW_CDC_LOOP 1\r\n") {
            found_at = Some(steps);
            break;
        }
    }

    let cdc = lab.cdc.lock().unwrap().clone();
    let uart = lab.uart.lock().unwrap().clone();
    assert!(
        found_at.is_some(),
        "no CDC loop output after {steps} steps.\n\
         USB_SERIAL_JTAG sink ({} bytes): {:?}\n\
         UART0 sink ({} bytes, tail): {:?}\n\
         This image has NO HardwareSerial linked — if the CDC sink is empty the \
         firmware's console is invisible in the twin.",
        cdc.len(),
        String::from_utf8_lossy(&cdc[..cdc.len().min(400)]),
        uart.len(),
        String::from_utf8_lossy(&uart[uart.len().saturating_sub(400)..]),
    );
    eprintln!(
        "CDC loop output observed after {} steps ({} bytes captured)",
        found_at.unwrap(),
        cdc.len()
    );
}

fn twoway_contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// CONTROL. The identical sketch built for UART0 (`ARDUINO_USB_CDC_ON_BOOT=0`)
/// must reach the same `loop()` print through the same harness.
///
/// This is what makes the CDC assertion above meaningful. If BOTH tests fail,
/// the twin's C3 boot is broken and the CDC result says nothing about the
/// USB_SERIAL_JTAG model. Only "control passes, CDC fails" isolates the fault to
/// the console peripheral.
#[test]
fn uart0_control_build_emits_loop_output() {
    let mut lab = build_lab("tests/fixtures/esp32c3-uart0-console-control-flash.bin");
    let budget = budget();
    let mut steps = 0u64;
    let mut found_at = None;

    while steps < budget {
        let chunk = 200_000u64;
        for _ in 0..chunk {
            if lab.machine.step().is_err() {
                break;
            }
        }
        steps += chunk;
        if twoway_contains(&lab.uart.lock().unwrap(), b"LW_CDC_LOOP 1\r\n") {
            found_at = Some(steps);
            break;
        }
    }

    let uart = lab.uart.lock().unwrap().clone();
    assert!(
        found_at.is_some(),
        "control (UART0) build produced no loop output after {steps} steps; \
         the C3 boot harness itself is broken, so the CDC result proves nothing.\n\
         UART0 sink ({} bytes): {:?}",
        uart.len(),
        String::from_utf8_lossy(&uart[..uart.len().min(600)]),
    );
    eprintln!(
        "control UART0 loop output observed after {} steps ({} bytes)",
        found_at.unwrap(),
        uart.len()
    );
}

/// Run until `needle` shows up in `pick(&lab)`, or the budget runs out.
fn run_until(lab: &mut CdcLab, needle: &[u8], pick: fn(&CdcLab) -> Vec<u8>) -> Option<u64> {
    let budget = budget();
    let mut steps = 0u64;
    while steps < budget {
        for _ in 0..200_000u64 {
            if lab.machine.step().is_err() {
                break;
            }
        }
        steps += 200_000;
        if twoway_contains(&pick(lab), needle) {
            return Some(steps);
        }
    }
    None
}

fn step_n(lab: &mut CdcLab, n: u64) {
    for _ in 0..n {
        if lab.machine.step().is_err() {
            break;
        }
    }
}

fn with_usb_serial_jtag<R>(lab: &mut CdcLab, f: impl FnOnce(&mut UsbSerialJtag) -> R) -> R {
    let idx = lab
        .machine
        .bus
        .find_peripheral_index_by_name("usb_serial_jtag")
        .expect("C3 rom-boot registers usb_serial_jtag");
    let any = lab.machine.bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("usb_serial_jtag is downcastable");
    f(any
        .downcast_mut::<UsbSerialJtag>()
        .expect("usb_serial_jtag is the behavioural model"))
}

/// NEGATIVE CONTROL 1 — the CDC sink must not be a short-circuit for `Serial`.
///
/// The UART0 build's `Serial` is a `HardwareSerial`; it never touches
/// USB_SERIAL_JTAG. If its application output turns up at the CDC sink anyway,
/// then something is copying `Serial` into that sink instead of modelling the
/// peripheral, and the headline test would pass for the wrong reason.
///
/// The assertion is on APPLICATION output specifically. The C3 mask ROM
/// deliberately mirrors its own boot banner to both UART0 and the USB CDC port
/// (`usb_uart_tx_one_char`), so a few ROM bytes at the CDC sink are correct and
/// expected — what must be absent is anything the sketch printed.
#[test]
fn uart0_build_puts_no_application_output_on_the_cdc_sink() {
    let mut lab = build_lab("tests/fixtures/esp32c3-uart0-console-control-flash.bin");
    let found = run_until(&mut lab, b"LW_CDC_LOOP 1\r\n", |l| {
        l.uart.lock().unwrap().clone()
    });
    assert!(found.is_some(), "control build produced no UART0 output");

    let cdc = lab.cdc.lock().unwrap().clone();
    assert!(
        !twoway_contains(&cdc, b"LW_CDC_"),
        "UART0-console firmware leaked application output into the \
         USB_SERIAL_JTAG sink — `Serial` is being short-circuited into the CDC \
         sink rather than modelled.\nCDC sink ({} bytes): {:?}",
        cdc.len(),
        String::from_utf8_lossy(&cdc[..cdc.len().min(400)]),
    );
    eprintln!(
        "control: {} bytes at the CDC sink, none of them application output",
        cdc.len()
    );
}

/// NEGATIVE CONTROL 2 — stopping SOF must stop the console.
///
/// `HWCDC` decides it is plugged in purely from the SOF raw bit. If the twin
/// stops raising SOF, `s_usb_serial_jtag_conn_status` must latch false within
/// `pdMS_TO_TICKS(5)` ticks and `HWCDC::write` must start discarding output.
///
/// Bytes that keep flowing after SOF stops would mean `isPlugged()` is not
/// actually being consulted — i.e. the output is reaching the sink by some path
/// other than the modelled FIFO, and the headline test proves nothing about the
/// peripheral.
#[test]
fn output_stops_when_the_host_stops_sending_sof() {
    let mut lab = build_cdc_lab();
    let found = run_until(&mut lab, b"LW_CDC_LOOP 1\r\n", |l| {
        l.cdc.lock().unwrap().clone()
    });
    assert!(found.is_some(), "no CDC output to withdraw");

    with_usb_serial_jtag(&mut lab, |u| u.set_sof_enabled(false));

    // Grace window: the driver needs ~5 FreeRTOS ticks to latch "unplugged",
    // and anything already queued may still drain. 160 MHz => 1.6M cycles/10 ms.
    step_n(&mut lab, 8_000_000);
    let after_grace = lab.cdc.lock().unwrap().len();

    // The sketch prints every 50 ms. 40M steps is far more than enough for
    // several more lines if the console were still alive.
    step_n(&mut lab, 40_000_000);
    let settled = lab.cdc.lock().unwrap().clone();

    assert_eq!(
        settled.len(),
        after_grace,
        "console kept emitting {} bytes after SOF stopped; isPlugged() is not \
         being consulted, so bytes are bypassing the modelled FIFO.\nTail: {:?}",
        settled.len() - after_grace,
        String::from_utf8_lossy(&settled[after_grace.saturating_sub(80)..]),
    );
    eprintln!("SOF withdrawn: console went silent at {after_grace} bytes");
}

/// Byte conservation end to end: every byte the model handed the console came
/// through EP1's FIFO, and every byte accepted at EP1 reached the console.
#[test]
fn cdc_sink_bytes_all_came_through_the_ep1_fifo() {
    let mut lab = build_cdc_lab();
    let found = run_until(&mut lab, b"LW_CDC_LOOP 1\r\n", |l| {
        l.cdc.lock().unwrap().clone()
    });
    assert!(found.is_some(), "no CDC output to account for");

    let sink_len = lab.cdc.lock().unwrap().len() as u64;
    let (accepted, emitted) = with_usb_serial_jtag(&mut lab, |u| {
        (u.ep1_bytes_accepted(), u.sink_bytes_emitted())
    });
    assert_eq!(
        accepted, emitted,
        "bytes accepted at EP1 ({accepted}) != bytes emitted ({emitted})"
    );
    assert_eq!(
        sink_len, emitted,
        "the sink holds {sink_len} bytes but only {emitted} went through the FIFO"
    );
}
