//! STM32L071/L072 smoke for the imported Renode platforms, shared by
//! `firmware-stm32l071-smoke` and `firmware-stm32l072-smoke`: RCC clock
//! gates, USART1/2/4/5 + LPUART1, GPIOA-E/H, SYSCFG EXTICR + EXTI,
//! TIM2/3/6/7/21/22, SPI1/SPI2 (classic SPI), I2C1/2/3 ("v2" I2C), DMA1 with
//! CSELR request selection, CRC and the ADC; on the L072 also the DAC and
//! the RNG.
//! Register offsets, bases and IRQ numbers are RM0376/RM0377 and ST's CMSIS
//! `stm32l071xx.h` / `stm32l072xx.h`.
//!
//! The fixture system puts a MAX31855 on each SPI (thermocouple 25 and
//! 100 °C, so each bus answers with its own frame), a second MAX31855 on SPI1
//! behind its own chip select (PB0, 50 °C), a TMP102 at 0x48 on each I2C
//! and two potentiometers on ADC IN1 (1.65 V) and IN9 (0.825 V).
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

/// Each crate constructs one variant.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    L071,
    L072,
}

const SUMMARY: u32 = 0x2000_1000;

// RCC (RM0376 §7.3).
const RCC: u32 = 0x4002_1000;
const RCC_CR: u32 = RCC;
const RCC_CRRCR: u32 = RCC + 0x08;
const RCC_IOPENR: u32 = RCC + 0x2C;
const RCC_AHBENR: u32 = RCC + 0x30;
const RCC_APB2ENR: u32 = RCC + 0x34;
const RCC_APB1ENR: u32 = RCC + 0x38;
// GPIOA..E, GPIOH.
const IOP_ON: u32 = 0b1_1111 | (1 << 7);
// DMA1, CRC.
const AHB_ON: u32 = 1 | (1 << 12);
// SYSCFG, TIM21, TIM22, SPI1, USART1.
const APB2_ON: u32 = 1 | (1 << 2) | (1 << 5) | (1 << 12) | (1 << 14);
// TIM2/3/6/7, SPI2, USART2, LPUART1, USART4/5, I2C1/2, I2C3.
const APB1_ON: u32 = 0b11 | (0b11 << 4) | (1 << 14) | (0b1111 << 17) | (0b11 << 21) | (1 << 30);
const RCC_APB1ENR_DACEN: u32 = 1 << 29;
const RCC_APB2ENR_ADCEN: u32 = 1 << 9;
const RCC_AHBENR_RNGEN: u32 = 1 << 20;

// USART "v2" layout.
const USART1: u32 = 0x4001_3800;
const USART2: u32 = 0x4000_4400;
const USART4: u32 = 0x4000_4C00;
const USART5: u32 = 0x4000_5000;
const LPUART1: u32 = 0x4000_4800;
const U_CR1: u32 = 0x00;
const U_BRR: u32 = 0x0C;
const U_ISR: u32 = 0x1C;
const U_TDR: u32 = 0x28;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_TXEIE: u32 = 1 << 7;

/// (name, base, NVIC line) — `stm32l07xxx.h` USARTx_IRQn; USART4/5 share
/// USART4_5_IRQn (14), LPUART1 is (RNG_)LPUART1_IRQn (29).
const USARTS: [(&str, u32, u32); 5] = [
    ("u1", USART1, 27),
    ("u2", USART2, 28),
    ("u4", USART4, 14),
    ("u5", USART5, 14),
    ("lp1", LPUART1, 29),
];

// GPIO (IOPORT).
const GPIOA: u32 = 0x5000_0000;
const GPIOB: u32 = 0x5000_0400;
const GPIOC: u32 = 0x5000_0800;
const GPIOD: u32 = 0x5000_0C00;
const GPIOE: u32 = 0x5000_1000;
const GPIOH: u32 = 0x5000_1C00;
const G_MODER: u32 = 0x00;
const G_PUPDR: u32 = 0x0C;
const G_IDR: u32 = 0x10;
const G_ODR: u32 = 0x14;
const G_BSRR: u32 = 0x18;
const G_BRR: u32 = 0x28;

// SYSCFG / EXTI.
const SYSCFG_EXTICR1: u32 = 0x4001_0008;
const SYSCFG_EXTICR4: u32 = 0x4001_0014;
const EXTI: u32 = 0x4001_0400;
const E_IMR: u32 = 0x00;
const E_FTSR: u32 = 0x0C;
const E_SWIER: u32 = 0x10;
const E_PR: u32 = 0x14;
// SYSCFG_EXTICRx port codes (`stm32l072xx.h` SYSCFG_EXTICR1_EXTI0_Px).
const CODE_PA: u32 = 0;
const CODE_PC: u32 = 2;
const CODE_PH: u32 = 5;

// TIMx.
const T_CR1: u32 = 0x00;
const T_DIER: u32 = 0x0C;
const T_SR: u32 = 0x10;
const T_CNT: u32 = 0x24;
const T_PSC: u32 = 0x28;
const T_ARR: u32 = 0x2C;

/// (name, base, NVIC line) — `stm32l072xx.h` TIMx_BASE / TIMx_IRQn. Every
/// L0 timer is 16-bit, TIM2 included.
const TIMERS: [(&str, u32, u32); 6] = [
    ("tim2", 0x4000_0000, 15),
    ("tim3", 0x4000_0400, 16),
    ("tim6", 0x4000_1000, 17),
    ("tim7", 0x4000_1400, 18),
    ("tim21", 0x4001_0800, 20),
    ("tim22", 0x4001_1400, 22),
];

// SPIx (classic SPI: CR1.DFF = 0 is an 8-bit frame).
const S_CR1: u32 = 0x00;
const S_CR2: u32 = 0x04;
const S_SR: u32 = 0x08;
const S_DR: u32 = 0x0C;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;
const SPI_SR_BSY: u32 = 1 << 7;
const SPI1: u32 = 0x4001_3000;
const SPI2: u32 = 0x4000_3800;

/// (name, base, NVIC line, CS port, CS pin, expected MAX31855 frame).
/// Frames: thermocouple << 18 (0.25 °C/LSB) | internal 22 °C << 4.
/// The second SPI1 slave (CS PB0): 50 °C thermocouple, 22 °C cold junction.
const SPI1_B_FRAME: u32 = 0x0320_1600;
const SPIS: [(&str, u32, u32, u32, u32, u32); 2] = [
    ("spi1", SPI1, 25, GPIOA, 4, 0x0190_1600),
    ("spi2", SPI2, 26, GPIOB, 12, 0x0640_1600),
];

// DMA1: ISR, IFCR, channel n at 0x08 + 20 * (n - 1), CSELR at 0xA8.
const DMA1: u32 = 0x4002_0000;
const D_ISR: u32 = 0x00;
const D_IFCR: u32 = 0x04;
const D_CSELR: u32 = 0xA8;
const fn dma_ch(n: u32) -> u32 {
    DMA1 + 0x08 + 20 * (n - 1)
}
const C_CCR: u32 = 0x00;
const C_CNDTR: u32 = 0x04;
const C_CPAR: u32 = 0x08;
const C_CMAR: u32 = 0x0C;
const DMA_BUF: u32 = 0x2000_1100;
const DMA_LINES: [u32; 3] = [9, 10, 11];

// I2Cx ("v2" IP).
const I_CR1: u32 = 0x00;
const I_CR2: u32 = 0x04;
const I_OAR1: u32 = 0x08;
const I_TIMINGR: u32 = 0x10;
const I_ISR: u32 = 0x18;
const I_ICR: u32 = 0x1C;
const I_RXDR: u32 = 0x24;
const I_TXDR: u32 = 0x28;
const I2C_ISR_TXIS: u32 = 1 << 1;
const I2C_ISR_RXNE: u32 = 1 << 2;
const I2C_ISR_NACKF: u32 = 1 << 4;
const I2C_ISR_STOPF: u32 = 1 << 5;
const I2C_ISR_TC: u32 = 1 << 6;
const I2C_ISR_BUSY: u32 = 1 << 15;
const I2C_CR1_PE: u32 = 1 << 0;
const I2C_CR1_RXIE: u32 = 1 << 2;
const I2C_CR1_NACKIE: u32 = 1 << 4;
const I2C_CR2_RD_WRN: u32 = 1 << 10;
const I2C_CR2_START: u32 = 1 << 13;
const I2C_CR2_AUTOEND: u32 = 1 << 25;
const TMP102: u32 = 0x48;

/// (name, base, NVIC line) — I2C1_IRQn 23, I2C2_IRQn 24, I2C3_IRQn 21.
const I2CS: [(&str, u32, u32); 3] = [
    ("i2c1", 0x4000_5400, 23),
    ("i2c2", 0x4000_5800, 24),
    ("i2c3", 0x4000_7800, 21),
];

// CRC.
const CRC: u32 = 0x4002_3000;
const CRC_DR: u32 = CRC;
const CRC_CR: u32 = CRC + 0x08;
const CRC_INIT: u32 = CRC + 0x10;

// DAC (L072).
const DAC: u32 = 0x4000_7400;
const DAC_CR: u32 = DAC;
const DAC_SWTRIGR: u32 = DAC + 0x04;
const DAC_DHR12R1: u32 = DAC + 0x08;
const DAC_DHR12L2: u32 = DAC + 0x18;
const DAC_DHR8RD: u32 = DAC + 0x28;
const DAC_DOR1: u32 = DAC + 0x2C;
const DAC_DOR2: u32 = DAC + 0x30;

// RNG (L072).
const RNG: u32 = 0x4002_5000;
const RNG_CR: u32 = RNG;
const RNG_SR: u32 = RNG + 0x04;
const RNG_DR: u32 = RNG + 0x08;
const RNG_CR_RNGEN: u32 = 1 << 2;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

// ADC (RM0377 §14.12, `stm32l072xx.h` ADC1_BASE / ADC1_COMP_IRQn).
const ADC: u32 = 0x4001_2400;
const ADC_ISR: u32 = ADC;
const ADC_IER: u32 = ADC + 0x04;
const ADC_CR: u32 = ADC + 0x08;
const ADC_CFGR1: u32 = ADC + 0x0C;
const ADC_SMPR: u32 = ADC + 0x14;
const ADC_TR: u32 = ADC + 0x20;
const ADC_CHSELR: u32 = ADC + 0x28;
const ADC_DR: u32 = ADC + 0x40;
const ADC_CCR: u32 = ADC + 0x308;
const ADC_LINE: u32 = 12;
const ADC_ISR_ADRDY: u32 = 1 << 0;
const ADC_ISR_EOC: u32 = 1 << 2;
const ADC_ISR_EOS: u32 = 1 << 3;
const ADC_ISR_OVR: u32 = 1 << 4;
const ADC_ISR_AWD: u32 = 1 << 7;
const ADC_ISR_EOCAL: u32 = 1 << 11;
const ADC_CR_ADEN: u32 = 1 << 0;
const ADC_CR_ADDIS: u32 = 1 << 1;
const ADC_CR_ADSTART: u32 = 1 << 2;
const ADC_CR_ADCAL: u32 = 1 << 31;
const ADC_CFGR1_DMAEN: u32 = 1 << 0;
const ADC_CFGR1_SCANDIR: u32 = 1 << 2;
const ADC_CFGR1_RES_8: u32 = 0b10 << 3;
const ADC_CFGR1_AWDSGL: u32 = 1 << 22;
const ADC_CFGR1_AWDEN: u32 = 1 << 23;
const ADC_CCR_VREFEN: u32 = 1 << 22;
const ADC_CCR_TSEN: u32 = 1 << 23;
/// IN1 (pot, 1.65 V), IN9 (pot, 0.825 V), IN17 (V_REFINT, 1.224 V typ),
/// IN18 (temperature sensor at 25 °C: 670 mV - 105 x 1.61 mV = 501 mV).
const ADC_SEQ: u32 = (1 << 1) | (1 << 9) | (1 << 17) | (1 << 18);
/// floor(mV x 4095 / 3300) for those four, in CHSELR order.
const ADC_WANT: [u32; 4] = [2047, 1023, 1518, 621];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn pending(irq: u32) -> bool {
    rd(NVIC_ISPR0) & (1 << irq) != 0
}

fn unpend(irq: u32) {
    wr(NVIC_ICPR0, 1 << irq);
}

fn uart_init(base: u32) {
    // LPUART1's BRR must be at least 0x300; the USARTs take any value.
    wr(base + U_BRR, 0x369);
    wr(base + U_CR1, (1 << 3) | 1); // TE | UE
}

fn send(base: u32, bytes: &[u8]) {
    for byte in bytes {
        let mut remaining = 100_000;
        while rd(base + U_ISR) & USART_ISR_TXE == 0 && remaining > 0 {
            remaining -= 1;
        }
        wr(base + U_TDR, u32::from(*byte));
    }
}

fn puts(s: &str) {
    send(USART1, s.as_bytes());
}

fn flag(name: &str, ok: bool) {
    puts(name);
    puts(if ok { "=1 " } else { "=0 " });
}

fn verdict(ok: bool) {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
}

fn hex(v: u32) {
    let digits = b"0123456789ABCDEF";
    let mut buf = [0u8; 10];
    buf[0] = b'0';
    buf[1] = b'x';
    for i in 0..8 {
        buf[2 + i] = digits[((v >> (28 - 4 * i)) & 0xF) as usize];
    }
    send(USART1, &buf);
}

/// RCC reset state (MSI on and ready, IOPENR = 0) and a clock gate that
/// holds: GPIOH ignores ODR writes until IOPHEN is set. Runs before
/// anything else, so USART1 is still gated and the result is reported
/// later.
fn rcc_bringup(variant: Variant) -> (bool, bool) {
    let reset = rd(RCC_CR) & 0x300 == 0x300 && rd(RCC_IOPENR) == 0;
    wr(GPIOH + G_ODR, 0x0003);
    let gated = rd(GPIOH + G_ODR) == 0;
    let apb1 = if variant == Variant::L072 {
        APB1_ON | RCC_APB1ENR_DACEN
    } else {
        APB1_ON
    };
    wr(RCC_IOPENR, IOP_ON);
    wr(RCC_AHBENR, AHB_ON);
    wr(RCC_APB2ENR, APB2_ON);
    wr(RCC_APB1ENR, apb1);
    wr(GPIOH + G_ODR, 0x0003);
    let gate = gated && rd(GPIOH + G_ODR) == 0x0003 && rd(RCC_IOPENR) == IOP_ON;
    wr(GPIOH + G_ODR, 0);
    (reset, gate)
}

fn report_rcc(reset: bool, gate: bool) -> bool {
    puts("rcc ");
    flag("reset", reset);
    flag("gate", gate);
    let ok = reset && gate;
    verdict(ok);
    ok
}

/// TXEIE on each USART raises its own line and no other USART's line
/// (USART4 and USART5 share line 14).
fn check_usart_irqs() -> bool {
    let mut ok = true;
    puts("usart irq ");
    for &(name, base, irq) in &USARTS {
        unpend(irq);
        let quiet = !pending(irq);
        wr(base + U_CR1, rd(base + U_CR1) | USART_CR1_TXEIE);
        let pend = pending(irq)
            && USARTS
                .iter()
                .all(|&(_, _, other)| other == irq || !pending(other));
        wr(base + U_CR1, rd(base + U_CR1) & !USART_CR1_TXEIE);
        unpend(irq);
        let released = !pending(irq);
        let this = quiet && pend && released;
        flag(name, this);
        ok &= this;
    }
    verdict(ok);
    ok
}

/// RM0376 reset values: the SWD pads on port A, every other pad analog
/// (MODER = 0xFFFFFFFF); set/reset through BSRR/BRR on port H.
fn check_gpio() -> bool {
    let pa = rd(GPIOA + G_MODER) == 0xEBFF_FCFF && rd(GPIOA + G_PUPDR) == 0x2400_0000;
    let others = [GPIOB, GPIOC, GPIOD, GPIOE, GPIOH]
        .iter()
        .all(|&p| rd(p + G_MODER) == 0xFFFF_FFFF && rd(p + G_PUPDR) == 0);
    // PH0 output; toggle it four times for the gpio_edges assertion.
    wr(GPIOH + G_MODER, (rd(GPIOH + G_MODER) & !0b11) | 0b01);
    let mut toggles = true;
    for _ in 0..2 {
        wr(GPIOH + G_BSRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 1 && rd(GPIOH + G_IDR) & 1 == 1;
        wr(GPIOH + G_BRR, 1);
        toggles &= rd(GPIOH + G_ODR) & 1 == 0;
    }
    // Each port keeps its own ODR.
    wr(GPIOD + G_ODR, 0x00F0);
    wr(GPIOE + G_ODR, 0x0F00);
    let de = rd(GPIOD + G_ODR) == 0x00F0 && rd(GPIOE + G_ODR) == 0x0F00 && rd(GPIOH + G_ODR) == 0;
    wr(GPIOD + G_ODR, 0);
    wr(GPIOE + G_ODR, 0);
    puts("gpio ");
    flag("pa", pa);
    flag("reset", others);
    flag("ph", toggles);
    flag("de", de);
    let ok = pa && others && toggles && de;
    verdict(ok);
    ok
}

/// SWIER on lines 1, 3 and 12 reaches the grouped NVIC vectors
/// (EXTI0_1 = 5, EXTI2_3 = 6, EXTI4_15 = 7), and only while unmasked.
fn check_exti_swier() -> bool {
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_SWIER, 1 << 1);
    let masked = rd(EXTI + E_PR) & (1 << 1) != 0 && !pending(5);
    wr(EXTI + E_PR, 1 << 1);
    unpend(5);
    let mut routed = true;
    for &(line, irq) in &[(1u32, 5u32), (3, 6), (12, 7)] {
        wr(EXTI + E_IMR, 1 << line);
        wr(EXTI + E_SWIER, 1 << line);
        routed &= rd(EXTI + E_PR) & (1 << line) != 0
            && pending(irq)
            && (5..=7).all(|o| o == irq || !pending(o));
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

fn wait_level(port: u32, pin: u32, high: bool) -> bool {
    for _ in 0..2_000_000 {
        if (rd(port + G_IDR) >> pin) & 1 == u32::from(high) {
            return true;
        }
    }
    false
}

/// Digital input with pull-up (the pad resets analog).
fn input_pullup(port: u32, pin: u32) {
    wr(port + G_MODER, rd(port + G_MODER) & !(0b11 << (2 * pin)));
    let pupdr = rd(port + G_PUPDR) & !(0b11 << (2 * pin));
    wr(port + G_PUPDR, pupdr | (0b01 << (2 * pin)));
}

/// Falling edges from the two fixture buttons (PH1, PC13; pressed = low)
/// through the SYSCFG port mux. Line 1 first selects code 0 (PA): the first
/// PH1 press must not pend it. With code 5 (PH) the second press must, on
/// NVIC EXTI0_1 = 5. PC13 lands on EXTI4_15 = 7.
fn check_exti_pads() -> bool {
    input_pullup(GPIOH, 1);
    input_pullup(GPIOC, 13);
    wr(SYSCFG_EXTICR1, CODE_PA << 4);
    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    wr(EXTI + E_FTSR, (1 << 1) | (1 << 13));
    wr(EXTI + E_IMR, (1 << 1) | (1 << 13));

    let idle = wait_level(GPIOH, 1, true) && wait_level(GPIOC, 13, true);
    let press1 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << 1) == 0 && !pending(5);

    wr(SYSCFG_EXTICR1, CODE_PH << 4);
    let press2 = wait_level(GPIOH, 1, false) && wait_level(GPIOH, 1, true);
    let ph = rd(EXTI + E_PR) & (1 << 1) != 0 && pending(5);
    wr(EXTI + E_PR, 1 << 1);
    unpend(5);

    let press3 = wait_level(GPIOC, 13, false) && wait_level(GPIOC, 13, true);
    let pc = rd(EXTI + E_PR) & (1 << 13) != 0 && pending(7);
    wr(EXTI + E_PR, 1 << 13);
    unpend(7);
    wr(EXTI + E_IMR, 0);

    puts("exti pads ");
    flag("idle", idle);
    flag("presses", press1 && press2 && press3);
    flag("pa_quiet", other_port_quiet);
    flag("ph1", ph);
    flag("pc13", pc);
    let ok = idle && press1 && press2 && press3 && other_port_quiet && ph && pc;
    verdict(ok);
    ok
}

/// Counter width (ARR keeps 16 bits), counting, and the update interrupt
/// reaching the timer's own NVIC line — and no other timer's.
fn check_timer(name: &str, base: u32, irq: u32) -> bool {
    wr(base + T_ARR, 0xFFFF_FFFF);
    let arr = rd(base + T_ARR);
    let width_ok = arr == 0xFFFF;

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
    let alone = TIMERS
        .iter()
        .all(|&(_, _, other)| other == irq || !pending(other));
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

fn wait(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if rd(addr) & mask == want {
            return true;
        }
    }
    false
}

fn cs(port: u32, pin: u32, low: bool) {
    wr(port + G_BSRR, if low { 1 << (pin + 16) } else { 1 << pin });
}

fn cs_output(port: u32, pin: u32) {
    cs(port, pin, false);
    let moder = rd(port + G_MODER) & !(0b11 << (2 * pin));
    wr(port + G_MODER, moder | (0b01 << (2 * pin)));
}

/// Master, software NSS (SSM | SSI), fPCLK/8, mode 0, 8-bit frames.
fn spi_enable(base: u32) {
    wr(base + S_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(base + S_CR1, rd(base + S_CR1) | (1 << 6)); // SPE
}

fn spi_xfer(base: u32, out: u8) -> Option<u8> {
    if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
        return None;
    }
    wr(base + S_DR, u32::from(out));
    if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
        return None;
    }
    Some(rd(base + S_DR) as u8)
}

/// One 32-bit MAX31855 frame under CS, by polling.
fn max31855_frame(base: u32, port: u32, pin: u32) -> Option<u32> {
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

/// Each SPI reads its own device's frame twice — polled, then with RXNEIE,
/// which must pend the SPI's own NVIC line and no other SPI's.
fn check_spi(name: &str, base: u32, irq: u32, port: u32, pin: u32, want: u32) -> bool {
    cs_output(port, pin);
    spi_enable(base);
    let frame = max31855_frame(base, port, pin).unwrap_or(0xDEAD_BEEF);
    let quiet = !pending(irq);

    unpend(irq);
    wr(base + S_CR2, 1 << 6); // RXNEIE
    let again = max31855_frame(base, port, pin);
    let data = frame == want && again == Some(want);
    let pend = quiet && pending(irq);
    let alone = SPIS
        .iter()
        .all(|&(_, _, other, _, _, _)| other == irq || !pending(other));
    wr(base + S_CR2, 0);
    unpend(irq);
    let released = !pending(irq) && rd(base + S_SR) & SPI_SR_RXNE == 0;

    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("data", data);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = data && pend && alone && released;
    verdict(ok);
    ok
}

/// Two MAX31855s share SPI1, told apart only by their GPIO chip selects
/// (PA4, PB0): each answers with its own frame while the other is released.
/// Then a frame cut short by CS↑ after two bytes: the next CS↓ must restart
/// the MAX31855 at bit 31, so the following whole frame is not rotated.
fn check_spi1_shared_cs() -> bool {
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

/// Arm an RX/TX channel pair for one four-byte SPI frame (RX has TCIE).
fn dma_arm(spi: u32, rx_ch: u32, tx_ch: u32, rx: u32, tx: u32) {
    let rxc = dma_ch(rx_ch);
    let txc = dma_ch(tx_ch);
    wr(rxc + C_CPAR, spi + S_DR);
    wr(rxc + C_CMAR, rx);
    wr(rxc + C_CNDTR, 4);
    wr(rxc + C_CCR, (1 << 7) | (1 << 1) | 1); // MINC | TCIE | EN, P->M
    wr(txc + C_CPAR, spi + S_DR);
    wr(txc + C_CMAR, tx);
    wr(txc + C_CNDTR, 4);
    wr(txc + C_CCR, (1 << 7) | (1 << 4) | 1); // MINC | DIR | EN, M->P
}

fn dma_disarm(spi: u32, rx_ch: u32, tx_ch: u32) {
    wr(spi + S_CR2, 0);
    wr(dma_ch(rx_ch) + C_CCR, 0);
    wr(dma_ch(tx_ch) + C_CCR, 0);
}

/// An SPI frame through a DMA1 channel pair. The L0 routes a request to a
/// channel only while that channel's CSELR nibble selects it (RM0376
/// Table 51: SPI1_RX/TX = channel 2/3 with C2S/C3S = 1, SPI2_RX/TX =
/// channel 4/5 with C4S/C5S = 2). First with the pair's nibbles holding
/// the wrong selector: nothing moves. Then with the right ones: the frame
/// lands, TCIF of the RX channel pends `line`, and no other DMA line pends.
fn check_dma_spi(
    name: &str,
    spi: u32,
    (rx_ch, tx_ch): (u32, u32),
    sel: u32,
    line: u32,
    (port, pin, want): (u32, u32, u32),
) -> bool {
    let rx = DMA_BUF;
    let tx = DMA_BUF + 0x10;
    wr(rx, 0xFFFF_FFFF);
    wr(tx, 0);
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    for &irq in &DMA_LINES {
        unpend(irq);
    }
    let nibbles = (0xF << (4 * (rx_ch - 1))) | (0xF << (4 * (tx_ch - 1)));
    let selected = (sel << (4 * (rx_ch - 1))) | (sel << (4 * (tx_ch - 1)));
    // A wrong selector (the other SPI's) on both channels.
    let wrong = (3 - sel) * (selected / sel);
    wr(DMA1 + D_CSELR, wrong);

    dma_arm(spi, rx_ch, tx_ch, rx, tx);
    cs(port, pin, true);
    wr(spi + S_CR2, (1 << 1) | 1); // TXDMAEN | RXDMAEN
    let still = (0..20_000).all(|_| rd(DMA1 + D_ISR) == 0);
    let unselected = still
        && rd(dma_ch(rx_ch) + C_CNDTR) == 4
        && rd(dma_ch(tx_ch) + C_CNDTR) == 4
        && rd(rx) == 0xFFFF_FFFF
        && DMA_LINES.iter().all(|&l| !pending(l));
    dma_disarm(spi, rx_ch, tx_ch);
    cs(port, pin, false);
    let _ = wait(spi + S_SR, SPI_SR_BSY, 0);
    let _ = rd(spi + S_DR);

    wr(DMA1 + D_CSELR, selected);
    let cselr = rd(DMA1 + D_CSELR) & nibbles == selected;
    dma_arm(spi, rx_ch, tx_ch, rx, tx);
    cs(port, pin, true);
    wr(spi + S_CR2, (1 << 1) | 1);
    let tcif = 1 << (4 * (rx_ch - 1) + 1);
    let done = wait(DMA1 + D_ISR, tcif, tcif);
    let _ = wait(spi + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);

    let frame = u32::from_be_bytes(rd(rx).to_le_bytes());
    let data = frame == want;
    let isr = rd(DMA1 + D_ISR);
    // GIF | TCIF | HTIF (four items cross the half-way mark) on both channels.
    let both = (0x7 << (4 * (rx_ch - 1))) | (0x7 << (4 * (tx_ch - 1)));
    let flags = isr == both && rd(dma_ch(rx_ch) + C_CNDTR) == 0 && rd(dma_ch(tx_ch) + C_CNDTR) == 0;
    let pend = done && pending(line);
    let alone = DMA_LINES.iter().all(|&l| l == line || !pending(l));
    wr(DMA1 + D_IFCR, both);
    unpend(line);
    let released = !pending(line) && rd(DMA1 + D_ISR) == 0;
    dma_disarm(spi, rx_ch, tx_ch);
    wr(DMA1 + D_CSELR, 0);

    puts("dma1 ");
    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("unselected", unselected);
    flag("cselr", cselr);
    flag("data", data);
    flag("flags", flags);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = unselected && cselr && data && flags && pend && alone && released;
    verdict(ok);
    ok
}

fn i2c_cr2(addr: u32, nbytes: u32, extra: u32) -> u32 {
    (addr << 1) | (nbytes << 16) | extra
}

/// TMP102 temperature register (pointer 0): one-byte write of the pointer
/// (software end, TC), then a two-byte read with AUTOEND.
fn tmp102_read(i2c: u32) -> Option<u16> {
    let isr = i2c + I_ISR;
    wr(i2c + I_CR2, i2c_cr2(TMP102, 1, I2C_CR2_START));
    if !wait(isr, I2C_ISR_TXIS, I2C_ISR_TXIS) {
        return None;
    }
    wr(i2c + I_TXDR, 0);
    if !wait(isr, I2C_ISR_TC, I2C_ISR_TC) {
        return None;
    }
    wr(
        i2c + I_CR2,
        i2c_cr2(TMP102, 2, I2C_CR2_RD_WRN | I2C_CR2_AUTOEND | I2C_CR2_START),
    );
    if !wait(isr, I2C_ISR_RXNE, I2C_ISR_RXNE) {
        return None;
    }
    let msb = rd(i2c + I_RXDR) as u8;
    if !wait(isr, I2C_ISR_RXNE, I2C_ISR_RXNE) {
        return None;
    }
    let lsb = rd(i2c + I_RXDR) as u8;
    if !wait(isr, I2C_ISR_STOPF, I2C_ISR_STOPF) {
        return None;
    }
    wr(i2c + I_ICR, I2C_ISR_STOPF);
    Some(u16::from_be_bytes([msb, lsb]))
}

/// I2C master on its own TMP102: OAR1 readback, a read (25 °C = 0x1900),
/// RXIE raising the bus's own line during a second read (25.5 °C) and no
/// other I2C's line, and an absent address NACKing (NACKF under NACKIE, the
/// same line), then ICR clearing both.
fn check_i2c(name: &str, i2c: u32, line: u32) -> bool {
    wr(i2c + I_OAR1, (1 << 15) | 0xAA);
    let oar = rd(i2c + I_OAR1) & 0xFFFF == 0x80AA;
    wr(i2c + I_OAR1, 0);
    wr(i2c + I_TIMINGR, 0x0000_0708); // 100 kHz from the 2.1 MHz MSI
    wr(i2c + I_CR1, I2C_CR1_PE);
    let temp = tmp102_read(i2c);
    let data = temp == Some(0x1900);

    unpend(line);
    let quiet = !pending(line);
    wr(i2c + I_CR1, I2C_CR1_PE | I2C_CR1_RXIE);
    let again = tmp102_read(i2c);
    // The TMP102 model self-drifts +0.5 °C (0x80) per full temperature read
    // (configs/devices/tmp102.yaml), so the second read is 25.5 °C.
    let rx_pend = quiet
        && again == Some(0x1980)
        && pending(line)
        && I2CS
            .iter()
            .all(|&(_, _, other)| other == line || !pending(other));
    wr(i2c + I_CR1, I2C_CR1_PE);
    unpend(line);

    wr(i2c + I_CR1, I2C_CR1_PE | I2C_CR1_NACKIE);
    wr(
        i2c + I_CR2,
        i2c_cr2(0x50, 1, I2C_CR2_AUTOEND | I2C_CR2_START),
    );
    let nack = wait(i2c + I_ISR, I2C_ISR_NACKF, I2C_ISR_NACKF);
    let nack_pend = nack && pending(line);
    let stopped = wait(i2c + I_ISR, I2C_ISR_STOPF, I2C_ISR_STOPF);
    wr(i2c + I_ICR, I2C_ISR_NACKF | I2C_ISR_STOPF);
    wr(i2c + I_CR1, I2C_CR1_PE);
    unpend(line);
    let released =
        rd(i2c + I_ISR) & (I2C_ISR_NACKF | I2C_ISR_STOPF | I2C_ISR_BUSY) == 0 && !pending(line);

    puts(name);
    puts(" tmp102=");
    hex(u32::from(temp.unwrap_or(0xFFFF)));
    puts(" ");
    flag("oar", oar);
    flag("data", data);
    flag("rx_pend", rx_pend);
    flag("nack", nack);
    flag("nack_pend", nack_pend);
    flag("stop", stopped);
    flag("released", released);
    let ok = oar && data && rx_pend && nack && nack_pend && stopped && released;
    verdict(ok);
    ok
}

/// CRC-32/MPEG-2 (poly 0x04C11DB7, no reflection): one word 0x12345678 from
/// the 0xFFFFFFFF reset seed gives 0xDF8A8A2B; CR.RESET reloads INIT, and a
/// zero INIT changes the result.
fn check_crc() -> bool {
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
    wr(CRC_INIT, 0xFFFF_FFFF);
    wr(CRC_CR, 1);

    puts("crc word=");
    hex(word);
    puts(" ");
    flag("seed", seed);
    flag("std", std);
    flag("init", init);
    let ok = seed && std && init;
    verdict(ok);
    ok
}

/// DAC (RM0376 §14.7): with no trigger, DHR12R1 and the left-aligned
/// DHR12L2 reach DOR1 (0x2C) and DOR2 (0x30) at once, and the dual 8-bit
/// DHR8RD loads both. With TEN1 and TSEL1 = software, DOR1 holds a new
/// DHR12R1 until SWTRIG1, which then reads back clear.
fn check_dac() -> bool {
    wr(DAC_CR, 1 | (1 << 16)); // EN1 | EN2
    wr(DAC_DHR12R1, 0x0ABC);
    wr(DAC_DHR12L2, 0x1230);
    let direct = rd(DAC_DOR1) == 0x0ABC && rd(DAC_DOR2) == 0x0123;
    wr(DAC_DHR8RD, 0x5AC3);
    let dual = rd(DAC_DOR1) == 0x0C30 && rd(DAC_DOR2) == 0x05A0;

    wr(DAC_CR, 1 | (1 << 2) | (0b111 << 3) | (1 << 16)); // EN1 | TEN1 | TSEL1=SW
    wr(DAC_DHR12R1, 0x0800);
    let held = rd(DAC_DOR1) == 0x0C30;
    wr(DAC_SWTRIGR, 1);
    let dor1 = rd(DAC_DOR1);
    let triggered = dor1 == 0x0800 && rd(DAC_SWTRIGR) == 0 && rd(DAC_DOR2) == 0x05A0;
    wr(DAC_CR, 0);

    puts("dac dor1=");
    hex(dor1);
    puts(" ");
    flag("direct", direct);
    flag("dual", dual);
    flag("held", held);
    flag("swtrig", triggered);
    let ok = direct && dual && held && triggered;
    verdict(ok);
    ok
}

/// The RNG runs on HSI48 (RM0376 §22.3): with RNGEN on the bus clock but
/// HSI48 off, the block stays dead (CR does not take RNGEN, DR reads 0).
/// With HSI48 on, DRDY rises and two DR words differ.
fn check_rng() -> bool {
    wr(RCC_AHBENR, rd(RCC_AHBENR) | RCC_AHBENR_RNGEN);
    wr(RNG_CR, RNG_CR_RNGEN);
    let dead = rd(RNG_CR) == 0 && rd(RNG_SR) & 1 == 0 && rd(RNG_DR) == 0;

    wr(RCC_CRRCR, 1); // HSI48ON
    let hsi48 = wait(RCC_CRRCR, 0b10, 0b10);
    wr(RNG_CR, RNG_CR_RNGEN);
    let ready = wait(RNG_SR, 1, 1);
    let a = rd(RNG_DR);
    let b = rd(RNG_DR);
    let draws = ready && a != 0 && b != 0 && a != b;
    wr(RNG_CR, 0);

    puts("rng ");
    flag("dead", dead);
    flag("hsi48", hsi48);
    flag("draws", draws);
    let ok = dead && hsi48 && draws;
    verdict(ok);
    ok
}

/// The CHSELR ADC: calibration (ADCAL self-clears, EOCAL), ADEN -> ADRDY,
/// then one software-triggered sequence over IN1/IN9/IN17/IN18 read EOC by
/// EOC (EOS ends it, ADSTART self-clears, EOSIE pends line 12). The same
/// sequence at 8 bits in the descending scan direction goes through DMA1
/// channel 1 (CSELR C1S = 0). Unread results overrun (OVR, first result
/// kept) and the analog watchdog on IN1 trips above HT = 0x700.
fn check_adc() -> bool {
    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | RCC_APB2ENR_ADCEN);
    // PA1 and PB1 analog (PB1 is already analog out of reset).
    wr(GPIOA + G_MODER, rd(GPIOA + G_MODER) | (0b11 << 2));
    wr(GPIOB + G_MODER, rd(GPIOB + G_MODER) | (0b11 << 2));

    wr(ADC_CR, ADC_CR_ADCAL);
    let cal = wait(ADC_CR, ADC_CR_ADCAL, 0) && rd(ADC_ISR) & ADC_ISR_EOCAL != 0;
    wr(ADC_ISR, ADC_ISR_EOCAL);
    wr(ADC_CCR, ADC_CCR_VREFEN | ADC_CCR_TSEN);
    wr(ADC_CR, ADC_CR_ADEN);
    let ready = wait(ADC_ISR, ADC_ISR_ADRDY, ADC_ISR_ADRDY);
    wr(ADC_ISR, ADC_ISR_ADRDY);
    // 160.5-clock sampling: what the internal channels need on silicon.
    wr(ADC_SMPR, 7);
    wr(ADC_CHSELR, ADC_SEQ);
    unpend(ADC_LINE);

    // Polled 12-bit sequence, EOSIE on.
    wr(ADC_IER, ADC_ISR_EOS);
    wr(ADC_CFGR1, 0);
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADSTART);
    let mut got = [0u32; 4];
    let mut eoc = true;
    for slot in got.iter_mut() {
        eoc &= wait(ADC_ISR, ADC_ISR_EOC, ADC_ISR_EOC);
        *slot = rd(ADC_DR);
    }
    let seq = eoc && got == ADC_WANT;
    let eos = wait(ADC_ISR, ADC_ISR_EOS, ADC_ISR_EOS)
        && rd(ADC_CR) & ADC_CR_ADSTART == 0
        && rd(ADC_ISR) & (ADC_ISR_EOC | ADC_ISR_OVR) == 0;
    let pend = pending(ADC_LINE);
    wr(ADC_ISR, ADC_ISR_EOS);
    wr(ADC_IER, 0);
    unpend(ADC_LINE);
    let released = !pending(ADC_LINE);

    // 8-bit, descending, through DMA1 channel 1.
    let buf = DMA_BUF;
    for i in 0..4 {
        wr(buf + 4 * i, 0xFFFF_FFFF);
    }
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    wr(DMA1 + D_CSELR, 0);
    let ch1 = dma_ch(1);
    wr(ch1 + C_CPAR, ADC_DR);
    wr(ch1 + C_CMAR, buf);
    wr(ch1 + C_CNDTR, 4);
    // MSIZE = PSIZE = 32 | MINC | EN, peripheral -> memory.
    wr(ch1 + C_CCR, (0b10 << 10) | (0b10 << 8) | (1 << 7) | 1);
    wr(
        ADC_CFGR1,
        ADC_CFGR1_DMAEN | ADC_CFGR1_SCANDIR | ADC_CFGR1_RES_8,
    );
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADSTART);
    let moved = wait(DMA1 + D_ISR, 0b10, 0b10);
    let dma_vals = [rd(buf), rd(buf + 4), rd(buf + 8), rd(buf + 12)];
    let want8 = [
        ADC_WANT[3] >> 4,
        ADC_WANT[2] >> 4,
        ADC_WANT[1] >> 4,
        ADC_WANT[0] >> 4,
    ];
    let dma = moved && dma_vals == want8 && rd(ADC_ISR) & ADC_ISR_OVR == 0;
    wr(ch1 + C_CCR, 0);
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    for &irq in &DMA_LINES {
        unpend(irq);
    }

    // Nobody reads DR: the second result overruns and the first stays.
    // The watchdog guards IN1 only, with HT = 0x700.
    wr(ADC_ISR, 0xFFF);
    wr(ADC_CHSELR, (1 << 1) | (1 << 9));
    wr(ADC_TR, 0x0700 << 16);
    wr(ADC_CFGR1, ADC_CFGR1_AWDEN | ADC_CFGR1_AWDSGL | (1 << 26));
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADSTART);
    let _ = wait(ADC_ISR, ADC_ISR_EOS, ADC_ISR_EOS);
    let isr = rd(ADC_ISR);
    let ovr = isr & ADC_ISR_OVR != 0 && rd(ADC_DR) == ADC_WANT[0];
    let awd = isr & ADC_ISR_AWD != 0;
    wr(ADC_ISR, 0xFFF);
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADDIS);
    let off = wait(ADC_CR, ADC_CR_ADEN, 0);

    puts("adc in1=");
    hex(got[0]);
    puts(" ts=");
    hex(got[3]);
    puts(" ");
    flag("cal", cal);
    flag("ready", ready);
    flag("seq", seq);
    flag("eos", eos);
    flag("pend", pend);
    flag("released", released);
    flag("dma", dma);
    flag("ovr", ovr);
    flag("awd", awd);
    flag("off", off);
    let ok = cal && ready && seq && eos && pend && released && dma && ovr && awd && off;
    verdict(ok);
    ok
}

pub fn run(variant: Variant) -> ! {
    let (rcc_reset, rcc_gate) = rcc_bringup(variant);
    for &(_, base, _) in &USARTS {
        uart_init(base);
    }
    puts(match variant {
        Variant::L071 => "STM32L071 Cortex-M0+ smoke\r\n",
        Variant::L072 => "STM32L072 Cortex-M0+ smoke\r\n",
    });
    send(USART2, b"usart2 ok\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate));
    mark(1, check_usart_irqs());
    mark(2, check_gpio());
    mark(3, check_exti_swier());
    for (i, &(name, base, irq)) in TIMERS.iter().enumerate() {
        mark(4 + i as u32, check_timer(name, base, irq));
    }
    mark(10, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all (an
    // undriven CS pad leaves its device on the bus).
    cs_output(GPIOB, 0);
    for (i, &(name, base, irq, port, pin, want)) in SPIS.iter().enumerate() {
        mark(11 + i as u32, check_spi(name, base, irq, port, pin, want));
    }
    mark(
        13,
        check_dma_spi("spi1", SPI1, (2, 3), 1, 10, (GPIOA, 4, SPIS[0].5)),
    );
    mark(
        14,
        check_dma_spi("spi2", SPI2, (4, 5), 2, 11, (GPIOB, 12, SPIS[1].5)),
    );
    for (i, &(name, base, line)) in I2CS.iter().enumerate() {
        mark(15 + i as u32, check_i2c(name, base, line));
    }
    mark(18, check_spi1_shared_cs());
    mark(19, check_crc());
    mark(22, check_adc());
    let tag = if variant == Variant::L072 {
        mark(20, check_dac());
        mark(21, check_rng());
        0x7200_0000
    } else {
        0x7100_0000
    };
    let summary = summary | tag;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
