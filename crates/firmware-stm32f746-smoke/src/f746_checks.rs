// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The STM32F746xG surface checks, shared by the stm32f746 fixture smoke
//! (`main.rs`) and the STM32F746G-DISCO board smoke
//! (`firmware-stm32f746g-disco-smoke`, which includes this file by path).
//! Register offsets, bases and IRQ numbers are RM0385 and ST's CMSIS
//! `stm32f746xx.h`. The SPI frame, shared-CS, SPI-DMA, TMP102 and fixture
//! button checks need the stm32f746 fixture system's slaves and switches;
//! the board smoke does not call them.

#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

pub const SUMMARY: u32 = 0x2000_3000;

// RCC (RM0385 §5.3).
pub const RCC: u32 = 0x4002_3800;
pub const RCC_CR: u32 = RCC;
pub const RCC_AHB1ENR: u32 = RCC + 0x30;
pub const RCC_AHB2ENR: u32 = RCC + 0x34;
pub const RCC_APB1ENR: u32 = RCC + 0x40;
pub const RCC_APB2ENR: u32 = RCC + 0x44;
/// AHB1ENR reset value: DTCMRAMEN.
pub const AHB1_RESET: u32 = 1 << 20;
/// Every APB2ENR enable bit the F746 has (`stm32f746xx.h` RCC_APB2ENR_*).
pub const APB2_MASK: u32 = 0x04F7_7F33;
// GPIOA..K, CRC, DTCMRAM, DMA1, DMA2.
pub const AHB1_ON: u32 = 0x7FF | (1 << 12) | (1 << 20) | (1 << 21) | (1 << 22);
// RNG.
pub const AHB2_ON: u32 = 1 << 6;
// TIM2..7/12..14, SPI2/3, USART2/3, UART4/5, I2C1..4, UART7/8.
pub const APB1_ON: u32 = 0x1FF | (0b11 << 14) | (0xF << 17) | (0xF << 21) | (0b11 << 30);
// TIM1, TIM8, USART1, USART6, SPI1, SYSCFG, TIM9..11.
pub const APB2_ON: u32 = 0b11 | (0b11 << 4) | (1 << 12) | (1 << 14) | (0b111 << 16);

// USART "v2" layout.
pub const USART1: u32 = 0x4001_1000;
pub const U_CR1: u32 = 0x00;
pub const U_BRR: u32 = 0x0C;
pub const U_ISR: u32 = 0x1C;
pub const U_TDR: u32 = 0x28;
pub const USART_ISR_TXE: u32 = 1 << 7;
pub const USART_CR1_TXEIE: u32 = 1 << 7;
/// (name, base, NVIC line) — `stm32f746xx.h` *_BASE / *_IRQn.
pub const UARTS: [(&str, u32, u32); 8] = [
    ("usart1", USART1, 37),
    ("usart2", 0x4000_4400, 38),
    ("usart3", 0x4000_4800, 39),
    ("uart4", 0x4000_4C00, 52),
    ("uart5", 0x4000_5000, 53),
    ("usart6", 0x4001_1400, 71),
    ("uart7", 0x4000_7800, 82),
    ("uart8", 0x4000_7C00, 83),
];

// GPIO (AHB1), A..K at a 0x400 stride.
pub const fn gpio(n: u32) -> u32 {
    0x4002_0000 + 0x400 * n
}
pub const GPIOA: u32 = gpio(0);
pub const GPIOB: u32 = gpio(1);
pub const GPIOC: u32 = gpio(2);
pub const GPIOD: u32 = gpio(3);
pub const GPIOI: u32 = gpio(8);
pub const GPIOJ: u32 = gpio(9);
pub const GPIOK: u32 = gpio(10);
pub const G_MODER: u32 = 0x00;
pub const G_OSPEEDR: u32 = 0x08;
pub const G_PUPDR: u32 = 0x0C;
pub const G_IDR: u32 = 0x10;
pub const G_ODR: u32 = 0x14;
pub const G_BSRR: u32 = 0x18;

// SYSCFG / EXTI.
pub const SYSCFG: u32 = 0x4001_3800;
pub const SYSCFG_MEMRMP: u32 = SYSCFG;
pub const SYSCFG_PMC: u32 = SYSCFG + 0x04;
pub const SYSCFG_EXTICR1: u32 = SYSCFG + 0x08;
pub const SYSCFG_EXTICR4: u32 = SYSCFG + 0x14;
pub const SYSCFG_CMPCR: u32 = SYSCFG + 0x20;
pub const EXTI: u32 = 0x4001_3C00;
pub const E_IMR: u32 = 0x00;
pub const E_FTSR: u32 = 0x0C;
pub const E_SWIER: u32 = 0x10;
pub const E_PR: u32 = 0x14;
// SYSCFG_EXTICRx port codes (stm32f746xx.h SYSCFG_EXTICR1_EXTI0_Px).
pub const CODE_PA: u32 = 0;
pub const CODE_PC: u32 = 2;
pub const CODE_PK: u32 = 10;
/// EXTI0..4, EXTI9_5, EXTI15_10.
pub const EXTI_LINES: [u32; 7] = [6, 7, 8, 9, 10, 23, 40];

// TIMx.
pub const T_CR1: u32 = 0x00;
pub const T_DIER: u32 = 0x0C;
pub const T_SR: u32 = 0x10;
pub const T_CNT: u32 = 0x24;
pub const T_PSC: u32 = 0x28;
pub const T_ARR: u32 = 0x2C;

/// (name, base, NVIC line of the update interrupt, counter width in bits).
/// TIM1 updates on TIM1_UP_TIM10_IRQn (25), TIM8 on TIM8_UP_TIM13_IRQn (44).
pub const TIMERS: [(&str, u32, u32, u32); 14] = [
    ("tim1", 0x4001_0000, 25, 16),
    ("tim2", 0x4000_0000, 28, 32),
    ("tim3", 0x4000_0400, 29, 16),
    ("tim4", 0x4000_0800, 30, 16),
    ("tim5", 0x4000_0C00, 50, 32),
    ("tim6", 0x4000_1000, 54, 16),
    ("tim7", 0x4000_1400, 55, 16),
    ("tim8", 0x4001_0400, 44, 16),
    ("tim9", 0x4001_4000, 24, 16),
    ("tim10", 0x4001_4400, 25, 16),
    ("tim11", 0x4001_4800, 26, 16),
    ("tim12", 0x4000_1800, 43, 16),
    ("tim13", 0x4000_1C00, 44, 16),
    ("tim14", 0x4000_2000, 45, 16),
];

// SPIx (FIFO SPI: CR2.DS[11:8], FRXTH bit 12; DR is accessed by byte).
pub const S_CR1: u32 = 0x00;
pub const S_CR2: u32 = 0x04;
pub const S_SR: u32 = 0x08;
pub const S_DR: u32 = 0x0C;
pub const SPI_SR_RXNE: u32 = 1 << 0;
pub const SPI_SR_TXE: u32 = 1 << 1;
pub const SPI_SR_BSY: u32 = 1 << 7;
pub const SPI_SR_FRLVL: u32 = 0b11 << 9;
/// FRXTH (RXNE at one byte) | DS = 8 bits.
pub const SPI_CR2_8BIT: u32 = (1 << 12) | (0b0111 << 8);
/// CR2 reset value: DS = 8 bits (RM0385 §32.9.2).
pub const SPI_CR2_RESET: u32 = 0x0700;
pub const SPI1: u32 = 0x4001_3000;
pub const SPI2: u32 = 0x4000_3800;

/// (name, base, NVIC line, CS port, CS pin, expected MAX31855 frame).
/// Frames: thermocouple << 18 (0.25 °C/LSB) | internal 22 °C << 4.
/// The second SPI1 slave (CS PB0): 50 °C thermocouple, 22 °C cold junction.
pub const SPI1_B_FRAME: u32 = 0x0320_1600;
pub const SPIS: [(&str, u32, u32, u32, u32, u32); 3] = [
    ("spi1", SPI1, 35, GPIOA, 4, 0x0190_1600),
    ("spi2", SPI2, 36, GPIOB, 12, 0x0640_1600),
    ("spi3", 0x4000_3C00, 51, GPIOD, 2, 0x04B0_1600),
];

// Stream DMA (RM0385 §8.5): LISR/HISR/LIFCR/HIFCR, stream n at 0x10 + 0x18 n.
pub const DMA1: u32 = 0x4002_6000;
pub const DMA2: u32 = 0x4002_6400;
pub const D_LISR: u32 = 0x00;
pub const D_HISR: u32 = 0x04;
pub const D_LIFCR: u32 = 0x08;
pub const D_HIFCR: u32 = 0x0C;
pub const fn stream(dma: u32, n: u32) -> u32 {
    dma + 0x10 + 0x18 * n
}
pub const X_CR: u32 = 0x00;
pub const X_NDTR: u32 = 0x04;
pub const X_PAR: u32 = 0x08;
pub const X_M0AR: u32 = 0x0C;
pub const X_CR_EN: u32 = 1 << 0;
pub const X_CR_TCIE: u32 = 1 << 4;
pub const X_CR_M2P: u32 = 0b01 << 6;
pub const X_CR_M2M: u32 = 0b10 << 6;
pub const X_CR_PINC: u32 = 1 << 9;
pub const X_CR_MINC: u32 = 1 << 10;
pub const X_CR_WORDS: u32 = (0b10 << 11) | (0b10 << 13); // PSIZE = MSIZE = 32
pub const fn chsel(n: u32) -> u32 {
    n << 25
}
/// Bit position of stream `n`'s flag field in LISR/HISR (RM0385 §8.5.1).
pub const fn shift(n: u32) -> u32 {
    [0, 6, 16, 22][(n % 4) as usize]
}
/// Stream `n`'s FEIF | DMEIF | TEIF | HTIF | TCIF.
pub const fn sflags(n: u32) -> u32 {
    0x3D << shift(n)
}
/// HTIF | TCIF of stream `n`.
pub const fn sdone(n: u32) -> u32 {
    0x30 << shift(n)
}
pub const DMA_BUF: u32 = 0x2000_3100;
pub const DMA1_LINES: [u32; 8] = [11, 12, 13, 14, 15, 16, 17, 47];
pub const DMA2_LINES: [u32; 8] = [56, 57, 58, 59, 60, 68, 69, 70];

// I2C ("v2" IP).
pub const I2C1: u32 = 0x4000_5400;
pub const I_CR1: u32 = 0x00;
pub const I_CR2: u32 = 0x04;
pub const I_OAR1: u32 = 0x08;
pub const I_TIMINGR: u32 = 0x10;
pub const I_ISR: u32 = 0x18;
pub const I_ICR: u32 = 0x1C;
pub const I_RXDR: u32 = 0x24;
pub const I_TXDR: u32 = 0x28;
pub const I2C_ISR_TXIS: u32 = 1 << 1;
pub const I2C_ISR_RXNE: u32 = 1 << 2;
pub const I2C_ISR_NACKF: u32 = 1 << 4;
pub const I2C_ISR_STOPF: u32 = 1 << 5;
pub const I2C_ISR_TC: u32 = 1 << 6;
pub const I2C_ISR_BUSY: u32 = 1 << 15;
pub const I2C_CR1_PE: u32 = 1 << 0;
pub const I2C_CR1_RXIE: u32 = 1 << 2;
pub const I2C_CR1_NACKIE: u32 = 1 << 4;
pub const I2C_CR2_RD_WRN: u32 = 1 << 10;
pub const I2C_CR2_START: u32 = 1 << 13;
pub const I2C_CR2_AUTOEND: u32 = 1 << 25;
pub const TMP102: u32 = 0x48;
/// (name, base, event line, error line).
pub const I2CS: [(&str, u32, u32, u32); 4] = [
    ("i2c1", I2C1, 31, 32),
    ("i2c2", 0x4000_5800, 33, 34),
    ("i2c3", 0x4000_5C00, 72, 73),
    ("i2c4", 0x4000_6000, 95, 96),
];

// CRC.
pub const CRC: u32 = 0x4002_3000;
pub const CRC_DR: u32 = CRC;
pub const CRC_IDR: u32 = CRC + 0x04;
pub const CRC_CR: u32 = CRC + 0x08;
pub const CRC_INIT: u32 = CRC + 0x10;
pub const CRC_POL: u32 = CRC + 0x14;

// RNG.
pub const RNG: u32 = 0x5006_0800;
pub const RNG_CR: u32 = RNG;
pub const RNG_SR: u32 = RNG + 0x04;
pub const RNG_DR: u32 = RNG + 0x08;

pub const NVIC_ISPR: u32 = 0xE000_E200;
pub const NVIC_ICPR: u32 = 0xE000_E280;

pub fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

pub fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

pub fn rd8(addr: u32) -> u8 {
    unsafe { read_volatile(addr as *const u8) }
}

pub fn wr8(addr: u32, value: u8) {
    unsafe { write_volatile(addr as *mut u8, value) }
}

pub fn pending(irq: u32) -> bool {
    rd(NVIC_ISPR + 4 * (irq / 32)) & (1 << (irq % 32)) != 0
}

pub fn unpend(irq: u32) {
    wr(NVIC_ICPR + 4 * (irq / 32), 1 << (irq % 32));
}

pub fn uart_init(base: u32) {
    wr(base + U_BRR, 0x8B); // 115200 Bd from the 16 MHz HSI
    wr(base + U_CR1, (1 << 3) | 1); // TE | UE
}

pub fn send(base: u32, bytes: &[u8]) {
    for byte in bytes {
        let mut remaining = 100_000;
        while rd(base + U_ISR) & USART_ISR_TXE == 0 && remaining > 0 {
            remaining -= 1;
        }
        wr(base + U_TDR, u32::from(*byte));
    }
}

pub fn puts(s: &str) {
    send(USART1, s.as_bytes());
}

pub fn flag(name: &str, ok: bool) {
    puts(name);
    puts(if ok { "=1 " } else { "=0 " });
}

pub fn verdict(ok: bool) {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
}

pub fn hex(v: u32) {
    let digits = b"0123456789ABCDEF";
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = digits[((v >> (28 - 4 * i)) & 0xF) as usize];
    }
    send(USART1, &buf);
}

/// RCC reset state (HSI on and ready, AHB1ENR = DTCMRAMEN), a clock gate
/// that holds (GPIOD ignores ODR writes until GPIODEN is set) and the
/// APB2ENR write mask (reserved bits stay 0). Runs before anything else, so
/// USART1 is still gated and the result is reported later.
pub fn rcc_bringup() -> (bool, bool, bool) {
    let reset = rd(RCC_CR) & 0b11 == 0b11 && rd(RCC_AHB1ENR) == AHB1_RESET;
    wr(GPIOD + G_ODR, 0x00A5);
    let gated = rd(GPIOD + G_ODR) == 0;
    wr(RCC_APB2ENR, 0xFFFF_FFFF);
    let mask = rd(RCC_APB2ENR) == APB2_MASK;
    wr(RCC_AHB1ENR, AHB1_ON);
    wr(RCC_AHB2ENR, AHB2_ON);
    wr(RCC_APB2ENR, APB2_ON);
    wr(RCC_APB1ENR, APB1_ON);
    wr(GPIOD + G_ODR, 0x00A5);
    let gate = gated && rd(GPIOD + G_ODR) == 0x00A5 && rd(RCC_AHB1ENR) == AHB1_ON;
    wr(GPIOD + G_ODR, 0);
    (reset, gate, mask)
}

pub fn report_rcc(reset: bool, gate: bool, mask: bool) -> bool {
    puts("rcc ");
    flag("reset", reset);
    flag("gate", gate);
    flag("mask", mask);
    let ok = reset && gate && mask;
    verdict(ok);
    ok
}

/// TXEIE on each USART/UART raises its own NVIC line and no other UART's.
pub fn check_usart_irqs() -> bool {
    let mut ok = true;
    puts("usart irq ");
    for &(name, base, irq) in &UARTS {
        unpend(irq);
        let quiet = !pending(irq);
        wr(base + U_CR1, rd(base + U_CR1) | USART_CR1_TXEIE);
        let pend = pending(irq);
        let alone = UARTS
            .iter()
            .all(|&(_, _, other)| other == irq || !pending(other));
        wr(base + U_CR1, rd(base + U_CR1) & !USART_CR1_TXEIE);
        unpend(irq);
        let released = !pending(irq);
        let this = quiet && pend && alone && released;
        flag(name, this);
        ok &= this;
    }
    verdict(ok);
    ok
}

/// RM0385 reset values of the JTAG/SWD pads on ports A and B, the other
/// ports at zero, set/reset through BSRR on PI1, and per-port ODRs.
pub fn check_gpio() -> bool {
    let pa = rd(GPIOA + G_MODER) == 0xA800_0000 && rd(GPIOA + G_PUPDR) == 0x6400_0000;
    let pb = rd(GPIOB + G_MODER) == 0x0000_0280
        && rd(GPIOB + G_OSPEEDR) == 0x0000_00C0
        && rd(GPIOB + G_PUPDR) == 0x0000_0100;
    let others = (2..=10).all(|n| rd(gpio(n) + G_MODER) == 0 && rd(gpio(n) + G_PUPDR) == 0);
    // PI1 output; toggle it twice for the gpio_edges assertion.
    wr(GPIOI + G_MODER, 0b01 << 2);
    let mut toggles = true;
    for _ in 0..2 {
        wr(GPIOI + G_BSRR, 1 << 1);
        toggles &= rd(GPIOI + G_ODR) & 2 == 2 && rd(GPIOI + G_IDR) & 2 == 2;
        wr(GPIOI + G_BSRR, 1 << 17);
        toggles &= rd(GPIOI + G_ODR) & 2 == 0;
    }
    // Each port keeps its own ODR, up to port K.
    wr(GPIOJ + G_ODR, 0x00F0);
    wr(GPIOK + G_ODR, 0x000F);
    let jk = rd(GPIOJ + G_ODR) == 0x00F0 && rd(GPIOK + G_ODR) == 0x000F && rd(GPIOI + G_ODR) == 0;
    wr(GPIOJ + G_ODR, 0);
    wr(GPIOK + G_ODR, 0);
    puts("gpio ");
    flag("pa", pa);
    flag("pb", pb);
    flag("reset", others);
    flag("pi1", toggles);
    flag("jk", jk);
    let ok = pa && pb && others && toggles && jk;
    verdict(ok);
    ok
}

/// SYSCFG: the F7 MEMRMP (SWP_FMC only; MEM_BOOT reads 0) and PMC fields,
/// and CMPCR.READY following CMP_PD.
pub fn check_syscfg() -> bool {
    wr(SYSCFG_MEMRMP, 0xFFFF_FFFF);
    let memrmp = rd(SYSCFG_MEMRMP);
    wr(SYSCFG_MEMRMP, 0);
    wr(SYSCFG_PMC, 0xFFFF_FFFF);
    let pmc = rd(SYSCFG_PMC);
    wr(SYSCFG_PMC, 0);
    let fields = memrmp == 0x0C00 && pmc == 0x0087_00FF;
    let idle = rd(SYSCFG_CMPCR) & (1 << 8) == 0;
    wr(SYSCFG_CMPCR, 1);
    let ready = idle && rd(SYSCFG_CMPCR) == 0x101;
    wr(SYSCFG_CMPCR, 0);
    puts("syscfg memrmp=");
    hex(memrmp);
    puts(" pmc=");
    hex(pmc);
    puts(" ");
    flag("fields", fields);
    flag("cmpcr", ready);
    let ok = fields && ready;
    verdict(ok);
    ok
}

/// SWIER on lines 1, 3, 7 and 12 reaches EXTI1 = 7, EXTI3 = 9,
/// EXTI9_5 = 23 and EXTI15_10 = 40, and only while unmasked.
pub fn check_exti_swier() -> bool {
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_SWIER, 1 << 1);
    let masked = rd(EXTI + E_PR) & (1 << 1) != 0 && !pending(7);
    wr(EXTI + E_PR, 1 << 1);
    unpend(7);
    let mut routed = true;
    for &(line, irq) in &[(1u32, 7u32), (3, 9), (7, 23), (12, 40)] {
        wr(EXTI + E_IMR, 1 << line);
        wr(EXTI + E_SWIER, 1 << line);
        routed &= rd(EXTI + E_PR) & (1 << line) != 0
            && pending(irq)
            && EXTI_LINES.iter().all(|&o| o == irq || !pending(o));
        wr(EXTI + E_PR, 1 << line);
        unpend(irq);
        routed &= rd(EXTI + E_PR) & (1 << line) == 0 && !pending(irq);
    }
    wr(EXTI + E_IMR, 0);
    puts("exti swier ");
    flag("masked", masked);
    flag("routed", routed);
    let ok = masked && routed;
    verdict(ok);
    ok
}

pub fn wait_level(port: u32, pin: u32, high: bool) -> bool {
    for _ in 0..2_000_000 {
        if (rd(port + G_IDR) >> pin) & 1 == u32::from(high) {
            return true;
        }
    }
    false
}

/// Falling edges from the two fixture buttons (PK3, PC13; pressed = low)
/// through the SYSCFG port mux. Line 3 first selects code 0 (PA): the first
/// PK3 press must not pend it. With code 10 (PK) the second press must, on
/// NVIC EXTI3 = 9. PC13 lands on EXTI15_10 = 40.
pub fn check_exti_pads() -> bool {
    // Inputs with pull-up.
    wr(GPIOK + G_PUPDR, 0b01 << 6);
    wr(GPIOC + G_PUPDR, 0b01 << 26);
    wr(SYSCFG_EXTICR1, CODE_PA << 12);
    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    wr(EXTI + E_FTSR, (1 << 3) | (1 << 13));
    wr(EXTI + E_IMR, (1 << 3) | (1 << 13));

    let idle = wait_level(GPIOK, 3, true) && wait_level(GPIOC, 13, true);
    let press1 = wait_level(GPIOK, 3, false) && wait_level(GPIOK, 3, true);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << 3) == 0 && !pending(9);

    wr(SYSCFG_EXTICR1, CODE_PK << 12);
    let press2 = wait_level(GPIOK, 3, false) && wait_level(GPIOK, 3, true);
    let pk = rd(EXTI + E_PR) & (1 << 3) != 0 && pending(9);
    wr(EXTI + E_PR, 1 << 3);
    unpend(9);

    let press3 = wait_level(GPIOC, 13, false) && wait_level(GPIOC, 13, true);
    let pc = rd(EXTI + E_PR) & (1 << 13) != 0 && pending(40);
    wr(EXTI + E_PR, 1 << 13);
    unpend(40);
    wr(EXTI + E_IMR, 0);

    puts("exti pads ");
    flag("idle", idle);
    flag("presses", press1 && press2 && press3);
    flag("pa_quiet", other_port_quiet);
    flag("pk3", pk);
    flag("pc13", pc);
    let ok = idle && press1 && press2 && press3 && other_port_quiet && pk && pc;
    verdict(ok);
    ok
}

/// Counter width (ARR keeps 16 or 32 bits), counting, and the update
/// interrupt reaching the timer's own NVIC line — and no other timer's.
pub fn check_timer(name: &str, base: u32, irq: u32, width: u32) -> bool {
    wr(base + T_ARR, 0xFFFF_FFFF);
    let top = if width == 32 { 0xFFFF_FFFF } else { 0xFFFF };
    let arr = rd(base + T_ARR);
    let width_ok = arr == top;

    wr(base + T_PSC, 0);
    wr(base + T_ARR, 999);
    wr(base + T_SR, 0);
    unpend(irq);
    wr(base + T_DIER, 1); // UIE
    wr(base + T_CR1, 1); // CEN
    let c0 = rd(base + T_CNT);
    let mut uif = false;
    for _ in 0..200_000 {
        if rd(base + T_SR) & 1 != 0 {
            uif = true;
            break;
        }
    }
    let counted = rd(base + T_CNT) != c0 || uif;
    let pend = pending(irq);
    // No other timer line is pending.
    let alone = TIMERS
        .iter()
        .all(|&(_, _, other, _)| other == irq || !pending(other));
    wr(base + T_CR1, 0);
    wr(base + T_DIER, 0);
    wr(base + T_SR, 0);
    unpend(irq);
    let released = !pending(irq);

    puts(name);
    puts(" arr=");
    hex(arr);
    puts(" ");
    flag("width", width_ok);
    flag("count", counted);
    flag("uif", uif);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = width_ok && counted && uif && pend && alone && released;
    verdict(ok);
    ok
}

pub fn wait(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if rd(addr) & mask == want {
            return true;
        }
    }
    false
}

pub fn cs(port: u32, pin: u32, low: bool) {
    wr(port + G_BSRR, if low { 1 << (pin + 16) } else { 1 << pin });
}

pub fn cs_output(port: u32, pin: u32) {
    cs(port, pin, false);
    let moder = rd(port + G_MODER) & !(0b11 << (2 * pin));
    wr(port + G_MODER, moder | (0b01 << (2 * pin)));
}

/// Master, software NSS (SSM | SSI), fPCLK/8, mode 0, 8-bit frames with
/// the RXNE threshold at one byte.
pub fn spi_enable(base: u32) {
    wr(base + S_CR2, SPI_CR2_8BIT);
    wr(base + S_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(base + S_CR1, rd(base + S_CR1) | (1 << 6)); // SPE
}

pub fn spi_xfer(base: u32, out: u8) -> Option<u8> {
    if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
        return None;
    }
    wr8(base + S_DR, out);
    if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
        return None;
    }
    Some(rd8(base + S_DR))
}

/// Transfer one byte and sample the NVIC line while RXNE is still asserted.
/// STM32 SPI's RXNE interrupt is level-sensitive: after the data-register
/// read drains the FIFO, the source line drops and the pending bit may clear.
pub fn spi_xfer_observe_irq(base: u32, out: u8, irq: u32) -> Option<(u8, bool)> {
    if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
        return None;
    }
    wr8(base + S_DR, out);
    if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
        return None;
    }
    let irq_pending_while_rxne = pending(irq);
    Some((rd8(base + S_DR), irq_pending_while_rxne))
}

/// One 32-bit MAX31855 frame under CS, by polling.
pub fn max31855_frame(base: u32, port: u32, pin: u32) -> Option<u32> {
    cs(port, pin, true);
    let mut word = 0u32;
    for _ in 0..4 {
        match spi_xfer(base, 0) {
            Some(b) => word = (word << 8) | u32::from(b),
            None => {
                cs(port, pin, false);
                return None;
            }
        }
    }
    let idle = wait(base + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    idle.then_some(word)
}

/// One MAX31855 frame while observing the RXNE interrupt before each read
/// drains the level-sensitive source.
pub fn max31855_frame_observe_irq(base: u32, port: u32, pin: u32, irq: u32) -> Option<(u32, bool)> {
    cs(port, pin, true);
    let mut word = 0u32;
    let mut irq_seen = false;
    for _ in 0..4 {
        match spi_xfer_observe_irq(base, 0, irq) {
            Some((byte, pending)) => {
                word = (word << 8) | u32::from(byte);
                irq_seen |= pending;
            }
            None => {
                cs(port, pin, false);
                return None;
            }
        }
    }
    let idle = wait(base + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    idle.then_some((word, irq_seen))
}

/// Each SPI resets CR2 to DS = 8 bits, then reads its own device's frame
/// twice — polled, then with RXNEIE. The second transfer samples the SPI's
/// NVIC line while RXNE is asserted; after the FIFO drains the level source
/// releases. No other SPI line may pend, and the FIFO must be empty (FRLVL=0).
pub fn check_spi(name: &str, base: u32, irq: u32, port: u32, pin: u32, want: u32) -> bool {
    let cr2_reset = rd(base + S_CR2) == SPI_CR2_RESET;
    cs_output(port, pin);
    spi_enable(base);
    let ds = cr2_reset && rd(base + S_CR2) & 0x1F00 == SPI_CR2_8BIT;
    let frame = max31855_frame(base, port, pin).unwrap_or(0xDEAD_BEEF);
    let quiet = !pending(irq);

    unpend(irq);
    wr(base + S_CR2, SPI_CR2_8BIT | (1 << 6)); // RXNEIE
    let again = max31855_frame_observe_irq(base, port, pin, irq);
    let data = frame == want && again.map(|(word, _)| word) == Some(want);
    let pend = quiet && again.is_some_and(|(_, seen)| seen);
    let alone = SPIS
        .iter()
        .all(|&(_, _, other, _, _, _)| other == irq || !pending(other));
    wr(base + S_CR2, SPI_CR2_8BIT);
    unpend(irq);
    let released = !pending(irq) && rd(base + S_SR) & (SPI_SR_RXNE | SPI_SR_FRLVL) == 0;

    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("ds", ds);
    flag("data", data);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = ds && data && pend && alone && released;
    verdict(ok);
    ok
}

/// Two MAX31855s share SPI1, told apart only by their GPIO chip selects
/// (PA4, PB0): each answers with its own frame while the other is released.
/// Then a frame cut short by CS↑ after two bytes: the next CS↓ must restart
/// the MAX31855 at bit 31, so the following whole frame is not rotated.
pub fn check_spi1_shared_cs() -> bool {
    let a = max31855_frame(SPI1, GPIOA, 4).unwrap_or(0xDEAD_BEEF);
    let b = max31855_frame(SPI1, GPIOB, 0).unwrap_or(0xDEAD_BEEF);
    let a_ok = a == SPIS[0].5;
    let b_ok = b == SPI1_B_FRAME;

    cs(GPIOA, 4, true);
    let _ = spi_xfer(SPI1, 0);
    let _ = spi_xfer(SPI1, 0);
    let _ = wait(SPI1 + S_SR, SPI_SR_BSY, 0);
    cs(GPIOA, 4, false);
    let after = max31855_frame(SPI1, GPIOA, 4).unwrap_or(0xDEAD_BEEF);
    let restart = after == SPIS[0].5;

    puts("spi1 shared a=");
    hex(a);
    puts(" b=");
    hex(b);
    puts(" after_cut=");
    hex(after);
    puts(" ");
    flag("a", a_ok);
    flag("b", b_ok);
    flag("restart", restart);
    let ok = a_ok && b_ok && restart;
    verdict(ok);
    ok
}

pub fn flag_reg(dma: u32, n: u32) -> u32 {
    dma + if n < 4 { D_LISR } else { D_HISR }
}

pub fn clear_flags(dma: u32) {
    wr(dma + D_LIFCR, 0x0F7D_0F7D);
    wr(dma + D_HIFCR, 0x0F7D_0F7D);
}

pub fn stream_off(dma: u32, n: u32) -> bool {
    let s = stream(dma, n);
    wr(s + X_CR, 0);
    wait(s + X_CR, X_CR_EN, 0)
}

/// One SPI frame through a pair of DMA streams with SxCR.CHSEL = `ch`: RX
/// stream `rx` (TCIE) stores four bytes, TX stream `tx` feeds four zeros.
/// Returns the frame and whether RX completed.
pub fn spi_dma_frame(
    spi: u32,
    port: u32,
    pin: u32,
    dma: u32,
    rx: u32,
    tx: u32,
    ch: u32,
) -> (u32, bool) {
    let rxbuf = DMA_BUF;
    let txbuf = DMA_BUF + 0x10;
    wr(rxbuf, 0xFFFF_FFFF);
    wr(txbuf, 0);
    clear_flags(dma);
    let r = stream(dma, rx);
    let t = stream(dma, tx);
    wr(r + X_PAR, spi + S_DR);
    wr(r + X_M0AR, rxbuf);
    wr(r + X_NDTR, 4);
    wr(r + X_CR, chsel(ch) | X_CR_MINC | X_CR_TCIE | X_CR_EN); // P->M
    wr(t + X_PAR, spi + S_DR);
    wr(t + X_M0AR, txbuf);
    wr(t + X_NDTR, 4);
    wr(t + X_CR, chsel(ch) | X_CR_MINC | X_CR_M2P | X_CR_EN);

    cs(port, pin, true);
    wr(spi + S_CR2, SPI_CR2_8BIT | (1 << 1) | 1); // TXDMAEN | RXDMAEN
    let tcif = 0x20 << shift(rx);
    let done = wait(flag_reg(dma, rx), tcif, tcif);
    let _ = wait(spi + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    wr(spi + S_CR2, SPI_CR2_8BIT);
    (u32::from_be_bytes(rd(rxbuf).to_le_bytes()), done)
}

/// SPI `name` full duplex through `dma` streams `rx` / `tx`. First with the
/// wrong SxCR.CHSEL (`wrong`): the request must reach neither stream, so
/// nothing moves. Then with the right one (`ch`): the frame lands, both
/// streams report HTIF | TCIF and nothing else, and only the RX stream's
/// line (TCIE) pends.
#[allow(clippy::too_many_arguments)]
pub fn check_spi_dma(
    name: &str,
    spi: u32,
    port: u32,
    pin: u32,
    want: u32,
    dma: u32,
    lines: &[u32; 8],
    rx: u32,
    tx: u32,
    ch: u32,
    wrong: u32,
) -> bool {
    for &irq in lines {
        unpend(irq);
    }
    let (_, moved_wrong) = spi_dma_frame(spi, port, pin, dma, rx, tx, wrong);
    let unselected = !moved_wrong
        && rd(stream(dma, rx) + X_NDTR) == 4
        && rd(stream(dma, tx) + X_NDTR) == 4
        && rd(DMA_BUF) == 0xFFFF_FFFF;
    let off = stream_off(dma, rx) && stream_off(dma, tx);
    // A TX request left pending by the unselected run is dropped with SPI
    // disabled and re-enabled.
    wr(spi + S_CR1, rd(spi + S_CR1) & !(1 << 6));
    while rd(spi + S_SR) & SPI_SR_RXNE != 0 {
        let _ = rd8(spi + S_DR);
    }
    wr(spi + S_CR1, rd(spi + S_CR1) | (1 << 6));

    let (frame, done) = spi_dma_frame(spi, port, pin, dma, rx, tx, ch);
    let data = done && frame == want;
    let rx_flags = rd(flag_reg(dma, rx)) & sflags(rx);
    let tx_flags = rd(flag_reg(dma, tx)) & sflags(tx);
    let flags = rx_flags == sdone(rx)
        && tx_flags == sdone(tx)
        && rd(stream(dma, rx) + X_NDTR) == 0
        && rd(stream(dma, tx) + X_NDTR) == 0;
    let rx_line = lines[rx as usize];
    let pend = pending(rx_line);
    let alone = lines.iter().all(|&o| o == rx_line || !pending(o));
    clear_flags(dma);
    unpend(rx_line);
    let released = !pending(rx_line) && rd(dma + D_LISR) == 0 && rd(dma + D_HISR) == 0;
    let _ = stream_off(dma, rx) && stream_off(dma, tx);

    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("unselected", unselected && off);
    flag("data", data);
    flag("flags", flags);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = unselected && off && data && flags && pend && alone && released;
    verdict(ok);
    ok
}

/// Memory-to-memory is DMA2 only (RM0385 §8.3.6): the same DIR = 10 stream 1
/// setup copies four words on DMA2 (TCIF1) and never starts on DMA1.
pub fn check_dma_m2m() -> bool {
    let src = DMA_BUF + 0x40;
    let dst = DMA_BUF + 0x60;
    let pattern = [0x1122_3344u32, 0x5566_7788, 0x99AA_BBCC, 0xDDEE_FF00];
    let run = |dma: u32| -> bool {
        for (i, &w) in pattern.iter().enumerate() {
            wr(src + 4 * i as u32, w);
            wr(dst + 4 * i as u32, 0);
        }
        clear_flags(dma);
        let s = stream(dma, 1);
        wr(s + X_PAR, src);
        wr(s + X_M0AR, dst);
        wr(s + X_NDTR, 4);
        wr(
            s + X_CR,
            X_CR_WORDS | X_CR_MINC | X_CR_PINC | X_CR_M2M | X_CR_EN,
        );
        wait(dma + D_LISR, 0x20 << 6, 0x20 << 6)
    };
    let dma1_ran = run(DMA1);
    let dma1_idle =
        !dma1_ran && rd(stream(DMA1, 1) + X_NDTR) == 4 && (0..4).all(|i| rd(dst + 4 * i) == 0);
    let _ = stream_off(DMA1, 1);
    clear_flags(DMA1);
    let dma2_ran = run(DMA2);
    let dma2_copied = dma2_ran
        && (0..4).all(|i| rd(dst + 4 * i) == pattern[i as usize])
        && rd(stream(DMA2, 1) + X_NDTR) == 0;
    let _ = stream_off(DMA2, 1);
    clear_flags(DMA2);

    puts("dma m2m ");
    flag("dma1_refused", dma1_idle);
    flag("dma2_copied", dma2_copied);
    let ok = dma1_idle && dma2_copied;
    verdict(ok);
    ok
}

pub fn i2c_cr2(addr: u32, nbytes: u32, extra: u32) -> u32 {
    (addr << 1) | (nbytes << 16) | extra
}

/// TMP102 temperature register (pointer 0): one-byte write of the pointer
/// (software end, TC), then a two-byte read with AUTOEND.
pub fn tmp102_read() -> Option<u16> {
    let isr = I2C1 + I_ISR;
    wr(I2C1 + I_CR2, i2c_cr2(TMP102, 1, I2C_CR2_START));
    if !wait(isr, I2C_ISR_TXIS, I2C_ISR_TXIS) {
        return None;
    }
    wr(I2C1 + I_TXDR, 0);
    if !wait(isr, I2C_ISR_TC, I2C_ISR_TC) {
        return None;
    }
    wr(
        I2C1 + I_CR2,
        i2c_cr2(TMP102, 2, I2C_CR2_RD_WRN | I2C_CR2_AUTOEND | I2C_CR2_START),
    );
    if !wait(isr, I2C_ISR_RXNE, I2C_ISR_RXNE) {
        return None;
    }
    let msb = rd(I2C1 + I_RXDR) as u8;
    if !wait(isr, I2C_ISR_RXNE, I2C_ISR_RXNE) {
        return None;
    }
    let lsb = rd(I2C1 + I_RXDR) as u8;
    if !wait(isr, I2C_ISR_STOPF, I2C_ISR_STOPF) {
        return None;
    }
    wr(I2C1 + I_ICR, I2C_ISR_STOPF);
    Some(u16::from_be_bytes([msb, lsb]))
}

/// I2C1 master: a TMP102 read (25 °C = 0x1900), then RXIE raising the
/// event line 31 during a second read (25.5 °C).
pub fn check_i2c1_tmp102() -> bool {
    wr(I2C1 + I_TIMINGR, 0x2000_090E); // 100 kHz from the 16 MHz HSI
    wr(I2C1 + I_CR1, I2C_CR1_PE);
    let temp = tmp102_read();
    let data = temp == Some(0x1900);

    unpend(31);
    let quiet = !pending(31);
    wr(I2C1 + I_CR1, I2C_CR1_PE | I2C_CR1_RXIE);
    let again = tmp102_read();
    // The TMP102 model self-drifts +0.5 °C (0x80) per full temperature read
    // (configs/devices/tmp102.yaml), so the second read is 25.5 °C.
    let rx_pend = quiet && again == Some(0x1980) && pending(31) && !pending(32);
    wr(I2C1 + I_CR1, I2C_CR1_PE);
    unpend(31);
    wr(I2C1 + I_CR1, 0);

    puts("i2c1 tmp102=");
    hex(u32::from(temp.unwrap_or(0xFFFF)));
    puts(" ");
    flag("data", data);
    flag("rx_pend", rx_pend);
    let ok = data && rx_pend;
    verdict(ok);
    ok
}

/// Each I2C: OAR1 readback, then an absent address NACKing under NACKIE —
/// NACKF on the instance's own event line (no error line, no other I2C's
/// lines), STOP after AUTOEND, ICR clearing both.
pub fn check_i2c_nack(name: &str, base: u32, ev: u32, er: u32) -> bool {
    wr(base + I_OAR1, (1 << 15) | 0xAA);
    let oar = rd(base + I_OAR1) & 0xFFFF == 0x80AA;
    wr(base + I_OAR1, 0);
    wr(base + I_TIMINGR, 0x2000_090E);
    for &(_, _, e, r) in &I2CS {
        unpend(e);
        unpend(r);
    }
    wr(base + I_CR1, I2C_CR1_PE | I2C_CR1_NACKIE);
    wr(
        base + I_CR2,
        i2c_cr2(0x50, 1, I2C_CR2_AUTOEND | I2C_CR2_START),
    );
    let nack = wait(base + I_ISR, I2C_ISR_NACKF, I2C_ISR_NACKF);
    let nack_pend = nack
        && pending(ev)
        && !pending(er)
        && I2CS
            .iter()
            .all(|&(_, _, e, r)| e == ev || (!pending(e) && !pending(r)));
    let stopped = wait(base + I_ISR, I2C_ISR_STOPF, I2C_ISR_STOPF);
    wr(base + I_ICR, I2C_ISR_NACKF | I2C_ISR_STOPF);
    wr(base + I_CR1, I2C_CR1_PE);
    unpend(ev);
    let released =
        rd(base + I_ISR) & (I2C_ISR_NACKF | I2C_ISR_STOPF | I2C_ISR_BUSY) == 0 && !pending(ev);
    wr(base + I_CR1, 0);

    puts(name);
    puts(" ");
    flag("oar", oar);
    flag("nack", nack);
    flag("nack_pend", nack_pend);
    flag("stop", stopped);
    flag("released", released);
    let ok = oar && nack && nack_pend && stopped && released;
    verdict(ok);
    ok
}

/// CRC-32/MPEG-2 (poly 0x04C11DB7, no reflection): one word 0x12345678 from
/// the 0xFFFFFFFF reset seed gives 0xDF8A8A2B; CR.RESET reloads INIT, a zero
/// INIT changes the result; the IDR keeps 8 bits; and a 16-bit polynomial
/// fed by bytes gives CRC-16/CCITT-FALSE ("123456789" -> 0x29B1).
pub fn check_crc() -> bool {
    wr(CRC_CR, 1); // RESET
    let seed = rd(CRC_DR) == 0xFFFF_FFFF;
    wr(CRC_DR, 0x1234_5678);
    let word = rd(CRC_DR);
    let std = word == 0xDF8A_8A2B;
    wr(CRC_INIT, 0);
    wr(CRC_CR, 1);
    let reloaded = rd(CRC_DR) == 0;
    wr(CRC_DR, 0x1234_5678);
    let zero_seed = rd(CRC_DR);
    let init = reloaded && zero_seed != word && rd(CRC_INIT) == 0;

    wr(CRC_IDR, 0xFFFF_FFFF);
    let idr = rd(CRC_IDR) == 0xFF;
    wr(CRC_IDR, 0);

    wr(CRC_POL, 0x1021);
    wr(CRC_INIT, 0xFFFF);
    wr(CRC_CR, (0b01 << 3) | 1); // POLYSIZE = 16 | RESET
    for b in b"123456789" {
        wr8(CRC_DR, *b);
    }
    let crc16 = rd(CRC_DR) & 0xFFFF;
    let poly16 = crc16 == 0x29B1;
    wr(CRC_POL, 0x04C1_1DB7);
    wr(CRC_INIT, 0xFFFF_FFFF);
    wr(CRC_CR, 1);

    puts("crc word=");
    hex(word);
    puts(" crc16=");
    hex(crc16);
    puts(" ");
    flag("seed", seed);
    flag("std", std);
    flag("init", init);
    flag("idr", idr);
    flag("poly16", poly16);
    let ok = seed && std && init && idr && poly16;
    verdict(ok);
    ok
}

/// RNG: DR reads 0 and DRDY is clear until RNGEN; then DRDY is set and
/// successive draws differ; clearing RNGEN drops DRDY again.
pub fn check_rng() -> bool {
    let off = rd(RNG_SR) & 1 == 0 && rd(RNG_DR) == 0;
    wr(RNG_CR, 1 << 2); // RNGEN
    let ready = wait(RNG_SR, 1, 1);
    let a = rd(RNG_DR);
    let b = rd(RNG_DR);
    let c = rd(RNG_DR);
    let draws = a != b && b != c && a != c && a != 0;
    wr(RNG_CR, 0);
    let stopped = rd(RNG_SR) & 1 == 0;

    puts("rng ");
    flag("off", off);
    flag("ready", ready);
    flag("draws", draws);
    flag("stopped", stopped);
    let ok = off && ready && draws && stopped;
    verdict(ok);
    ok
}
