// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! NUCLEO-H563ZI silicon/sim smoke firmware.
//!
//! One ELF, two targets: flash it to the real board (probe-rs) AND run it on
//! the simulator (`configs/chips/stm32h563.yaml`), then diff the UART output.
//! Unlike the tier1 fixture (which leans on the sim's always-on peripherals),
//! this firmware performs the full real-silicon USART3 bring-up:
//!
//! 1. RCC: GPIOB+GPIOD clocks on AHB2ENR, USART3 on APB1LENR.
//! 2. GPIO: PD8/PD9 → AF7 (USART3 TX/RX), PB0 (green LED) → output.
//! 3. USART3: BRR = 32 MHz / 115200 (HSI 64 MHz ÷2 out of reset feeds
//!    rcc_pclk1 with no APB prescaling), then UE|TE.
//! 4. Report boot-state registers over the UART:
//!    `H563-SMOKE CR=<RCC_CR> MODERA=<GPIOA_MODER> CALIB=<SYSTICK_CALIB>`
//!    then `H563-SMOKE done`, then blink PB0 forever.
//!
//! The three reported words are exactly the values pinned by the silicon
//! capture (`scripts/hw-capture-stm32h563.sh`), so identical sim and silicon
//! output lines are the smoke-level conformance check.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4402_0C00;
const RCC_AHB2ENR: u32 = RCC_BASE + 0x8C;
const RCC_APB1LENR: u32 = RCC_BASE + 0x9C;

const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOB_BASE: u32 = 0x4202_0400;
const GPIOD_BASE: u32 = 0x4202_0C00;
const MODER: u32 = 0x00;
const BSRR: u32 = 0x18;
const AFRH: u32 = 0x24;

const USART3_BASE: u32 = 0x4000_4800;
const USART_CR1: u32 = USART3_BASE + 0x00;
const USART_BRR: u32 = USART3_BASE + 0x0C;
const USART_ISR: u32 = USART3_BASE + 0x1C;
const USART_TDR: u32 = USART3_BASE + 0x28;
const TXE: u32 = 1 << 7;

const SYSTICK_CALIB: u32 = 0xE000_E01C;

fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(byte: u8) {
    for _ in 0..100_000 {
        if rd32(USART_ISR) & TXE != 0 {
            break;
        }
    }
    unsafe { write_volatile(USART_TDR as *mut u8, byte) };
}

fn puts(s: &[u8]) {
    for &b in s {
        putc(b);
    }
}

fn put_hex(value: u32) {
    for i in (0..8).rev() {
        let nibble = (value >> (i * 4)) & 0xF;
        putc(b"0123456789ABCDEF"[nibble as usize]);
    }
}

fn spin(iters: u32) {
    for i in 0..iters {
        core::hint::black_box(i);
    }
}

#[entry]
fn main() -> ! {
    // Capture boot state BEFORE touching anything.
    let rcc_cr = rd32(RCC_BASE);
    let moder_a = rd32(GPIOA_BASE + MODER);
    let calib = rd32(SYSTICK_CALIB);

    // Clocks: GPIOB(1)|GPIOD(3) on AHB2, USART3(18) on APB1L.
    wr32(RCC_AHB2ENR, rd32(RCC_AHB2ENR) | (1 << 1) | (1 << 3));
    wr32(RCC_APB1LENR, rd32(RCC_APB1LENR) | (1 << 18));

    // PD8/PD9 → alternate function 7 (USART3 TX/RX).
    let afrh = rd32(GPIOD_BASE + AFRH);
    wr32(GPIOD_BASE + AFRH, (afrh & !0xFF) | 0x77);
    let moder_d = rd32(GPIOD_BASE + MODER);
    wr32(GPIOD_BASE + MODER, (moder_d & !0xF_0000) | 0xA_0000);

    // PB0 (green LED) → output.
    let moder_b = rd32(GPIOB_BASE + MODER);
    wr32(GPIOB_BASE + MODER, (moder_b & !0x3) | 0x1);

    // USART3: 115200 8N1 from the 32 MHz reset clock, then enable+transmit.
    wr32(USART_BRR, 32_000_000 / 115_200);
    wr32(USART_CR1, (1 << 0) | (1 << 3)); // UE | TE

    puts(b"H563-SMOKE CR=");
    put_hex(rcc_cr);
    puts(b" MODERA=");
    put_hex(moder_a);
    puts(b" CALIB=");
    put_hex(calib);
    puts(b"\nH563-SMOKE done\n");

    // Blink PB0 forever (visible proof of life on the bench).
    loop {
        wr32(GPIOB_BASE + BSRR, 1 << 0);
        spin(400_000);
        wr32(GPIOB_BASE + BSRR, 1 << 16);
        spin(400_000);
    }
}
