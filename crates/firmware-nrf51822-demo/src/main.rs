// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// micro:bit V1 (nRF51822) UART smoke firmware.
//
// The nRF51 has no EasyDMA: its UART is the legacy one (nRF51 Series
// Reference Manual v3.0, §29 UART). Set PSELTXD/PSELRXD and BAUDRATE, ENABLE
// = 4, trigger STARTTX, then for each byte write TXD and wait for
// EVENTS_TXDRDY. Bare registers, no HAL, no SoftDevice.
//
// micro:bit V1 UART is bridged to the interface MCU (KL26/DAPLink): target
// TX = P0.24, RX = P0.25 (tech.microbit.org/hardware/1-5-revision/). No LED
// is driven: the matrix is not modelled.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const UART0_BASE: u32 = 0x40002000;
const UART0_TASKS_STARTTX: *mut u32 = (UART0_BASE + 0x008) as *mut u32;
const UART0_EVENTS_TXDRDY: *mut u32 = (UART0_BASE + 0x11C) as *mut u32;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_PSELTXD: *mut u32 = (UART0_BASE + 0x50C) as *mut u32;
const UART0_PSELRXD: *mut u32 = (UART0_BASE + 0x514) as *mut u32;
const UART0_TXD: *mut u32 = (UART0_BASE + 0x51C) as *mut u32;
const UART0_BAUDRATE: *mut u32 = (UART0_BASE + 0x524) as *mut u32;

const UART_ENABLE: u32 = 4; // legacy UART (RM v3.0 §29.8.10)
const BAUDRATE_115200: u32 = 0x01D7_E000; // RM v3.0 §29.8.13

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(UART0_PSELTXD, 24);
        write_volatile(UART0_PSELRXD, 25);
        write_volatile(UART0_BAUDRATE, BAUDRATE_115200);
        write_volatile(UART0_ENABLE, UART_ENABLE);
        write_volatile(UART0_TASKS_STARTTX, 1);
        for &b in b"OK\n" {
            write_volatile(UART0_EVENTS_TXDRDY, 0);
            write_volatile(UART0_TXD, b as u32);
            while read_volatile(UART0_EVENTS_TXDRDY) == 0 {}
        }
    }

    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}
