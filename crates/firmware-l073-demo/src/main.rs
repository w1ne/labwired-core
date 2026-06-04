#![no_std]
#![no_main]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// NUCLEO-L073RZ comprehensive peripheral demo (Cortex-M0+, thumbv6m-none-eabi).
//
// Bare-register firmware — no HAL. Uses real STM32L0 register offsets
// (RM0367), so the SAME binary runs on physical silicon and in the simulator.
// It exercises each important peripheral and prints one deterministic token
// per check, so the board's UART can be diffed against the simulator's stdout.
//
// Token meaning:
//   <NAME>=OK    self-contained result that must match silicon byte-for-byte
//   <NAME>=<hex> raw value (informational; analog/random need not match)
//   <NAME>=UP    behavioural (e.g. a counter advanced)

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ---- RCC (0x40021000, RM0367 §7) ---------------------------------------
const RCC_CR: *mut u32 = 0x4002_1000 as *mut u32;
const RCC_CFGR: *mut u32 = 0x4002_100C as *mut u32;
const RCC_AHBENR: *mut u32 = 0x4002_1030 as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_1038 as *mut u32;
const RCC_IOPENR: *mut u32 = 0x4002_102C as *mut u32;
const RCC_CRRCR: *mut u32 = 0x4002_1098 as *mut u32; // HSI48 control (RNG clock)
const RCC_CR_HSI16ON: u32 = 1 << 0;
const RCC_CR_HSI16RDY: u32 = 1 << 2;
const RCC_CRRCR_HSI48ON: u32 = 1 << 0;
const RCC_CRRCR_HSI48RDY: u32 = 1 << 1;
const RCC_CFGR_SW_HSI16: u32 = 0b01;
const RCC_CFGR_SWS_HSI16: u32 = 0b01 << 2;
// AHBENR
const DMAEN: u32 = 1 << 0;
const CRCEN: u32 = 1 << 12;
const RNGEN: u32 = 1 << 20;
// APB2ENR
const TIM21EN: u32 = 1 << 2;
const ADC1EN: u32 = 1 << 9;
const SPI1EN: u32 = 1 << 12;
// APB1ENR
const USART2EN: u32 = 1 << 17;
const I2C1EN: u32 = 1 << 21;
const IOPAEN: u32 = 1 << 0;

// ---- GPIOA (IOPORT bus @ 0x50000000) -----------------------------------
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
// GPIOC (B1 user button on PC13)
const GPIOC_IDR: *const u32 = 0x5000_0810 as *const u32;

// ---- USART2 (0x40004400, modern stm32v2 layout) ------------------------
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_TE: u32 = 1 << 3;

// ---- CRC (0x40023000) --------------------------------------------------
const CRC_DR: *mut u32 = 0x4002_3000 as *mut u32;
const CRC_CR: *mut u32 = 0x4002_3008 as *mut u32;
const CRC_CR_RESET: u32 = 1 << 0;

// ---- DMA1 (0x40020000), channel 1 --------------------------------------
const DMA_ISR: *const u32 = 0x4002_0000 as *const u32;
const DMA_IFCR: *mut u32 = 0x4002_0004 as *mut u32;
const DMA_CCR1: *mut u32 = 0x4002_0008 as *mut u32;
const DMA_CNDTR1: *mut u32 = 0x4002_000C as *mut u32;
const DMA_CPAR1: *mut u32 = 0x4002_0010 as *mut u32;
const DMA_CMAR1: *mut u32 = 0x4002_0014 as *mut u32;
const DMA_TCIF1: u32 = 1 << 1;

// ---- TIM21 (0x40010800) ------------------------------------------------
const TIM21_CR1: *mut u32 = 0x4001_0800 as *mut u32;
const TIM21_CNT: *const u32 = 0x4001_0824 as *const u32;
const TIM21_ARR: *mut u32 = 0x4001_082C as *mut u32;

// ---- I2C1 (0x40005400, v2 layout) --------------------------------------
const I2C1_CR1: *mut u32 = 0x4000_5400 as *mut u32;
const I2C1_CR2: *mut u32 = 0x4000_5404 as *mut u32;
const I2C1_ISR: *const u32 = 0x4000_5418 as *const u32;
const I2C1_ICR: *mut u32 = 0x4000_541C as *mut u32;
const I2C_ISR_NACKF: u32 = 1 << 4;

// ---- SPI1 (0x40013000) -------------------------------------------------
const SPI1_CR1: *mut u32 = 0x4001_3000 as *mut u32;
const SPI1_SR: *const u32 = 0x4001_3008 as *const u32;
const SPI_SR_TXE: u32 = 1 << 1;

// ---- ADC1 (0x40012400) -------------------------------------------------
const ADC_ISR: *mut u32 = 0x4001_2400 as *mut u32;
const ADC_CR: *mut u32 = 0x4001_2408 as *mut u32;
const ADC_CHSELR: *mut u32 = 0x4001_2428 as *mut u32;
const ADC_DR: *const u32 = 0x4001_2440 as *const u32;
const ADC_CCR: *mut u32 = 0x4001_2788 as *mut u32; // common ctrl
const ADC_CR_ADEN: u32 = 1 << 0;
const ADC_CR_ADSTART: u32 = 1 << 2;
const ADC_ISR_ADRDY: u32 = 1 << 0;
const ADC_ISR_EOC: u32 = 1 << 2;
const ADC_CCR_VREFEN: u32 = 1 << 22;

// ---- RNG (0x40025000) --------------------------------------------------
const RNG_CR: *mut u32 = 0x4002_5000 as *mut u32;
const RNG_SR: *const u32 = 0x4002_5004 as *const u32;
const RNG_DR: *const u32 = 0x4002_5008 as *const u32;
const RNG_CR_RNGEN: u32 = 1 << 2;
const RNG_SR_DRDY: u32 = 1 << 0;

// ---- DBGMCU (APB @ 0x40015800 on Cortex-M0+) ---------------------------
const DBGMCU_IDCODE: *const u32 = 0x4001_5800 as *const u32;

const LED_PIN: u32 = 5;
const SPIN_LIMIT: u32 = 20_000;

#[entry]
fn main() -> ! {
    unsafe {
        // SYSCLK -> HSI16 (16 MHz) for a real 9600 baud on silicon.
        write_volatile(RCC_CR, read_volatile(RCC_CR) | RCC_CR_HSI16ON);
        spin_until(|| read_volatile(RCC_CR) & RCC_CR_HSI16RDY != 0);
        write_volatile(
            RCC_CFGR,
            (read_volatile(RCC_CFGR) & !0b11) | RCC_CFGR_SW_HSI16,
        );
        spin_until(|| read_volatile(RCC_CFGR) & (0b11 << 2) == RCC_CFGR_SWS_HSI16);

        // HSI48 (RC48) — the RNG kernel clock on the L0. Without it the RNG
        // never produces DRDY (silicon returns 0). Bounded wait for the sim.
        write_volatile(RCC_CRRCR, read_volatile(RCC_CRRCR) | RCC_CRRCR_HSI48ON);
        spin_until(|| read_volatile(RCC_CRRCR) & RCC_CRRCR_HSI48RDY != 0);

        // Clock-enable every block we touch (real-silicon required; sim no-op).
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | IOPAEN | (1 << 2)); // A + C
        write_volatile(
            RCC_AHBENR,
            read_volatile(RCC_AHBENR) | DMAEN | CRCEN | RNGEN,
        );
        write_volatile(
            RCC_APB2ENR,
            read_volatile(RCC_APB2ENR) | TIM21EN | ADC1EN | SPI1EN,
        );
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | USART2EN | I2C1EN);

        gpio_usart_init();
    }

    print_str("L073-DEMO BOOT\n");
    print_kv("DEV", unsafe { read_volatile(DBGMCU_IDCODE) });
    print_kv("CLK", unsafe { read_volatile(RCC_CFGR) & 0b1100 }); // SWS field

    // --- must-match (byte-for-byte) -------------------------------------
    print_kv("CRC", test_crc());
    print_str(if test_dma() { "DMA=OK\n" } else { "DMA=FAIL\n" });

    // --- behavioural ----------------------------------------------------
    print_str(if test_tim() { "TIM=UP\n" } else { "TIM=FLAT\n" });
    print_str(if test_i2c_nack() {
        "I2C=NACK\n"
    } else {
        "I2C=?\n"
    });
    print_str(if test_spi_txe() {
        "SPI=TXE\n"
    } else {
        "SPI=?\n"
    });

    // --- informational (analog / random; need not match) ----------------
    print_kv("ADC", test_adc_vrefint());
    print_kv("RNG", test_rng());

    // --- GPIO input -----------------------------------------------------
    let btn = (unsafe { read_volatile(GPIOC_IDR) } >> 13) & 1;
    print_kv("BTN", btn);

    // --- GPIO output (LD2) ----------------------------------------------
    for _ in 0..3 {
        led_set(true);
        print_str("LED ON\n");
        delay(40_000);
        led_set(false);
        print_str("LED OFF\n");
        delay(40_000);
    }

    print_str("DONE\n");
    loop {
        print_str("L073 ALIVE DEV=");
        print_hex32(unsafe { read_volatile(DBGMCU_IDCODE) });
        print_str("\n");
        delay(400_000);
    }
}

// ---- peripheral tests --------------------------------------------------

/// CRC-32 over two fixed words. Deterministic; must match silicon.
fn test_crc() -> u32 {
    unsafe {
        write_volatile(CRC_CR, CRC_CR_RESET); // reset DR to 0xFFFFFFFF
        write_volatile(CRC_DR, 0xDEAD_BEEF);
        write_volatile(CRC_DR, 0x1234_5678);
        read_volatile(CRC_DR)
    }
}

/// DMA1 memory-to-memory copy; verify the destination equals the source.
/// For MEM2MEM with DIR=1 the data flows CMAR -> CPAR, so CMAR is the SOURCE
/// and CPAR is the DESTINATION (matches STM32 silicon and the L476 reference).
fn test_dma() -> bool {
    let src: u32 = 0xDEAD_BEEF;
    let mut dst: u32 = 0;
    unsafe {
        write_volatile(DMA_CCR1, 0); // disable
        write_volatile(DMA_CMAR1, (&src as *const u32) as u32); // source
        write_volatile(DMA_CPAR1, (&mut dst as *mut u32) as u32); // destination
        write_volatile(DMA_CNDTR1, 4); // 4 bytes (default 8-bit size)
                                       // MEM2MEM(14) | MINC(7) | PINC(6) | DIR(4) | EN(0)
        write_volatile(
            DMA_CCR1,
            (1 << 14) | (1 << 7) | (1 << 6) | (1 << 4) | (1 << 0),
        );
        spin_until(|| read_volatile(DMA_ISR) & DMA_TCIF1 != 0);
        write_volatile(DMA_IFCR, DMA_TCIF1);
        write_volatile(DMA_CCR1, 0);
    }
    dst == 0xDEAD_BEEF
}

/// TIM21 free-running counter must advance.
fn test_tim() -> bool {
    unsafe {
        write_volatile(TIM21_ARR, 0xFFFF);
        write_volatile(TIM21_CR1, 1); // CEN
        let a = read_volatile(TIM21_CNT);
        delay(2_000);
        let b = read_volatile(TIM21_CNT);
        write_volatile(TIM21_CR1, 0);
        a != b
    }
}

/// I2C1 master addressing an absent device should raise NACKF.
fn test_i2c_nack() -> bool {
    unsafe {
        write_volatile(I2C1_CR1, 1); // PE
                                     // addr 0x52<<1, NBYTES=1, START, write
        write_volatile(I2C1_CR2, (0x52 << 1) | (1 << 16) | (1 << 13));
        let mut nack = false;
        let mut guard = 0u32;
        while guard < SPIN_LIMIT {
            if read_volatile(I2C1_ISR) & I2C_ISR_NACKF != 0 {
                nack = true;
                break;
            }
            guard += 1;
        }
        write_volatile(I2C1_ICR, I2C_ISR_NACKF);
        write_volatile(I2C1_CR1, 0);
        nack
    }
}

/// SPI1 master: TXE must be set after enable (empty TX buffer).
fn test_spi_txe() -> bool {
    unsafe {
        // MSTR(2) | BR=fpclk/8(3:5=010) | SSM(9) | SSI(8) | SPE(6)
        write_volatile(
            SPI1_CR1,
            (1 << 2) | (0b010 << 3) | (1 << 9) | (1 << 8) | (1 << 6),
        );
        let ok = read_volatile(SPI1_SR) & SPI_SR_TXE != 0;
        write_volatile(SPI1_CR1, 0);
        ok
    }
}

/// ADC VREFINT internal channel — raw conversion (analog; informational).
fn test_adc_vrefint() -> u32 {
    unsafe {
        write_volatile(ADC_CCR, read_volatile(ADC_CCR) | ADC_CCR_VREFEN);
        write_volatile(ADC_CR, ADC_CR_ADEN);
        spin_until(|| read_volatile(ADC_ISR) & ADC_ISR_ADRDY != 0);
        write_volatile(ADC_CHSELR, 1 << 17); // VREFINT = channel 17
        write_volatile(ADC_CR, read_volatile(ADC_CR) | ADC_CR_ADSTART);
        spin_until(|| read_volatile(ADC_ISR) & ADC_ISR_EOC != 0);
        read_volatile(ADC_DR)
    }
}

/// RNG draw — true random on silicon, deterministic LFSR in sim (need not match).
fn test_rng() -> u32 {
    unsafe {
        write_volatile(RNG_CR, RNG_CR_RNGEN);
        spin_until(|| read_volatile(RNG_SR) & RNG_SR_DRDY != 0);
        read_volatile(RNG_DR)
    }
}

// ---- low-level helpers -------------------------------------------------

unsafe fn gpio_usart_init() {
    // PA5 output (LED); PA2/PA3 AF4 (USART2 TX/RX).
    let mut moder = read_volatile(GPIOA_MODER);
    moder &= !(0b11 << (LED_PIN * 2));
    moder |= 0b01 << (LED_PIN * 2);
    moder &= !((0b11 << (2 * 2)) | (0b11 << (3 * 2)));
    moder |= (0b10 << (2 * 2)) | (0b10 << (3 * 2));
    write_volatile(GPIOA_MODER, moder);
    let mut afrl = read_volatile(GPIOA_AFRL);
    afrl &= !((0xF << (2 * 4)) | (0xF << (3 * 4)));
    afrl |= (0x4 << (2 * 4)) | (0x4 << (3 * 4));
    write_volatile(GPIOA_AFRL, afrl);
    // USART2 9600 8N1 @ 16 MHz.
    write_volatile(USART2_BRR, 16_000_000 / 9_600);
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

fn print_kv(key: &str, v: u32) {
    print_str(key);
    putc(b'=');
    print_hex32(v);
    putc(b'\n');
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

fn print_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for shift in (0..32).step_by(4).rev() {
        putc(HEX[((v >> shift) & 0xF) as usize]);
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
