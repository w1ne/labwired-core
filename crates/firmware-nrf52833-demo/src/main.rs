// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// micro:bit v2 (nRF52833) UART smoke firmware.
//
// Enables UARTE0 the way an nRF52 driver does — PSEL.TXD/RXD, BAUDRATE,
// ENABLE — then pushes "OK\n" through the EasyDMA TXD path and waits for
// EVENTS_ENDTX. Bare-register driver, no HAL; the same binary targets silicon
// and the model.
//
// micro:bit v2 UART is bridged to the interface MCU (KL27/DAPLink): target TX
// is P0.06 and target RX is P1.08 (tech.microbit.org/hardware/schematic/; the
// schematic's UART_INT_* labels are from the interface chip's perspective, and
// codal-microbit-v2 sets MICROBIT_PIN_UART_TX = P0.06). No LED is driven: the
// 5x5 matrix is charlieplexed and is not modelled.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// nRF52833 UARTE0 (PS v1.7 §6.33), same register map as nRF52840 UARTE0.
const UART0_BASE: u32 = 0x40002000;
const UART0_TASKS_STARTTX: *mut u32 = (UART0_BASE + 0x008) as *mut u32;
const UART0_EVENTS_ENDTX: *mut u32 = (UART0_BASE + 0x120) as *mut u32;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_PSEL_TXD: *mut u32 = (UART0_BASE + 0x50C) as *mut u32;
const UART0_PSEL_RXD: *mut u32 = (UART0_BASE + 0x514) as *mut u32;
const UART0_BAUDRATE: *mut u32 = (UART0_BASE + 0x524) as *mut u32;
const UART0_TXD_PTR: *mut u32 = (UART0_BASE + 0x544) as *mut u32;
const UART0_TXD_MAXCNT: *mut u32 = (UART0_BASE + 0x548) as *mut u32;

const UARTE_ENABLE: u32 = 8; // 8 = UARTE (EasyDMA); 4 would be the legacy UART
const BAUDRATE_115200: u32 = 0x01D6_0000; // PS: round(115200 * 2^32 / 16 MHz)

// PSEL fields: PIN [4:0], PORT [5], CONNECT (bit 31; 0 = connected).
const PSEL_TXD_P0_06: u32 = 6;
const PSEL_RXD_P1_08: u32 = (1 << 5) | 8;

// EasyDMA reads from RAM, never flash. A `static mut` with an initializer
// lives in .data, which the reset handler copies into RAM before `main`. A
// stack local is NOT used here: its initialization can be dead-code-eliminated
// before the TXD.PTR write (observed — the DMA read zeros), because casting
// the pointer to u32 erases provenance as far as LLVM is concerned.
static mut MSG: [u8; 3] = *b"OK\n";

#[entry]
fn main() -> ! {
    let msg_ptr = core::ptr::addr_of!(MSG) as *const u8 as u32;

    unsafe {
        write_volatile(UART0_PSEL_TXD, PSEL_TXD_P0_06);
        write_volatile(UART0_PSEL_RXD, PSEL_RXD_P1_08);
        write_volatile(UART0_BAUDRATE, BAUDRATE_115200);
        write_volatile(UART0_ENABLE, UARTE_ENABLE);

        write_volatile(UART0_EVENTS_ENDTX, 0);
        write_volatile(UART0_TXD_PTR, msg_ptr);
        write_volatile(UART0_TXD_MAXCNT, 3);
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        write_volatile(UART0_TASKS_STARTTX, 1);
        while read_volatile(UART0_EVENTS_ENDTX) == 0 {}
    }

    #[allow(clippy::empty_loop)] // bare-metal firmware: park the core forever
    loop {}
}
