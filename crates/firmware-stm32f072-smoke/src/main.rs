#![no_std]
#![no_main]

//! STM32F072xB smoke for the imported Renode platform: RCC clock gates,
//! USART1..4, GPIOA..F, SYSCFG EXTICR + EXTI, TIM1/2/3/6/7/14/15/16/17,
//! SPI1/SPI2 (FIFO SPI), I2C1 ("v2" I2C), DMA1, CRC and the CHSELR ADC. Register offsets,
//! bases and IRQ numbers are RM0091 and ST's CMSIS `stm32f072xb.h`.
//!
//! The fixture system puts a MAX31855 on each SPI (thermocouple 25 and
//! 100 °C, so each bus answers with its own frame), a second MAX31855 on SPI1
//! behind its own chip select (PB0, 50 °C), a TMP102 at 0x48 on I2C1 and
//! potentiometers on ADC IN1 (1.65 V) and IN9 (0.825 V).
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

const SUMMARY: u32 = 0x2000_3000;

// RCC (RM0091 §6.4).
const RCC: u32 = 0x4002_1000;
const RCC_CR: u32 = RCC;
const RCC_AHBENR: u32 = RCC + 0x14;
const RCC_APB2ENR: u32 = RCC + 0x18;
const RCC_APB1ENR: u32 = RCC + 0x1C;
// DMA1, SRAM, FLITF, CRC, IOPA..IOPF.
const AHB_ON: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (0x3F << 17);
// SYSCFG, ADC, TIM1, SPI1, USART1, TIM15/16/17.
const APB2_ON: u32 = (1 << 0) | (1 << 9) | (1 << 11) | (1 << 12) | (1 << 14) | (0b111 << 16);
// TIM2/3/6/7/14, SPI2, USART2/3/4, I2C1.
const APB1_ON: u32 = 0b11 | (0b11 << 4) | (1 << 8) | (1 << 14) | (0b111 << 17) | (1 << 21);

// USART "v2" layout.
const USART1: u32 = 0x4001_3800;
const USART2: u32 = 0x4000_4400;
const USART3: u32 = 0x4000_4800;
const USART4: u32 = 0x4000_4C00;
const U_CR1: u32 = 0x00;
const U_BRR: u32 = 0x0C;
const U_ISR: u32 = 0x1C;
const U_TDR: u32 = 0x28;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_TXEIE: u32 = 1 << 7;

// GPIO (AHB2).
const GPIOA: u32 = 0x4800_0000;
const GPIOB: u32 = 0x4800_0400;
const GPIOC: u32 = 0x4800_0800;
const GPIOD: u32 = 0x4800_0C00;
const GPIOE: u32 = 0x4800_1000;
const GPIOF: u32 = 0x4800_1400;
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
// SYSCFG_EXTICRx port codes (stm32f072xb.h SYSCFG_EXTICR1_EXTI0_Px).
const CODE_PA: u32 = 0;
const CODE_PC: u32 = 2;
const CODE_PF: u32 = 5;

// TIMx.
const T_CR1: u32 = 0x00;
const T_DIER: u32 = 0x0C;
const T_SR: u32 = 0x10;
const T_CNT: u32 = 0x24;
const T_PSC: u32 = 0x28;
const T_ARR: u32 = 0x2C;

/// (name, base, NVIC line, counter width in bits) — `stm32f072xb.h`
/// TIMx_BASE / TIMx_IRQn; TIM1's update raises TIM1_BRK_UP_TRG_COM_IRQn.
const TIMERS: [(&str, u32, u32, u32); 9] = [
    ("tim1", 0x4001_2C00, 13, 16),
    ("tim2", 0x4000_0000, 15, 32),
    ("tim3", 0x4000_0400, 16, 16),
    ("tim6", 0x4000_1000, 17, 16),
    ("tim7", 0x4000_1400, 18, 16),
    ("tim14", 0x4000_2000, 19, 16),
    ("tim15", 0x4001_4000, 20, 16),
    ("tim16", 0x4001_4400, 21, 16),
    ("tim17", 0x4001_4800, 22, 16),
];

// SPIx (FIFO SPI: CR2.DS[11:8], FRXTH bit 12; DR is accessed by byte).
const S_CR1: u32 = 0x00;
const S_CR2: u32 = 0x04;
const S_SR: u32 = 0x08;
const S_DR: u32 = 0x0C;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;
const SPI_SR_BSY: u32 = 1 << 7;
const SPI_SR_FRLVL: u32 = 0b11 << 9;
/// FRXTH (RXNE at one byte) | DS = 8 bits.
const SPI_CR2_8BIT: u32 = (1 << 12) | (0b0111 << 8);
const SPI1: u32 = 0x4001_3000;

/// (name, base, NVIC line, CS port, CS pin, expected MAX31855 frame).
/// Frames: thermocouple << 18 (0.25 °C/LSB) | internal 22 °C << 4.
/// The second SPI1 slave (CS PB0): 50 °C thermocouple, 22 °C cold junction.
const SPI1_B_FRAME: u32 = 0x0320_1600;
const SPIS: [(&str, u32, u32, u32, u32, u32); 2] = [
    ("spi1", SPI1, 25, GPIOA, 4, 0x0190_1600),
    ("spi2", 0x4000_3800, 26, GPIOB, 12, 0x0640_1600),
];

// DMA1: ISR, IFCR, then channel n at 0x08 + 20 * (n - 1).
const DMA1: u32 = 0x4002_0000;
const D_ISR: u32 = 0x00;
const D_IFCR: u32 = 0x04;
const fn dma_ch(n: u32) -> u32 {
    DMA1 + 0x08 + 20 * (n - 1)
}
const C_CCR: u32 = 0x00;
const C_CNDTR: u32 = 0x04;
const C_CPAR: u32 = 0x08;
const C_CMAR: u32 = 0x0C;
const DMA_BUF: u32 = 0x2000_3100;

// I2C1 ("v2" IP).
const I2C1: u32 = 0x4000_5400;
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
const I2C_LINE: u32 = 23;
const TMP102: u32 = 0x48;

// CRC.
const CRC: u32 = 0x4002_3000;
const CRC_DR: u32 = CRC;
const CRC_CR: u32 = CRC + 0x08;
const CRC_INIT: u32 = CRC + 0x10;

// ADC (RM0091 §13.12).
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
// IN1 (pot, 1.65 V), IN9 (pot, 0.825 V), IN16 (TS), IN17 (V_REFINT).
const ADC_SEQ: u32 = (1 << 1) | (1 << 9) | (1 << 16) | (1 << 17);
// 12-bit codes at V_DDA = 3.3 V: TS at 25 °C is 1.43 V + 5 K * 4.3 mV/K
// (DS9826 V_30 / Avg_Slope), V_REFINT 1.23 V.
const ADC_WANT: [u32; 4] = [2047, 1023, 1801, 1526];
const DMA_LINES: [u32; 3] = [9, 10, 11];

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;
const SYSTICK_CALIB: u32 = 0xE000_E01C;
const SCB_ICSR: u32 = 0xE000_ED04;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;

static mut SYSTICK_EXTERNAL_ENTRIES: u32 = 0;
static mut SYSTICK_EXTERNAL_ACTIVE_MASK: u32 = 0;
static mut SYSTICK_EXTERNAL_COUNTFLAG_MASK: u32 = 0;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn rd8(addr: u32) -> u8 {
    unsafe { read_volatile(addr as *const u8) }
}

fn wr8(addr: u32, value: u8) {
    unsafe { write_volatile(addr as *mut u8, value) }
}

fn pending(irq: u32) -> bool {
    rd(NVIC_ISPR0) & (1 << irq) != 0
}

fn unpend(irq: u32) {
    wr(NVIC_ICPR0, 1 << irq);
}

/// Wait briefly for an enabled level source to be reflected in ISPR after a
/// pending clear or enable write. Each volatile read advances the simulated
/// core and gives the peripheral/NVIC level path a chance to settle.
fn wait_pending(irq: u32, want: bool) -> bool {
    for _ in 0..128 {
        if pending(irq) == want {
            return true;
        }
    }
    false
}

/// Hold a disabled source under observation long enough to catch a stale
/// pending bit or a level that reasserts while its interrupt enable is clear.
fn pending_stays(irq: u32, want: bool) -> bool {
    let mut stayed = true;
    for _ in 0..16 {
        stayed &= pending(irq) == want;
    }
    stayed
}

fn uart_init(base: u32) {
    wr(base + U_BRR, 0x45);
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

/// RCC reset state (HSI on and ready, AHBENR = SRAM | FLITF) and a clock
/// gate that holds: GPIOD ignores ODR writes until IOPDEN is set. Runs
/// before anything else, so USART1 is still gated and the result is
/// reported later.
fn rcc_bringup() -> (bool, bool) {
    let reset = rd(RCC_CR) & 0b11 == 0b11 && rd(RCC_AHBENR) == 0x14;
    wr(GPIOD + G_ODR, 0x00A5);
    let gated = rd(GPIOD + G_ODR) == 0;
    wr(RCC_AHBENR, AHB_ON);
    wr(RCC_APB2ENR, APB2_ON);
    wr(RCC_APB1ENR, APB1_ON);
    wr(GPIOD + G_ODR, 0x00A5);
    let gate = gated && rd(GPIOD + G_ODR) == 0x00A5 && rd(RCC_AHBENR) == AHB_ON;
    wr(GPIOD + G_ODR, 0);
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

/// TXEIE on USART2..4 raises each one's own line; USART3 and USART4 share
/// USART3_4_IRQn = 29.
fn check_usart_irqs() -> bool {
    let mut ok = true;
    puts("usart irq ");
    for &(name, base, irq) in &[
        ("u2", USART2, 28u32),
        ("u3", USART3, 29),
        ("u4", USART4, 29),
    ] {
        unpend(irq);
        let quiet = !pending(irq);
        wr(base + U_CR1, rd(base + U_CR1) | USART_CR1_TXEIE);
        let pend = pending(irq);
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

/// RM0091 reset values of the SWD pads on port A, and set/reset through
/// BSRR/BRR on ports B..F.
fn check_gpio() -> bool {
    let pa = rd(GPIOA + G_MODER) == 0x2800_0000 && rd(GPIOA + G_PUPDR) == 0x2400_0000;
    let others = [GPIOB, GPIOC, GPIOD, GPIOE, GPIOF]
        .iter()
        .all(|&p| rd(p + G_MODER) == 0 && rd(p + G_PUPDR) == 0);
    // PF0 output; toggle it four times for the gpio_edges assertion.
    wr(GPIOF + G_MODER, 0b01);
    let mut toggles = true;
    for _ in 0..2 {
        wr(GPIOF + G_BSRR, 1);
        toggles &= rd(GPIOF + G_ODR) & 1 == 1 && rd(GPIOF + G_IDR) & 1 == 1;
        wr(GPIOF + G_BRR, 1);
        toggles &= rd(GPIOF + G_ODR) & 1 == 0;
    }
    // Each port keeps its own ODR.
    wr(GPIOD + G_ODR, 0x00F0);
    wr(GPIOE + G_ODR, 0x0F00);
    let de = rd(GPIOD + G_ODR) == 0x00F0 && rd(GPIOE + G_ODR) == 0x0F00 && rd(GPIOF + G_ODR) == 0;
    wr(GPIOD + G_ODR, 0);
    wr(GPIOE + G_ODR, 0);
    puts("gpio ");
    flag("pa", pa);
    flag("reset", others);
    flag("pf", toggles);
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

/// Falling edges from the two fixture buttons (PF1, PC13; pressed = low)
/// through the SYSCFG port mux. Line 1 first selects code 0 (PA): the first
/// PF1 press must not pend it. With code 5 (PF) the second press must, on
/// NVIC EXTI0_1 = 5. PC13 lands on EXTI4_15 = 7.
fn check_exti_pads() -> bool {
    // Inputs with pull-up.
    wr(GPIOF + G_PUPDR, 0b01 << 2);
    wr(GPIOC + G_PUPDR, 0b01 << 26);
    wr(SYSCFG_EXTICR1, CODE_PA << 4);
    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    wr(EXTI + E_FTSR, (1 << 1) | (1 << 13));
    wr(EXTI + E_IMR, (1 << 1) | (1 << 13));

    let idle = wait_level(GPIOF, 1, true) && wait_level(GPIOC, 13, true);
    let press1 = wait_level(GPIOF, 1, false) && wait_level(GPIOF, 1, true);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << 1) == 0 && !pending(5);

    wr(SYSCFG_EXTICR1, CODE_PF << 4);
    let press2 = wait_level(GPIOF, 1, false) && wait_level(GPIOF, 1, true);
    let pf = rd(EXTI + E_PR) & (1 << 1) != 0 && pending(5);
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
    flag("pf1", pf);
    flag("pc13", pc);
    let ok = idle && press1 && press2 && press3 && other_port_quiet && pf && pc;
    verdict(ok);
    ok
}

/// Counter width (ARR keeps 16 or 32 bits), counting, and the update
/// interrupt reaching the timer's own NVIC line — and no other timer's.
fn check_timer(name: &str, base: u32, irq: u32, width: u32) -> bool {
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

/// Master, software NSS (SSM | SSI), fPCLK/8, mode 0, 8-bit frames with
/// the RXNE threshold at one byte.
fn spi_enable(base: u32) {
    wr(base + S_CR2, SPI_CR2_8BIT);
    wr(base + S_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(base + S_CR1, rd(base + S_CR1) | (1 << 6)); // SPE
}

fn spi_xfer(base: u32, out: u8) -> Option<u8> {
    if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
        return None;
    }
    wr8(base + S_DR, out);
    if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
        return None;
    }
    Some(rd8(base + S_DR))
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

/// Read a frame with RXNEIE active. The first byte checks IRQ assertion,
/// re-pending after NVIC clear, masking while RXNE remains set, re-enabling,
/// and release after the DR read.
fn max31855_frame_with_irq(
    base: u32,
    irq: u32,
    port: u32,
    pin: u32,
) -> Option<(u32, bool, bool, bool, bool, bool)> {
    cs(port, pin, true);
    let mut word = 0u32;
    let mut irq_seen = false;
    let mut irq_repended = false;
    let mut irq_masked = false;
    let mut irq_unmasked = false;
    for byte_index in 0..4 {
        if !wait(base + S_SR, SPI_SR_TXE, SPI_SR_TXE) {
            cs(port, pin, false);
            return None;
        }
        wr8(base + S_DR, 0);
        if !wait(base + S_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
            cs(port, pin, false);
            return None;
        }
        let rxne = rd(base + S_SR) & SPI_SR_RXNE != 0;
        irq_seen |= pending(irq) && rxne;
        if byte_index == 0 {
            // RXNE is a level source: clearing NVIC pending while RXNE and
            // RXNEIE remain asserted must let the line pend again.
            unpend(irq);
            irq_repended = wait_pending(irq, true) && rd(base + S_SR) & SPI_SR_RXNE != 0;

            // Masking RXNEIE must suppress the line without consuming DR;
            // restoring it must expose the still-active source again.
            wr(base + S_CR2, SPI_CR2_8BIT);
            unpend(irq);
            irq_masked = rd(base + S_SR) & SPI_SR_RXNE != 0 && pending_stays(irq, false);
            wr(base + S_CR2, SPI_CR2_8BIT | (1 << 6)); // RXNEIE
            irq_unmasked = wait_pending(irq, true) && rd(base + S_SR) & SPI_SR_RXNE != 0;
        }
        word = (word << 8) | u32::from(rd8(base + S_DR));
    }
    let idle = wait(base + S_SR, SPI_SR_BSY, 0);
    cs(port, pin, false);
    let released = !pending(irq)
        && rd(base + S_SR) & (SPI_SR_RXNE | SPI_SR_FRLVL) == 0
        && rd(base + S_CR2) & (1 << 6) != 0;
    idle.then_some((
        word,
        irq_seen,
        irq_repended,
        irq_masked,
        irq_unmasked,
        released,
    ))
}

/// Each SPI reads its own device's frame, then verifies the FIFO SPI's TXEIE
/// and RXNEIE levels on its own NVIC line. TXEIE must release when disabled;
/// the DR read must release RXNE while RXNEIE remains enabled.
fn check_spi(name: &str, base: u32, irq: u32, port: u32, pin: u32, want: u32) -> bool {
    cs_output(port, pin);
    spi_enable(base);
    let ds = rd(base + S_CR2) & 0x1F00 == SPI_CR2_8BIT;
    let frame = max31855_frame(base, port, pin).unwrap_or(0xDEAD_BEEF);
    unpend(irq);
    let quiet = !pending(irq);

    wr(base + S_CR2, SPI_CR2_8BIT | (1 << 7)); // TXEIE
    let txe_irq = rd(base + S_SR) & SPI_SR_TXE != 0 && pending(irq);
    wr(base + S_CR2, SPI_CR2_8BIT);
    let txe_released = !pending(irq);

    wr(base + S_CR2, SPI_CR2_8BIT | (1 << 6)); // RXNEIE
    let again = max31855_frame_with_irq(base, irq, port, pin);
    let again_frame = again.map(|(frame, _, _, _, _, _)| frame);
    let rxne_irq = again.is_some_and(|(_, seen, _, _, _, _)| seen);
    let rxne_repended = again.is_some_and(|(_, _, repended, _, _, _)| repended);
    let rxne_masked = again.is_some_and(|(_, _, _, masked, _, _)| masked);
    let rxne_unmasked = again.is_some_and(|(_, _, _, _, unmasked, _)| unmasked);
    let rxne_released = again.is_some_and(|(_, _, _, _, _, released)| released);
    let data = frame == want && again_frame == Some(want);
    let pend = quiet && rxne_irq && rxne_repended && rxne_masked && rxne_unmasked;
    let alone = SPIS
        .iter()
        .all(|&(_, _, other, _, _, _)| other == irq || !pending(other));
    wr(base + S_CR2, SPI_CR2_8BIT);
    let released = rxne_released && !pending(irq);

    puts(name);
    puts(" frame=");
    hex(frame);
    puts(" ");
    flag("ds", ds);
    flag("data", data);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = ds && data && pend && alone && released && txe_irq && txe_released;
    verdict(ok);
    puts(name);
    puts(" TXEIE ");
    flag("pend", txe_irq);
    flag("released", txe_released);
    verdict(txe_irq && txe_released);
    puts(name);
    puts(" RXNEIE ");
    flag("asserted", rxne_irq);
    flag("repended", rxne_repended);
    flag("masked", rxne_masked);
    flag("unmasked", rxne_unmasked);
    flag("released", rxne_released);
    verdict(rxne_irq && rxne_repended && rxne_masked && rxne_unmasked && rxne_released);
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

/// SPI1 full duplex through DMA1: channel 3 (SPI1_TX) feeds four zero
/// bytes, channel 2 (SPI1_RX) stores the frame. Only channel 2 has TCIE;
/// channels 2 and 3 share DMA1_Channel2_3_IRQn (10), so 10 pends and
/// neither 9 (channel 1) nor 11 (channels 4..7) does.
fn check_dma_spi1() -> bool {
    let rx = DMA_BUF;
    let tx = DMA_BUF + 0x10;
    wr(rx, 0xFFFF_FFFF);
    wr(tx, 0);
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    for irq in 9..=11 {
        unpend(irq);
    }
    let ch2 = dma_ch(2);
    let ch3 = dma_ch(3);
    wr(ch2 + C_CPAR, SPI1 + S_DR);
    wr(ch2 + C_CMAR, rx);
    wr(ch2 + C_CNDTR, 4);
    wr(ch2 + C_CCR, (1 << 7) | (1 << 1) | 1); // MINC | TCIE | EN, P->M
    wr(ch3 + C_CPAR, SPI1 + S_DR);
    wr(ch3 + C_CMAR, tx);
    wr(ch3 + C_CNDTR, 4);
    wr(ch3 + C_CCR, (1 << 7) | (1 << 4) | 1); // MINC | DIR | EN, M->P

    cs(GPIOA, 4, true);
    wr(SPI1 + S_CR2, SPI_CR2_8BIT | (1 << 1) | 1); // TXDMAEN | RXDMAEN
    let done = wait(DMA1 + D_ISR, 1 << 5, 1 << 5); // TCIF2
    let _ = wait(SPI1 + S_SR, SPI_SR_BSY, 0);
    cs(GPIOA, 4, false);
    wr(SPI1 + S_CR2, SPI_CR2_8BIT);

    let b = rd(rx).to_le_bytes();
    let frame = u32::from_be_bytes(b);
    let data = frame == SPIS[0].5;
    let isr = rd(DMA1 + D_ISR);
    // GIF | TCIF | HTIF (four items cross the half-way mark) on both channels.
    let flags = isr & 0xFF0 == 0x770 && rd(ch2 + C_CNDTR) == 0 && rd(ch3 + C_CNDTR) == 0;
    let pend = done && pending(10);
    let alone = !pending(9) && !pending(11);
    wr(DMA1 + D_IFCR, 0xFF0);
    unpend(10);
    let released = !pending(10) && rd(DMA1 + D_ISR) == 0;
    wr(ch2 + C_CCR, 0);
    wr(ch3 + C_CCR, 0);

    puts("dma1 spi1 frame=");
    hex(frame);
    puts(" ");
    flag("data", data);
    flag("flags", flags);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = data && flags && pend && alone && released;
    verdict(ok);
    ok
}

fn i2c_cr2(addr: u32, nbytes: u32, extra: u32) -> u32 {
    (addr << 1) | (nbytes << 16) | extra
}

/// TMP102 temperature register (pointer 0): one-byte write of the pointer
/// (software end, TC), then a two-byte read with AUTOEND.
fn tmp102_read() -> Option<u16> {
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

/// I2C1 master: OAR1 readback, a TMP102 read (25 °C = 0x1900), RXIE
/// raising the combined line 23 during a second read (25.5 °C), and an absent address
/// NACKing (NACKF under NACKIE, also line 23), then ICR clearing both.
fn check_i2c1() -> bool {
    wr(I2C1 + I_OAR1, (1 << 15) | 0xAA);
    let oar = rd(I2C1 + I_OAR1) & 0xFFFF == 0x80AA;
    wr(I2C1 + I_OAR1, 0);
    wr(I2C1 + I_TIMINGR, 0x2000_090E); // 100 kHz from 8 MHz HSI
    wr(I2C1 + I_CR1, I2C_CR1_PE);
    let temp = tmp102_read();
    let data = temp == Some(0x1900);

    unpend(I2C_LINE);
    let quiet = !pending(I2C_LINE);
    wr(I2C1 + I_CR1, I2C_CR1_PE | I2C_CR1_RXIE);
    let again = tmp102_read();
    // The TMP102 model self-drifts +0.5 °C (0x80) per full temperature read
    // (configs/devices/tmp102.yaml), so the second read is 25.5 °C.
    let rx_pend = quiet && again == Some(0x1980) && pending(I2C_LINE);
    wr(I2C1 + I_CR1, I2C_CR1_PE);
    unpend(I2C_LINE);

    wr(I2C1 + I_CR1, I2C_CR1_PE | I2C_CR1_NACKIE);
    wr(
        I2C1 + I_CR2,
        i2c_cr2(0x50, 1, I2C_CR2_AUTOEND | I2C_CR2_START),
    );
    let nack = wait(I2C1 + I_ISR, I2C_ISR_NACKF, I2C_ISR_NACKF);
    let nack_pend = nack && pending(I2C_LINE);
    let stopped = wait(I2C1 + I_ISR, I2C_ISR_STOPF, I2C_ISR_STOPF);
    wr(I2C1 + I_ICR, I2C_ISR_NACKF | I2C_ISR_STOPF);
    wr(I2C1 + I_CR1, I2C_CR1_PE);
    unpend(I2C_LINE);
    let released = rd(I2C1 + I_ISR) & (I2C_ISR_NACKF | I2C_ISR_STOPF | I2C_ISR_BUSY) == 0
        && !pending(I2C_LINE);

    puts("i2c1 tmp102=");
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

/// The CHSELR ADC: calibration, ADRDY, a polled 12-bit sequence (two pots,
/// TS, V_REFINT) with EOS on ADC1_COMP_IRQn, then 8-bit descending through
/// DMA1 channel 1, an overrun that keeps the first result, the single-
/// channel watchdog and ADDIS.
fn check_adc() -> bool {
    // PA1 and PB1 analog.
    wr(GPIOA + G_MODER, rd(GPIOA + G_MODER) | (0b11 << 2));
    wr(GPIOB + G_MODER, rd(GPIOB + G_MODER) | (0b11 << 2));

    wr(ADC_CR, ADC_CR_ADCAL);
    let cal = wait(ADC_CR, ADC_CR_ADCAL, 0);
    wr(ADC_CCR, ADC_CCR_VREFEN | ADC_CCR_TSEN);
    wr(ADC_CR, ADC_CR_ADEN);
    let ready = wait(ADC_ISR, ADC_ISR_ADRDY, ADC_ISR_ADRDY);
    wr(ADC_ISR, ADC_ISR_ADRDY);
    // 239.5-clock sampling: what the internal channels need on silicon.
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
    hex(got[2]);
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

/// Disabling SysTick freezes its in-progress count; re-enabling resumes from
/// that count instead of starting a fresh RELOAD period.
fn check_systick_pause_resume() -> bool {
    const RELOAD: u32 = 2047;

    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_RVR, RELOAD);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x5); // ENABLE | processor clock; TICKINT stays clear.

    // Catch the counter partway through its period, with ample distance from
    // both zero and RELOAD so a restart is distinguishable from a resume.
    let mut pause_window = false;
    for _ in 0..10_000 {
        let value = rd(SYSTICK_CVR);
        if (1024..=1792).contains(&value) {
            pause_window = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    wr(SYSTICK_CSR, 0x4); // Disable while preserving CLKSOURCE.
    let paused = rd(SYSTICK_CVR);
    for _ in 0..64 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let frozen = rd(SYSTICK_CVR);

    wr(SYSTICK_CSR, 0x5);
    for _ in 0..64 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let resumed = rd(SYSTICK_CVR);
    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_CVR, 0);

    let frozen_ok = paused == frozen;
    let resumed_ok = pause_window && paused > 256 && resumed > 0 && resumed < paused;
    let ok = frozen_ok && resumed_ok;

    puts("systick pause-resume ");
    flag("window", pause_window);
    flag("frozen", frozen_ok);
    flag("resumed", resumed_ok);
    puts("paused=");
    hex(paused);
    puts(" frozen_at=");
    hex(frozen);
    puts(" resumed_at=");
    hex(resumed);
    puts(" ");
    verdict(ok);
    ok
}

/// RM0091 §11.1.2 specifies TENMS=6000 for the 6 MHz HCLK/8 SysTick source.
/// The CALIB bits also identify the reference source and its calibration skew.
fn check_systick_calibration() -> bool {
    let calib = rd(SYSTICK_CALIB);
    let tenms = calib & 0x00FF_FFFF;
    let skew = calib & (1 << 30) != 0;
    let noref = calib & (1 << 31) != 0;
    let ok = calib == 0x4000_1770 && tenms == 6000 && skew && !noref;

    puts("systick calib=");
    hex(calib);
    puts(" tenms=");
    hex(tenms);
    puts(" skew=");
    puts(if skew { "1" } else { "0" });
    puts(" noref=");
    puts(if noref { "1" } else { "0" });
    puts(if ok { " ok\r\n" } else { " bad\r\n" });
    ok
}

/// The STM32F072 reference source is HCLK/8. ENABLE must freeze that
/// countdown and re-enabling it must continue the same in-progress period.
fn check_systick_external_pause_resume() -> bool {
    const RELOAD: u32 = 4095;

    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_RVR, RELOAD);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x1); // ENABLE | reference clock; TICKINT stays clear.
    let reference_selected = rd(SYSTICK_CSR) & 0x5 == 0x1;

    let mut pause_window = false;
    for _ in 0..20_000 {
        let value = rd(SYSTICK_CVR);
        if (1024..=3072).contains(&value) {
            pause_window = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    wr(SYSTICK_CSR, 0); // Disable without selecting the processor clock.
    let paused = rd(SYSTICK_CVR);
    for _ in 0..128 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let frozen = rd(SYSTICK_CVR);

    wr(SYSTICK_CSR, 0x1);
    for _ in 0..128 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let resumed = rd(SYSTICK_CVR);
    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_CVR, 0);

    let frozen_ok = paused == frozen;
    let resumed_ok = pause_window && paused > 256 && resumed > 0 && resumed < paused;
    let ok = reference_selected && frozen_ok && resumed_ok;

    puts("systick external pause-resume ");
    flag("reference", reference_selected);
    flag("window", pause_window);
    flag("frozen", frozen_ok);
    flag("resumed", resumed_ok);
    puts("paused=");
    hex(paused);
    puts(" frozen_at=");
    hex(frozen);
    puts(" resumed_at=");
    hex(resumed);
    puts(" ");
    verdict(ok);
    ok
}

/// The STM32F072 HCLK/8 reference source must repeatedly pend SysTick and
/// enter exception 15 through the scheduler-driven event path.
fn check_systick_external_periodic() -> bool {
    const WANT_ENTRIES: u32 = 3;

    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    wr(SYSTICK_RVR, 127);
    wr(SYSTICK_CVR, 0);
    unsafe {
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ENTRIES), 0);
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ACTIVE_MASK), 0);
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_COUNTFLAG_MASK), 0);
    }

    // ENABLE | TICKINT, with CLKSOURCE clear for the HCLK/8 reference.
    wr(SYSTICK_CSR, 0x3);
    let reference_selected = rd(SYSTICK_CSR) & 0x5 == 0x1;
    let mut remaining = 100_000;
    while unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) } < WANT_ENTRIES
        && remaining > 0
    {
        remaining -= 1;
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    // Stop after the third handler has returned so a fourth periodic event
    // cannot turn this into a threshold-only check.
    wr(SYSTICK_CSR, 0);
    let entries = unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) };
    let active_mask = unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ACTIVE_MASK)) };
    let countflag_mask =
        unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_COUNTFLAG_MASK)) };
    let pending = rd(SCB_ICSR) & ICSR_PENDSTSET != 0;
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    wr(SYSTICK_CVR, 0);

    let active_each = active_mask == (1 << WANT_ENTRIES) - 1;
    let countflag_each = countflag_mask == (1 << WANT_ENTRIES) - 1;
    let ok =
        reference_selected && entries == WANT_ENTRIES && active_each && countflag_each && !pending;

    puts("systick external periodic ");
    flag("reference", reference_selected);
    puts("entries=");
    hex(entries);
    puts(" ");
    flag("active", active_each);
    flag("countflag", countflag_each);
    flag("pending", pending);
    verdict(ok);
    ok
}

/// Several HCLK/8 zero-hits while PRIMASK is set must coalesce into one
/// pending SysTick exception, delivered once after interrupts are restored.
fn check_systick_external_pending_coalesces() -> bool {
    const RELOAD: u32 = 63;
    const COUNTFLAG: u32 = 1 << 16;
    const WANT_WRAPS: u32 = 3;

    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nostack, preserves_flags),
        );
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, ICSR_PENDSTCLR);
        wr(SYSTICK_RVR, RELOAD);
        wr(SYSTICK_CVR, 0);
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ENTRIES), 0);
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ACTIVE_MASK), 0);
        write_volatile(core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_COUNTFLAG_MASK), 0);
        wr(SYSTICK_CSR, 0x3); // ENABLE | TICKINT; HCLK/8 reference source.
    }
    let reference_selected = rd(SYSTICK_CSR) & 0x5 == 0x1;

    // COUNTFLAG is read-to-clear. Observe it three separate times while the
    // exception is masked, proving the timer wrapped repeatedly, not just once.
    let mut wraps = 0u32;
    let mut pending_each = true;
    let mut handler_stayed_masked = true;
    for _ in 0..WANT_WRAPS {
        let mut saw_wrap = false;
        for _ in 0..20_000 {
            if rd(SYSTICK_CSR) & COUNTFLAG != 0 {
                saw_wrap = true;
                break;
            }
        }
        wraps += u32::from(saw_wrap);
        pending_each &= rd(SCB_ICSR) & ICSR_PENDSTSET != 0;
        handler_stayed_masked &=
            unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) == 0 };
    }

    let control_before_disable = rd(SYSTICK_CSR) & 0x7;
    wr(SYSTICK_CSR, 0);
    let pending_after_disable = rd(SCB_ICSR) & ICSR_PENDSTSET != 0;
    let handler_before_unmask =
        unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) };

    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..100_000 {
        if unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) } != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let entries = unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) };
    let active_mask = unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ACTIVE_MASK)) };
    let pending_after_return = rd(SCB_ICSR) & ICSR_PENDSTSET != 0;
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
        asm!("cpsid i", options(nostack, preserves_flags));
        wr(SCB_ICSR, ICSR_PENDSTCLR);
        wr(SYSTICK_CVR, 0);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let one_active_handler = entries == 1 && active_mask & 1 == 1;
    let ok = prior_primask == 0
        && reference_selected
        && wraps == WANT_WRAPS
        && pending_each
        && handler_stayed_masked
        && control_before_disable == 0x3
        && pending_after_disable
        && handler_before_unmask == 0
        && one_active_handler
        && !pending_after_return
        && thread_ipsr == 0;

    puts("systick external pending-coalesce ");
    flag("reference", reference_selected);
    puts("wraps=");
    hex(wraps);
    puts(" ");
    flag("pending_each", pending_each);
    flag(
        "masked",
        handler_stayed_masked && handler_before_unmask == 0,
    );
    flag("pending_after_disable", pending_after_disable);
    puts("entries=");
    hex(entries);
    puts(" ");
    flag("active", one_active_handler);
    flag("cleared", !pending_after_return);
    verdict(ok);
    ok
}

#[exception]
fn SysTick() {
    let entry = unsafe { read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ENTRIES)) };
    if entry < 3 {
        let active = rd(SCB_ICSR) & 0x1FF == 15;
        // The word CSR read returns then clears COUNTFLAG, independently of
        // the exception's pending/active state.
        let countflag = rd(SYSTICK_CSR) & (1 << 16) != 0;
        unsafe {
            let active_mask = read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_ACTIVE_MASK));
            let countflag_mask =
                read_volatile(core::ptr::addr_of!(SYSTICK_EXTERNAL_COUNTFLAG_MASK));
            write_volatile(
                core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ACTIVE_MASK),
                active_mask | (u32::from(active) << entry),
            );
            write_volatile(
                core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_COUNTFLAG_MASK),
                countflag_mask | (u32::from(countflag) << entry),
            );
        }
    }
    unsafe {
        write_volatile(
            core::ptr::addr_of_mut!(SYSTICK_EXTERNAL_ENTRIES),
            entry.saturating_add(1),
        );
    }
}

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate) = rcc_bringup();
    uart_init(USART1);
    uart_init(USART2);
    uart_init(USART3);
    uart_init(USART4);
    puts("STM32F072 Cortex-M0 smoke\r\n");
    send(USART2, b"usart2 ok\r\n");
    send(USART3, b"usart3 ok\r\n");
    send(USART4, b"usart4 ok\r\n");

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
    for (i, &(name, base, irq, width)) in TIMERS.iter().enumerate() {
        mark(4 + i as u32, check_timer(name, base, irq, width));
    }
    mark(13, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all (an
    // undriven CS pad leaves its device on the bus).
    cs_output(GPIOB, 0);
    for (i, &(name, base, irq, port, pin, want)) in SPIS.iter().enumerate() {
        mark(14 + i as u32, check_spi(name, base, irq, port, pin, want));
    }
    mark(16, check_dma_spi1());
    mark(17, check_i2c1());
    mark(18, check_spi1_shared_cs());
    mark(19, check_crc());
    mark(20, check_adc());
    mark(21, check_systick_pause_resume());
    mark(22, check_systick_external_pause_resume());
    let systick_external_periodic = check_systick_external_periodic();
    let systick_external_coalesces = check_systick_external_pending_coalesces();
    mark(23, systick_external_periodic && systick_external_coalesces);
    mark(27, check_systick_calibration());
    let summary = summary | 0x0700_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
