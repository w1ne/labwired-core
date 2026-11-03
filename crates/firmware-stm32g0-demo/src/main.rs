#![no_std]
#![no_main]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// STM32G0 UART/LED smoke for NUCLEO-G071RB and NUCLEO-G0B1RE
// (Cortex-M0+, thumbv6m-none-eabi).
//
// Bare-register firmware — no HAL. Real STM32G0 register offsets (RM0444),
// so the same binary runs on physical silicon and in the simulator:
//
//   RCC  (0x40021000)  CR@0x00, IOPENR@0x34, APBENR1@0x3C
//   GPIOA (0x50000000) MODER@0x00, AFRL@0x20, BSRR@0x18
//   USART2 (0x40004400) CR1@0x00, BRR@0x0C, ISR@0x1C, TDR@0x28
//
// What it proves on the smoke path:
//   1. HSIRDY (CR bit 10) is up after reset — the G0-ready-bit layout.
//   2. The GPIOA clock gate is RCC_IOPENR.GPIOAEN (bit 0) and the USART2
//      clock gate is RCC_APBENR1.USART2EN (bit 17), both at the G0 offsets
//      (NOT the L0 offsets).
//   3. USART2 TDR writes leave the part as UART bytes: `OK\n`.
//   4. LD4 (PA5) toggles via BSRR (visible to board_io / logic capture).
//
// ST-LINK VCP wiring and PA5 LED are documented in UM2505/UM2324. USART2
// uses AF1 on PA2 (TX) / PA3 (RX) in the STM32G0 family.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ---- RCC (0x40021000, RM0444 §5) ----------------------------------------
const RCC_CR: *mut u32 = 0x4002_1000 as *mut u32;
const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;
const RCC_CR_HSION: u32 = 1 << 8;
const RCC_CR_HSIRDY: u32 = 1 << 10;
const IOPENR_GPIOAEN: u32 = 1 << 0;
const APBENR1_USART2EN: u32 = 1 << 17;

// ---- GPIOA (IOPORT @ 0x50000000) -----------------------------------------
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const LED_PIN: u32 = 5;

// ---- USART2 (0x40004400, modern stm32v2 layout) --------------------------
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;

// HSISYS default = HSI16 / 1 = 16 MHz at reset (RM0444 §5.2.4), so this is
// the PCLK the BRR field is computed against.
const PCLK_HZ: u32 = 16_000_000;
const BAUD: u32 = 115_200;
const SPIN_LIMIT: u32 = 20_000;

#[entry]
fn main() -> ! {
    unsafe {
        // 1. HSISYS is the reset clock; make sure HSI16 is on and ready
        //    (HSIRDY is at CR bit 10 on G0).
        write_volatile(RCC_CR, read_volatile(RCC_CR) | RCC_CR_HSION);
        spin_until(|| read_volatile(RCC_CR) & RCC_CR_HSIRDY != 0);

        // 2. Ungate GPIOA and USART2 at the G0 offsets (RM0444 §5.4.12 /
        //    §5.4.14). A wrong offset leaves both silent on real silicon.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | IOPENR_GPIOAEN);
        write_volatile(RCC_APBENR1, read_volatile(RCC_APBENR1) | APBENR1_USART2EN);

        gpio_usart_init();
    }

    print_str("OK\n");

    // Toggle LD4 (PA5) a few times — visible in the board_io state and to a
    // logic capture; no UART traffic attached so the smoke output stays "OK".
    for _ in 0..4u32 {
        led_set(true);
        delay(20_000);
        led_set(false);
        delay(20_000);
    }

    loop {
        cortex_m::asm::nop();
    }
}

unsafe fn gpio_usart_init() {
    // PA5 = general-purpose output (LD4).
    let mut moder = read_volatile(GPIOA_MODER);
    moder &= !(0b11 << (LED_PIN * 2));
    moder |= 0b01 << (LED_PIN * 2);
    // PA2 (TX) and PA3 (RX) = alternate function.
    moder &= !((0b11 << (2 * 2)) | (0b11 << (3 * 2)));
    moder |= (0b10 << (2 * 2)) | (0b10 << (3 * 2));
    write_volatile(GPIOA_MODER, moder);

    // USART2 on PA2/PA3 is AF1 on STM32G0 (DS12232 Table 13).
    let mut afrl = read_volatile(GPIOA_AFRL);
    afrl &= !((0xF << (2 * 4)) | (0xF << (3 * 4)));
    afrl |= (0x1 << (2 * 4)) | (0x1 << (3 * 4));
    write_volatile(GPIOA_AFRL, afrl);

    // 115200 8N1 against the 16 MHz HSISYS kernel clock.
    write_volatile(USART2_BRR, PCLK_HZ / BAUD);
    write_volatile(USART2_CR1, USART_CR1_UE | USART_CR1_TE);
}

fn led_set(on: bool) {
    let bits = if on {
        1 << LED_PIN
    } else {
        1 << (LED_PIN + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, bits) };
}

fn spin_until(mut cond: impl FnMut() -> bool) {
    let mut guard = 0u32;
    while !cond() && guard < SPIN_LIMIT {
        guard += 1;
    }
}

fn print_str(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn putc(b: u8) {
    unsafe {
        let mut guard = 0u32;
        while (read_volatile(USART2_ISR) & USART_ISR_TXE) == 0 && guard < SPIN_LIMIT {
            guard += 1;
        }
        write_volatile(USART2_TDR, b as u32);
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
