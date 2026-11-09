// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

#![no_std]
#![no_main]

//! STM32F76x/F77x smoke for the imported Renode stm32f777 platform: the
//! F746 surface (`f746_checks`: USART1/2/3/6 + UART4/5/7/8, GPIOA..K,
//! EXTI, TIM1..TIM14, SPI1/2/3, I2C1..4, DMA1/DMA2, CRC, RNG) plus what the
//! F76x changes (`f767_checks`: RCC APB1/APB2 masks, SYSCFG SWP_FB, EXTI
//! line 24, 2 MiB / 512 KiB, SPI4/5/6). RM0410 and `stm32f767xx.h`.
//!
//! The fixture system is the stm32f746 one with a MAX31855 added on each of
//! SPI4/5/6. Interrupts stay disabled in the NVIC: each check reads the
//! line's ISPR bit. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-stm32f746-smoke/src/f746_checks.rs"]
mod f746_checks;
mod f767_checks;
use f746_checks::*;
use f767_checks::*;

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup_f767();
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    puts("STM32F767 Cortex-M7 smoke\r\n");
    for &(name, base, _) in &UARTS[1..] {
        send(base, name.as_bytes());
        send(base, b" ok\r\n");
    }

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate, rcc_mask));
    mark(1, check_geometry());
    mark(2, check_usart_irqs());
    mark(3, check_gpio());
    mark(4, check_syscfg_f767());
    mark(5, check_exti_swier() && check_exti_line24());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(6, timers);
    mark(7, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all.
    cs_output(GPIOB, 0);
    let mut spis = true;
    for &(name, base, irq, port, pin, want) in SPIS.iter().chain(SPIS_F767.iter()) {
        spis &= check_spi(name, base, irq, port, pin, want);
    }
    mark(8, spis);
    mark(9, check_spi1_shared_cs());
    mark(
        10,
        check_spi_dma(
            "dma2 spi1",
            SPI1,
            GPIOA,
            4,
            SPIS[0].5,
            DMA2,
            &DMA2_LINES,
            0,
            3,
            3,
            2,
        ),
    );
    mark(
        11,
        check_spi_dma(
            "dma1 spi2",
            SPI2,
            GPIOB,
            12,
            SPIS[1].5,
            DMA1,
            &DMA1_LINES,
            3,
            4,
            0,
            1,
        ),
    );
    mark(12, check_dma_m2m());
    mark(13, check_i2c1_tmp102());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(14 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(18, check_crc());
    mark(19, check_rng());
    let summary = summary | 0x7670_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
