// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Reference firmware for NUCLEO-L476RG.
//!
//! Exercises every peripheral that has been hardware-validated against
//! real silicon: USART2 (Virtual COM Port), GPIOA (LD2 user LED),
//! GPIOC (B1 user button), SPI1, I2C1, ADC1, DMA1. The output stream
//! is locked as a survival test in `crates/core/tests/firmware_survival.rs`
//! so any drift in the simulator's L4 peripheral models breaks CI.
//!
//! Pin map (per UM1724 STM32 Nucleo-64 user manual):
//!   PA2  - USART2_TX (AF7) -> ST-LINK / J-Link OB Virtual COM Port
//!   PA3  - USART2_RX (AF7)
//!   PA5  - LD2 user LED (active high)
//!   PC13 - B1 user button (active low — pressed = 0)
//!
//! All MMIO accesses go through `core::ptr::{read,write}_volatile`. Plain
//! `*ptr = X` and `*ptr` are NOT volatile in Rust and the optimiser is
//! free to reorder, coalesce or drop them — that bit me once already
//! when CCR writes landed before CNDTR.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

// ---------- Register addresses (RM0351) -----------------------------------

const RCC_BASE:        u32 = 0x4002_1000;
const RCC_AHB1ENR:     *mut u32 = (RCC_BASE + 0x48) as *mut u32;
const RCC_AHB2ENR:     *mut u32 = (RCC_BASE + 0x4C) as *mut u32;
const RCC_APB1ENR1:    *mut u32 = (RCC_BASE + 0x58) as *mut u32;
const RCC_APB2ENR:     *mut u32 = (RCC_BASE + 0x60) as *mut u32;

const GPIOA_BASE:      u32 = 0x4800_0000;
const GPIOA_MODER:     *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_ODR:       *mut u32 = (GPIOA_BASE + 0x14) as *mut u32;
const GPIOA_AFRL:      *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

const GPIOC_BASE:      u32 = 0x4800_0800;
const GPIOC_IDR:       *const u32 = (GPIOC_BASE + 0x10) as *const u32;

const USART2_BASE:     u32 = 0x4000_4400;
const USART2_CR1:      *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR:      *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR:      *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR:      *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const SPI1_BASE:       u32 = 0x4001_3000;
const SPI1_CR1:        *mut u16 = SPI1_BASE as *mut u16;
const SPI1_CR2:        *mut u16 = (SPI1_BASE + 0x04) as *mut u16;
const SPI1_SR:         *const u16 = (SPI1_BASE + 0x08) as *const u16;

const I2C1_BASE:       u32 = 0x4000_5400;
const I2C1_CR1:        *mut u32 = I2C1_BASE as *mut u32;
const I2C1_TIMINGR:    *mut u32 = (I2C1_BASE + 0x10) as *mut u32;
const I2C1_ISR:        *const u32 = (I2C1_BASE + 0x18) as *const u32;

const ADC1_BASE:       u32 = 0x5004_0000;
const ADC1_CR:         *mut u32 = (ADC1_BASE + 0x08) as *mut u32;

const DMA1_BASE:       u32 = 0x4002_0000;
const DMA1_ISR:        *const u32 = DMA1_BASE as *const u32;
const DMA1_CCR1:       *mut u32 = (DMA1_BASE + 0x08) as *mut u32;
const DMA1_CNDTR1:     *mut u32 = (DMA1_BASE + 0x0C) as *mut u32;
const DMA1_CPAR1:      *mut u32 = (DMA1_BASE + 0x10) as *mut u32;
const DMA1_CMAR1:      *mut u32 = (DMA1_BASE + 0x14) as *mut u32;

const DBGMCU_IDCODE:   *const u32 = 0xE004_2000 as *const u32;

// USART2_ISR flags
const TXE: u32 = 1 << 7;
const TC:  u32 = 1 << 6;

// ---------- Tiny helpers --------------------------------------------------

#[inline(always)]
fn delay(loops: u32) {
    for _ in 0..loops {
        unsafe { core::arch::asm!("nop") }
    }
}

#[inline(always)]
fn rmw32(ptr: *mut u32, set: u32) {
    unsafe { write_volatile(ptr, read_volatile(ptr) | set); }
}

#[inline(always)]
fn rmw32_mask(ptr: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(ptr) & !clear;
        write_volatile(ptr, v | set);
    }
}

fn uart_putc(c: u8) {
    unsafe {
        while (read_volatile(USART2_ISR) & TXE) == 0 {}
        write_volatile(USART2_TDR, c as u32);
    }
}

fn uart_puts(s: &[u8]) {
    for &b in s {
        uart_putc(b);
    }
}

fn uart_put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        uart_putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

// ---------- Peripheral bring-up helpers -----------------------------------

fn usart2_init() {
    rmw32(RCC_AHB2ENR, 1 << 0);   // GPIOAEN
    rmw32(RCC_APB1ENR1, 1 << 17); // USART2EN

    // PA2 -> AF mode (bits[5:4] = 0b10), AF7 USART2_TX (AFRL bits[11:8] = 0x7).
    rmw32_mask(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw32_mask(GPIOA_AFRL,  0xF << 8, 0x7 << 8);

    unsafe {
        write_volatile(USART2_BRR, 35); // 115200 @ 4 MHz MSI
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE + TE
    }
    delay(5_000);
}

fn led_init() {
    rmw32(RCC_AHB2ENR, (1 << 0) | (1 << 2)); // GPIOA + GPIOC
    rmw32_mask(GPIOA_MODER, 0x3 << 10, 0x1 << 10); // PA5 -> output
}

fn led_on()  { rmw32(GPIOA_ODR, 1 << 5); }
fn led_off() { rmw32_mask(GPIOA_ODR, 1 << 5, 0); }

fn button_pressed() -> bool {
    unsafe { (read_volatile(GPIOC_IDR) & (1 << 13)) == 0 }
}

fn spi1_init() {
    rmw32(RCC_APB2ENR, 1 << 12); // SPI1EN
    delay(100);
    unsafe {
        // CR2: DS = 8-bit (0x7 << 8), FRXTH = 1 (1 << 12).
        write_volatile(SPI1_CR2, (0x7 << 8) | (1 << 12));
        // CR1: MSTR | SSM | SSI | BR=/256 | SPE.
        write_volatile(
            SPI1_CR1,
            (1 << 2) | (1 << 9) | (1 << 8) | (0x7 << 3) | (1 << 6),
        );
    }
}

fn spi1_ready() -> bool {
    unsafe { (read_volatile(SPI1_SR) & 0x2) != 0 }
}

fn i2c1_init() {
    rmw32(RCC_APB1ENR1, 1 << 21); // I2C1EN
    delay(100);
    unsafe {
        write_volatile(I2C1_TIMINGR, 0x1080_5E89); // 100 kHz @ 4 MHz
        write_volatile(I2C1_CR1, 1); // PE
    }
}

fn i2c1_ready() -> bool {
    unsafe { (read_volatile(I2C1_ISR) & 1) != 0 }
}

fn adc1_init() {
    rmw32(RCC_AHB2ENR, 1 << 13); // ADCEN
    delay(100);
    unsafe {
        write_volatile(ADC1_CR, 0); // exit DEEPPWD
        delay(100);
        write_volatile(ADC1_CR, 1 << 28); // ADVREGEN
        delay(1_000);
    }
}

fn adc1_ready() -> bool {
    unsafe { (read_volatile(ADC1_CR) & (1 << 28)) != 0 }
}

/// Run a 4-byte memory-to-memory DMA copy and verify the destination
/// matches the source.
///
/// Uses fixed SRAM addresses rather than stack-allocated buffers
/// because the minimal linker script has no .data/.bss layout — static
/// muts would land in flash and writes would drop. Hard-coded SRAM
/// addresses are safe because the stack (SP = 0x20018000) grows down
/// from the top of SRAM1; 0x20000010 / 0x20000020 are well below
/// anything the stack will reach for this firmware.
const DMA_SRC_ADDR: u32 = 0x2000_0010;
const DMA_DST_ADDR: u32 = 0x2000_0020;
const DMA_SRC_PTR: *mut u32 = DMA_SRC_ADDR as *mut u32;
const DMA_DST_PTR: *mut u32 = DMA_DST_ADDR as *mut u32;

fn dma1_self_test() -> bool {
    rmw32(RCC_AHB1ENR, 1); // DMA1EN
    delay(100);

    unsafe {
        // Seed source, pre-zero destination.
        write_volatile(DMA_SRC_PTR, 0xDEAD_BEEF);
        write_volatile(DMA_DST_PTR, 0);

        // STM32 mem-to-mem direction quirk (RM0351 §11.4.7): with
        // MEM2MEM=1, DIR=1 must be set and data flows CMAR -> CPAR.
        // So CMAR is the SOURCE and CPAR is the DESTINATION here.
        write_volatile(DMA1_CMAR1,  DMA_SRC_ADDR);
        write_volatile(DMA1_CPAR1,  DMA_DST_ADDR);
        write_volatile(DMA1_CNDTR1, 4);
        // MEM2MEM | MINC | PINC | DIR | EN.
        write_volatile(
            DMA1_CCR1,
            (1 << 14) | (1 << 7) | (1 << 6) | (1 << 4) | (1 << 0),
        );

        // Spin until TCIF1 (bit 1) lights or we exhaust patience.
        for _ in 0..100_000 {
            if (read_volatile(DMA1_ISR) & (1 << 1)) != 0 {
                break;
            }
        }

        read_volatile(DMA_DST_PTR) == 0xDEAD_BEEF
    }
}

// ---------- main ----------------------------------------------------------

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    led_init();
    usart2_init();

    uart_puts(b"L476-DEMO BOOT\r\n");

    uart_puts(b"DEV=");
    uart_put_hex32(unsafe { read_volatile(DBGMCU_IDCODE) });
    uart_puts(b"\r\n");

    spi1_init();
    uart_puts(if spi1_ready() { b"SPI1 OK\r\n" } else { b"SPI1 FAIL\r\n" });

    i2c1_init();
    uart_puts(if i2c1_ready() { b"I2C1 OK\r\n" } else { b"I2C1 FAIL\r\n" });

    adc1_init();
    uart_puts(if adc1_ready() { b"ADC1 OK\r\n" } else { b"ADC1 FAIL\r\n" });

    uart_puts(if dma1_self_test() { b"DMA1 OK\r\n" } else { b"DMA1 FAIL\r\n" });

    led_on();
    delay(50_000);
    uart_puts(b"LED ON\r\n");

    led_off();
    delay(50_000);
    uart_puts(b"LED OFF\r\n");

    // B1 button state (active-low). Printed as a single hex digit so
    // both 0 and 1 are deterministic in the survival test. The default
    // GPIO IDR in the simulator reads 0 (= "pressed" in active-low
    // semantics) — that's a separate fidelity gap from real silicon
    // which pulls PC13 up to VDD via R34. The demo just reports
    // whatever's in IDR rather than gating output on it.
    uart_puts(b"BTN=");
    let pressed = if button_pressed() { 1 } else { 0 };
    uart_putc(b'0' + pressed);
    uart_puts(b"\r\n");

    uart_puts(b"DONE\r\n");

    // Drain so a debugger reset can't truncate the trailing newline.
    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }

    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
