// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! nRF54L10 emulated on the nRF54L15-DK: LED0 GPIO and UARTE20 VCOM smoke.

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO2_BASE: usize = 0x5005_0400;
const GPIO2_OUTSET: *mut u32 = (GPIO2_BASE + 0x004) as *mut u32;
const GPIO2_OUTCLR: *mut u32 = (GPIO2_BASE + 0x008) as *mut u32;
const GPIO2_DIRSET: *mut u32 = (GPIO2_BASE + 0x014) as *mut u32;
const LED0_P2_09: u32 = 1 << 9;

const UARTE20_BASE: usize = 0x500C_6000;
const UARTE20_TASKS_DMA_TX_START: *mut u32 = (UARTE20_BASE + 0x050) as *mut u32;
const UARTE20_TASKS_DMA_TX_STOP: *mut u32 = (UARTE20_BASE + 0x054) as *mut u32;
const UARTE20_EVENTS_TXSTOPPED: *mut u32 = (UARTE20_BASE + 0x130) as *mut u32;
const UARTE20_EVENTS_DMA_TX_END: *mut u32 = (UARTE20_BASE + 0x168) as *mut u32;
const UARTE20_ENABLE: *mut u32 = (UARTE20_BASE + 0x500) as *mut u32;
const UARTE20_BAUDRATE: *mut u32 = (UARTE20_BASE + 0x524) as *mut u32;
const UARTE20_PSEL_TXD: *mut u32 = (UARTE20_BASE + 0x604) as *mut u32;
const UARTE20_PSEL_RXD: *mut u32 = (UARTE20_BASE + 0x60C) as *mut u32;
const UARTE20_DMA_TX_PTR: *mut u32 = (UARTE20_BASE + 0x73C) as *mut u32;
const UARTE20_DMA_TX_MAXCNT: *mut u32 = (UARTE20_BASE + 0x740) as *mut u32;

const MESSAGE: [u8; 17] = *b"nRF54L10 boot OK\n";
const PSEL_P1_04: u32 = (1 << 5) | 4;
const PSEL_P1_05: u32 = (1 << 5) | 5;

// A mutable initialized static is placed in .data and copied into SRAM by the
// reset handler. Nordic EasyDMA requires the TX buffer to be in RAM.
static mut TX_BUFFER: [u8; MESSAGE.len()] = MESSAGE;

#[inline(never)]
fn spin() {
    for _ in 0..64 {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

unsafe fn write_banner() {
    write_volatile(UARTE20_PSEL_TXD, PSEL_P1_04);
    write_volatile(UARTE20_PSEL_RXD, PSEL_P1_05);
    write_volatile(UARTE20_BAUDRATE, 0x01D7_E000);
    write_volatile(UARTE20_ENABLE, 8);
    write_volatile(UARTE20_EVENTS_DMA_TX_END, 0);
    write_volatile(UARTE20_EVENTS_TXSTOPPED, 0);
    write_volatile(UARTE20_DMA_TX_PTR, addr_of_mut!(TX_BUFFER) as u32);
    write_volatile(UARTE20_DMA_TX_MAXCNT, MESSAGE.len() as u32);
    write_volatile(UARTE20_TASKS_DMA_TX_START, 1);

    while read_volatile(UARTE20_EVENTS_DMA_TX_END) == 0 {
        core::hint::spin_loop();
    }

    write_volatile(UARTE20_EVENTS_DMA_TX_END, 0);
    write_volatile(UARTE20_TASKS_DMA_TX_STOP, 1);
}

#[entry]
fn main() -> ! {
    unsafe {
        // LED0 is active-high on the nRF54L15-DK. Leave it high after edges.
        write_volatile(GPIO2_DIRSET, LED0_P2_09);
        write_volatile(GPIO2_OUTCLR, LED0_P2_09);
        for _ in 0..8 {
            write_volatile(GPIO2_OUTSET, LED0_P2_09);
            spin();
            write_volatile(GPIO2_OUTCLR, LED0_P2_09);
            spin();
        }
        write_volatile(GPIO2_OUTSET, LED0_P2_09);
        write_banner();
    }

    loop {
        core::hint::spin_loop();
    }
}
