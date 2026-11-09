#![no_std]
#![no_main]

//! STM32F746xG smoke for the imported Renode platform: RCC clock gates and
//! ENR masks, USART1/2/3/6 + UART4/5/7/8, GPIOA..K, SYSCFG EXTICR + EXTI,
//! TIM1..TIM14, SPI1/2/3 (FIFO SPI), I2C1..4 ("v2" I2C), DMA1/DMA2 (stream
//! DMA), CRC and RNG. Register offsets, bases and IRQ numbers are RM0385 and
//! ST's CMSIS `stm32f746xx.h`.
//!
//! The fixture system puts a MAX31855 on each SPI (thermocouple 25, 100 and
//! 75 °C, so each bus answers with its own frame), a second MAX31855 on SPI1
//! behind its own chip select (PB0, 50 °C) and a TMP102 at 0x48 on I2C1.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

mod f746_checks;
use f746_checks::*;

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup();
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    puts("STM32F746 Cortex-M7 smoke\r\n");
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
    mark(1, check_usart_irqs());
    mark(2, check_gpio());
    mark(3, check_syscfg());
    mark(4, check_exti_swier());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(5, timers);
    mark(6, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all (an
    // undriven CS pad leaves its device on the bus).
    cs_output(GPIOB, 0);
    for (i, &(name, base, irq, port, pin, want)) in SPIS.iter().enumerate() {
        mark(7 + i as u32, check_spi(name, base, irq, port, pin, want));
    }
    mark(10, check_spi1_shared_cs());
    mark(
        11,
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
        12,
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
    mark(13, check_dma_m2m());
    mark(14, check_i2c1_tmp102());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(15 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(19, check_crc());
    mark(20, check_rng());
    let summary = summary | 0x7460_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
