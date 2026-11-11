// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal ESP32-S2 (Xtensa LX7) system wiring for the DevKitC-1 smoke path.
//!
//! The board onboarding slice maps the S2's internal SRAM, its two flash-XIP
//! windows, and UART0. It intentionally does not register Wi-Fi, USB OTG,
//! PSRAM, or the board's GPIO18 addressable RGB LED.

use super::RamPeripheral;
use crate::bus::SystemBus;
use crate::cpu::xtensa_lx7::XtensaLx7;
use crate::memory::LinearMemory;
use crate::peripherals::esp_uart::EspUart;
use crate::Cpu;

/// UART0 register window on ESP32-S2's PeriBUS2.
pub const ESP32S2_UART0_BASE: u64 = 0x6000_0000;
/// UART0 interrupt-matrix source ID (ESP-IDF ETS_UART0_INTR_SOURCE).
pub const ESP32S2_UART0_INTR_SOURCE: u32 = 27;
/// ESP32-S2 maximum core clock, used by the currently modelled board.
pub const ESP32S2_CPU_CLOCK_HZ: u32 = 240_000_000;

/// Install only the S2 memories and UART0 used by the onboarding smoke.
///
/// Firmware ELF segments are loaded into the S2's architectural IROM/DROM
/// windows before the core is released at its entry point. This is the usual
/// LabWired fast-start path: the mask ROM, SPI flash controller/MMU setup, and
/// 2 MiB module PSRAM are outside this model slice.
pub fn configure_xtensa_esp32s2(bus: &mut SystemBus, cpu_clock_hz: u32) -> XtensaLx7 {
    // SystemBus::empty() is used by the S2 builder, but clear here as well so
    // this helper cannot leave the default STM32 peripheral map installed.
    bus.peripherals.clear();
    bus.bit_band_enabled = false;

    // ESP32-S2 TRM §3.3.2: 320 KiB SRAM is exposed through data and
    // instruction address ranges. These views are separate backing stores in
    // this fast-start slice; the smoke does not depend on cross-view aliasing.
    bus.add_peripheral(
        "dram",
        0x3FFB_0000,
        320 * 1024,
        None,
        Box::new(RamPeripheral::new(320 * 1024)),
    );
    bus.add_peripheral(
        "iram",
        0x4002_0000,
        320 * 1024,
        None,
        Box::new(RamPeripheral::new(320 * 1024)),
    );
    // The S2's instruction and data cache windows are direct-mapped for this
    // ELF fast-start profile. Store them as CPU-visible memory regions so the
    // ELF loader can populate them before starting the core. The real boot
    // ROM/MMU configuration is not run.
    bus.extra_mem
        .push(LinearMemory::new(7 * 1024 * 1024 + 512 * 1024, 0x4008_0000));
    bus.extra_mem
        .push(LinearMemory::new(4 * 1024 * 1024, 0x3F00_0000));
    bus.add_peripheral(
        "rtc_fast",
        0x4007_0000,
        8 * 1024,
        None,
        Box::new(RamPeripheral::new(8 * 1024)),
    );
    bus.add_peripheral(
        "rtc_slow",
        0x5000_0000,
        8 * 1024,
        None,
        Box::new(RamPeripheral::new(8 * 1024)),
    );

    // ESP32-S2 TRM §23.4 gives UART0's PeriBUS2 base as 0x6000_0000. The
    // FIFO/register IP is shared with the existing Espressif UART model.
    bus.add_peripheral(
        "uart0",
        ESP32S2_UART0_BASE,
        0x100,
        None,
        Box::new(EspUart::new_with_cpu_clock(
            false,
            ESP32S2_UART0_INTR_SOURCE,
            u64::from(cpu_clock_hz),
        )),
    );

    let mut cpu = XtensaLx7::new();
    cpu.reset(bus).expect("ESP32-S2 Xtensa reset");
    cpu
}
