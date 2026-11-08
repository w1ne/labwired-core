// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! nRF54LM20A-emulated-on-DK smoke: UARTE20 VCOM and active-high LED0 P1.22.

#![no_std]
#![no_main]

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO1_BASE: usize = 0x500D_8200;
const GPIO1_OUTSET: *mut u32 = (GPIO1_BASE + 0x004) as *mut u32;
const GPIO1_OUTCLR: *mut u32 = (GPIO1_BASE + 0x008) as *mut u32;
const GPIO1_PIN_CNF22: *mut u32 = (GPIO1_BASE + 0x0D8) as *mut u32;
const LED0_P1_22: u32 = 1 << 22;

const UARTE20_BASE: usize = 0x500C_6000;
const UARTE20_TASKS_DMA_TX_START: *mut u32 = (UARTE20_BASE + 0x050) as *mut u32;
const UARTE20_EVENTS_DMA_TX_END: *mut u32 = (UARTE20_BASE + 0x168) as *mut u32;
const UARTE20_ENABLE: *mut u32 = (UARTE20_BASE + 0x500) as *mut u32;
const UARTE20_BAUDRATE: *mut u32 = (UARTE20_BASE + 0x524) as *mut u32;
const UARTE20_PSEL_TXD: *mut u32 = (UARTE20_BASE + 0x604) as *mut u32;
const UARTE20_PSEL_RXD: *mut u32 = (UARTE20_BASE + 0x60C) as *mut u32;
const UARTE20_DMA_TX_PTR: *mut u32 = (UARTE20_BASE + 0x73C) as *mut u32;
const UARTE20_DMA_TX_MAXCNT: *mut u32 = (UARTE20_BASE + 0x740) as *mut u32;

const MESSAGE: [u8; 25] = *b"NRF54LM20DK_LED_SMOKE_OK\n";
const PSEL_P1_16: u32 = (1 << 5) | 16;
const PSEL_P1_17: u32 = (1 << 5) | 17;

// Nordic EasyDMA reads the source from RAM, so keep the message in writable
// initialized data instead of passing its RRAM string-literal address.
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
    write_volatile(UARTE20_PSEL_TXD, PSEL_P1_16);
    write_volatile(UARTE20_PSEL_RXD, PSEL_P1_17);
    write_volatile(UARTE20_BAUDRATE, 0x01D7_E000);
    write_volatile(UARTE20_ENABLE, 8);
    write_volatile(UARTE20_EVENTS_DMA_TX_END, 0);
    write_volatile(UARTE20_DMA_TX_PTR, addr_of_mut!(TX_BUFFER) as u32);
    write_volatile(UARTE20_DMA_TX_MAXCNT, MESSAGE.len() as u32);
    write_volatile(UARTE20_TASKS_DMA_TX_START, 1);

    while read_volatile(UARTE20_EVENTS_DMA_TX_END) == 0 {
        core::hint::spin_loop();
    }

    write_volatile(UARTE20_EVENTS_DMA_TX_END, 0);
}

#[entry]
fn main() -> ! {
    unsafe {
        // Nordic's DK hardware guide maps LED0 to active-high P1.22.
        write_volatile(GPIO1_PIN_CNF22, 0x3);
        write_volatile(GPIO1_OUTCLR, LED0_P1_22);
        for _ in 0..8 {
            write_volatile(GPIO1_OUTSET, LED0_P1_22);
            spin();
            write_volatile(GPIO1_OUTCLR, LED0_P1_22);
            spin();
        }
        write_volatile(GPIO1_OUTSET, LED0_P1_22);
        write_banner();
    }

    loop {
        core::hint::spin_loop();
    }
}
