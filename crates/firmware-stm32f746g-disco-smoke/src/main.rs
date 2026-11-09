// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F746G-DISCO (32F746GDISCOVERY) smoke firmware.
//!
//! The board's STM32F746NGH6 is a Cortex-M7 with 1 MiB of flash at
//! 0x08000000 and 320 KiB of SRAM at 0x20000000. The console is USART1
//! (PA9 TX, the ST-LINK/V2-1 virtual COM port). The green user LED LD1 is
//! on PI1 and the blue user button B1 on PI11, both active high (UM1907
//! §6.5/§6.6).
//!
//! Renode builds the board as `using "platforms/cpus/stm32f746.repl"`, so
//! the row loads the stm32f746 silicon profile and this checks that surface
//! through the shared `f746_checks` module: RCC reset state, clock gate and
//! APB2ENR mask, all eight U(S)ART lines, the GPIO reset state and ports
//! up to K, SYSCFG MEMRMP/PMC/CMPCR, EXTI software triggers, TIM1..TIM14,
//! DMA2 memory-to-memory (refused on DMA1), I2C1..4 NACK lines, CRC and RNG.
//! The board IO adds LD1 and B1 (rising edge through SYSCFG EXTICR3 into
//! EXTI15_10). SPI and the TMP102 path need slaves the board does not have
//! and are proven by the stm32f746 fixture smoke instead.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's ISPR
//! bit. Every check prints one line and sets one bit of the summary word at
//! `SUMMARY`.

#![no_std]
#![no_main]

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-stm32f746-smoke/src/f746_checks.rs"]
mod f746_checks;
use f746_checks::*;

const SYSCFG_EXTICR3: u32 = SYSCFG + 0x10;
const E_RTSR: u32 = 0x08;
const CODE_PI: u32 = 8;
const EXTI15_10_IRQ: u32 = 40;
/// LD1 (green) on PI1, B1 on PI11.
const LD1: u32 = 1;
const B1: u32 = 11;

/// LD1 on PI1 as a push-pull output: three blinks through BSRR, each level
/// visible in ODR and IDR.
fn check_ld1() -> bool {
    let moder = rd(GPIOI + G_MODER) & !(0b11 << (2 * LD1));
    wr(GPIOI + G_MODER, moder | (0b01 << (2 * LD1)));
    let mut blink = true;
    for _ in 0..3 {
        wr(GPIOI + G_BSRR, 1 << LD1);
        blink &= rd(GPIOI + G_ODR) & (1 << LD1) != 0 && rd(GPIOI + G_IDR) & (1 << LD1) != 0;
        wr(GPIOI + G_BSRR, 1 << (LD1 + 16));
        blink &= rd(GPIOI + G_ODR) & (1 << LD1) == 0 && rd(GPIOI + G_IDR) & (1 << LD1) == 0;
    }
    wr(GPIOI + G_BSRR, 1 << LD1);
    let on = rd(GPIOI + G_ODR);
    let lit = on == 1 << LD1;
    wr(GPIOI + G_BSRR, 1 << (LD1 + 16));
    let off = rd(GPIOI + G_ODR) == 0;
    puts("led=ld1 odr=");
    hex(on);
    puts(" ");
    flag("blink", blink);
    flag("lit", lit);
    flag("off", off);
    let ok = blink && lit && off;
    verdict(ok);
    ok
}

/// B1 on PI11 (pressed = high; input with pull-down) through the SYSCFG
/// port mux into EXTI line 11 on the rising edge. With EXTICR3 selecting
/// port A the first press must not pend line 11; with port I the second
/// press must, on NVIC EXTI15_10 = 40 and no other EXTI vector.
fn check_b1() -> bool {
    let pupdr = rd(GPIOI + G_PUPDR) & !(0b11 << (2 * B1));
    wr(GPIOI + G_PUPDR, pupdr | (0b10 << (2 * B1)));
    let moder = rd(GPIOI + G_MODER) & !(0b11 << (2 * B1));
    wr(GPIOI + G_MODER, moder);
    wr(SYSCFG_EXTICR3, CODE_PA << 12);
    wr(EXTI + E_RTSR, 1 << B1);
    wr(EXTI + E_IMR, 1 << B1);
    unpend(EXTI15_10_IRQ);

    let idle = wait_level(GPIOI, B1, false);
    let press1 = wait_level(GPIOI, B1, true) && wait_level(GPIOI, B1, false);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);

    wr(SYSCFG_EXTICR3, CODE_PI << 12);
    let press2 = wait_level(GPIOI, B1, true) && wait_level(GPIOI, B1, false);
    let pi11 = rd(EXTI + E_PR) & (1 << B1) != 0
        && pending(EXTI15_10_IRQ)
        && EXTI_LINES
            .iter()
            .all(|&o| o == EXTI15_10_IRQ || !pending(o));
    wr(EXTI + E_PR, 1 << B1);
    unpend(EXTI15_10_IRQ);
    let cleared = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_RTSR, 0);
    wr(SYSCFG_EXTICR3, 0);

    puts("b1 ");
    flag("idle", idle);
    flag("presses", press1 && press2);
    flag("pa_quiet", other_port_quiet);
    flag("pi11", pi11);
    flag("cleared", cleared);
    let ok = idle && press1 && press2 && other_port_quiet && pi11 && cleared;
    verdict(ok);
    ok
}

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup();
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    puts("STM32F746G-DISCO SMOKE\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate, rcc_mask));
    mark(1, check_usart_irqs());
    mark(2, check_gpio());
    mark(3, check_ld1());
    mark(4, check_syscfg());
    mark(5, check_exti_swier());
    mark(6, check_b1());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(7, timers);
    mark(8, check_dma_m2m());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(9 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(13, check_crc());
    mark(14, check_rng());
    let summary = summary | 0x7DC0_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
