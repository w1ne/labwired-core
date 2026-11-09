// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-F767ZI smoke firmware.
//!
//! The board's STM32F767ZIT6 is a Cortex-M7 with 2 MiB of flash and 512 KiB
//! of SRAM. The console here is USART1 (the board routes the ST-LINK VCP to
//! USART3 on PD8/PD9; every U(S)ART is brought up and USART3 prints too).
//! LD1 (green) PB0, LD2 (blue) PB7 and LD3 (red) PB14 are active high, and
//! B1 USER on PC13 is active high (UM1974 §6.5/§6.6).
//!
//! Renode builds the board as `using "platforms/cpus/stm32f777.repl"`, so
//! the row loads the stm32f777 silicon profile and this checks that surface
//! through the shared `f746_checks` / `f767_checks` modules: RCC reset state,
//! clock gate and the F767 APB1/APB2 masks, the 512 KiB / 2 MiB geometry,
//! all eight U(S)ART lines, GPIO reset state, SYSCFG (SWP_FB), EXTI software
//! triggers and line 24, TIM1..14, SPI4/5/6 interrupt lines, DMA2
//! memory-to-memory, I2C1..4 NACK lines, CRC and RNG. The board IO adds
//! the three LEDs and B1 (rising edge through SYSCFG EXTICR4 into
//! EXTI15_10). SPI frames, SPI DMA and the TMP102 need slaves the board does
//! not have and are proven by the stm32f777 fixture smoke instead.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's ISPR
//! bit. Every check prints one line and sets one bit of the summary word at
//! `SUMMARY`.

#![no_std]
#![no_main]

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../../firmware-stm32f746-smoke/src/f746_checks.rs"]
mod f746_checks;
#[path = "../f767_checks.rs"]
mod f767_checks;
use f746_checks::*;
use f767_checks::*;

const E_RTSR: u32 = 0x08;
const EXTI15_10_IRQ: u32 = 40;
/// LD1 (green) PB0, LD2 (blue) PB7, LD3 (red) PB14.
const LEDS: [(&str, u32); 3] = [("ld1", 0), ("ld2", 7), ("ld3", 14)];
/// B1 USER on PC13.
const B1: u32 = 13;

/// Each LED as a push-pull output: three blinks through BSRR, each level
/// visible in ODR and IDR, and only its own ODR bit set while lit.
fn check_leds() -> bool {
    let mut ok = true;
    puts("leds ");
    for &(name, pin) in &LEDS {
        let moder = rd(GPIOB + G_MODER) & !(0b11 << (2 * pin));
        wr(GPIOB + G_MODER, moder | (0b01 << (2 * pin)));
        let mut blink = true;
        for _ in 0..3 {
            wr(GPIOB + G_BSRR, 1 << pin);
            blink &= rd(GPIOB + G_ODR) & (1 << pin) != 0 && rd(GPIOB + G_IDR) & (1 << pin) != 0;
            wr(GPIOB + G_BSRR, 1 << (pin + 16));
            blink &= rd(GPIOB + G_ODR) & (1 << pin) == 0 && rd(GPIOB + G_IDR) & (1 << pin) == 0;
        }
        wr(GPIOB + G_BSRR, 1 << pin);
        let lit = rd(GPIOB + G_ODR) == 1 << pin;
        wr(GPIOB + G_BSRR, 1 << (pin + 16));
        let this = blink && lit && rd(GPIOB + G_ODR) == 0;
        flag(name, this);
        ok &= this;
    }
    verdict(ok);
    ok
}

/// B1 on PC13 (pressed = high; input with pull-down) through the SYSCFG
/// port mux into EXTI line 13 on the rising edge. With EXTICR4 selecting
/// port A the first press must not pend line 13; with port C the second
/// press must, on NVIC EXTI15_10 = 40 and no other EXTI vector.
fn check_b1() -> bool {
    let pupdr = rd(GPIOC + G_PUPDR) & !(0b11 << (2 * B1));
    wr(GPIOC + G_PUPDR, pupdr | (0b10 << (2 * B1)));
    let moder = rd(GPIOC + G_MODER) & !(0b11 << (2 * B1));
    wr(GPIOC + G_MODER, moder);
    wr(SYSCFG_EXTICR4, CODE_PA << 4);
    wr(EXTI + E_RTSR, 1 << B1);
    wr(EXTI + E_IMR, 1 << B1);
    unpend(EXTI15_10_IRQ);

    let idle = wait_level(GPIOC, B1, false);
    let press1 = wait_level(GPIOC, B1, true) && wait_level(GPIOC, B1, false);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);

    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    let press2 = wait_level(GPIOC, B1, true) && wait_level(GPIOC, B1, false);
    let pc13 = rd(EXTI + E_PR) & (1 << B1) != 0
        && pending(EXTI15_10_IRQ)
        && EXTI_LINES
            .iter()
            .all(|&o| o == EXTI15_10_IRQ || !pending(o));
    wr(EXTI + E_PR, 1 << B1);
    unpend(EXTI15_10_IRQ);
    let cleared = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_RTSR, 0);
    wr(SYSCFG_EXTICR4, 0);

    puts("b1 ");
    flag("idle", idle);
    flag("presses", press1 && press2);
    flag("pa_quiet", other_port_quiet);
    flag("pc13", pc13);
    flag("cleared", cleared);
    let ok = idle && press1 && press2 && other_port_quiet && pc13 && cleared;
    verdict(ok);
    ok
}

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup_f767();
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    puts("NUCLEO-F767ZI SMOKE\r\n");
    send(UARTS[2].1, b"usart3 vcp ok\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate, rcc_mask));
    mark(1, check_geometry());
    mark(2, check_usart_irqs());
    mark(3, check_leds());
    mark(4, check_syscfg_f767());
    mark(5, check_exti_swier() && check_exti_line24());
    mark(6, check_b1());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(7, timers);
    mark(8, check_spi456_lines());
    mark(9, check_dma_m2m());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(10 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(14, check_crc());
    mark(15, check_rng());
    let summary = summary | 0x7670_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
