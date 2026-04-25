// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! CubeMX-style HAL firmware for NUCLEO-L476RG.
//!
//! Mimics the canonical pattern an STM32CubeIDE-generated project follows
//! for an L4 with USART2 on the Virtual COM Port:
//!
//!   Reset_Handler   ── .data copy, .bss zero, FPU enable, then main
//!   SystemInit()    ── relocate vector table (VTOR)
//!   HAL_Init()      ── SysTick @ 1 ms (priority 15), uwTick++
//!   SystemClock_    ── PWR.VOSCR=1, MSI->4 MHz->PLL->80 MHz, FLASH latency
//!     Config()        4WS, AHB/APB1/APB2 prescalers, source switch
//!   MX_USART2_       ── PA2/PA3 AF7, USART2 9600 8N1
//!     UART_Init()
//!   loop:           ── HAL_GetTick() readback, HAL_Delay(100), counter print
//!
//! The point of this firmware is to stress the simulator's PLL state
//! machine, SysTick interrupt-driven tick counter, and vector-table
//! relocation — patterns the previous firmware suite touched only
//! peripherally.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

// ---------- Vector table --------------------------------------------------
//
// Cortex-M4: 16 system entries + 82 NVIC IRQs for STM32L4. Only the ones
// this firmware actually arms are non-default. Linker script keeps the
// `.isr_vector` section at FLASH origin.
//
// Cast-from-fn-ptr to integer is not const-eval; use a union to embed both
// raw addresses (initial SP) and function pointers in the same array.

#[repr(C)]
pub union Vector {
    handler: unsafe extern "C" fn(),
    reserved: usize,
}

#[link_section = ".isr_vector"]
#[no_mangle]
pub static VECTORS: [Vector; 16 + 82] = {
    let mut v = [Vector { handler: default_handler }; 16 + 82];
    v[0]  = Vector { reserved: 0x2001_8000 }; // Initial SP
    v[1]  = Vector { handler: reset_handler };
    v[15] = Vector { handler: systick_handler };
    v
};

// `Vector` needs Copy so the array initialiser works; both arms are POD.
impl Clone for Vector {
    fn clone(&self) -> Self {
        unsafe { Vector { reserved: self.reserved } }
    }
}
impl Copy for Vector {}

// ---------- Globals -------------------------------------------------------
//
// `uwTick` is incremented by SysTick. Lives in .bss; Reset_Handler zeros
// the section, so we don't get a runtime initializer.

static mut UW_TICK: u32 = 0;
static mut COUNTER: u32 = 0;

// ---------- Register helpers ---------------------------------------------

const RCC_BASE:        u32 = 0x4002_1000;
const RCC_CR:          *mut u32 = RCC_BASE as *mut u32;
const RCC_CFGR:        *mut u32 = (RCC_BASE + 0x08) as *mut u32;
const RCC_PLLCFGR:     *mut u32 = (RCC_BASE + 0x0C) as *mut u32;
const RCC_AHB2ENR:     *mut u32 = (RCC_BASE + 0x4C) as *mut u32;
const RCC_APB1ENR1:    *mut u32 = (RCC_BASE + 0x58) as *mut u32;

const PWR_BASE:        u32 = 0x4000_7000;
const PWR_CR1:         *mut u32 = PWR_BASE as *mut u32;
const PWR_SR2:         *const u32 = (PWR_BASE + 0x14) as *const u32;

const FLASH_BASE:      u32 = 0x4002_2000;
const FLASH_ACR:       *mut u32 = FLASH_BASE as *mut u32;

const GPIOA_MODER:     *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL:      *mut u32 = 0x4800_0020 as *mut u32;

const USART2_BASE:     u32 = 0x4000_4400;
const USART2_CR1:      *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR:      *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR:      *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR:      *mut u32 = (USART2_BASE + 0x28) as *mut u32;

const SCB_VTOR:        *mut u32 = 0xE000_ED08 as *mut u32;
const SCB_CPACR:       *mut u32 = 0xE000_ED88 as *mut u32;
const SCB_SHPR3:       *mut u32 = 0xE000_ED20 as *mut u32;

const SYST_CSR:        *mut u32 = 0xE000_E010 as *mut u32;
const SYST_RVR:        *mut u32 = 0xE000_E014 as *mut u32;
const SYST_CVR:        *mut u32 = 0xE000_E018 as *mut u32;

#[inline(always)]
fn rmw(p: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(p) & !clear;
        write_volatile(p, v | set);
    }
}

#[inline(always)]
fn set(p: *mut u32, mask: u32) {
    unsafe { write_volatile(p, read_volatile(p) | mask); }
}

// ---------- HAL_Init / SysTick -------------------------------------------

fn system_init() {
    // CubeMX generates a SystemInit that sets VTOR and enables the FPU.
    unsafe {
        // Vector-table relocation: VTOR = flash base.
        write_volatile(SCB_VTOR, 0x0800_0000);
        // FPU full access (CP10 + CP11).
        let v = read_volatile(SCB_CPACR);
        write_volatile(SCB_CPACR, v | (0xF << 20));
    }
}

fn hal_init() {
    // Configure SysTick for 1 ms tick assuming 4 MHz HCLK on entry (MSI).
    // RVR = 4_000 - 1.
    unsafe {
        write_volatile(SYST_RVR, 4_000 - 1);
        write_volatile(SYST_CVR, 0);
        // CSR: ENABLE | TICKINT | CLKSOURCE (= core clock).
        write_volatile(SYST_CSR, 0b111);
        // SysTick priority 15 (lowest) — SHPR3 byte 3.
        let v = read_volatile(SCB_SHPR3) & 0x00FF_FFFF;
        write_volatile(SCB_SHPR3, v | (15 << 24));
    }
}

fn hal_get_tick() -> u32 {
    unsafe { read_volatile(&raw const UW_TICK) }
}

fn hal_delay(ms: u32) {
    let start = hal_get_tick();
    // HAL adds an extra 1 ms cushion to avoid race with the next tick.
    while hal_get_tick().wrapping_sub(start) < ms.wrapping_add(1) {
        unsafe { core::arch::asm!("nop") }
    }
}

#[no_mangle]
pub unsafe extern "C" fn systick_handler() {
    unsafe {
        let v = read_volatile(&raw const UW_TICK);
        write_volatile(&raw mut UW_TICK, v.wrapping_add(1));
    }
}

// ---------- SystemClock_Config (MSI -> PLL @ 80 MHz) ----------------------

fn system_clock_config() {
    // Enable PWR clock and set voltage scaling range 1 (boost mode).
    set(RCC_APB1ENR1, 1 << 28);
    rmw(PWR_CR1, 0x3 << 9, 0x1 << 9); // VOS=01 (range 1)
    while unsafe { (read_volatile(PWR_SR2) & (1 << 10)) != 0 } {} // VOSF cleared

    // Set FLASH latency to 4WS (required for 80 MHz).
    rmw(FLASH_ACR, 0x7, 4);
    while unsafe { (read_volatile(FLASH_ACR) & 0x7) != 4 } {}

    // Configure PLL: source = MSI (bits[1:0] = 1), PLLM = 1 (bits[6:4] = 0),
    // PLLN = 40 (bits[14:8] = 40), PLLR = 2 (bits[26:25] = 0), PLLREN bit 24.
    unsafe {
        write_volatile(
            RCC_PLLCFGR,
            (1 << 0) | (40 << 8) | (1 << 24),
        );
    }

    // Enable PLL.
    set(RCC_CR, 1 << 24);
    while unsafe { (read_volatile(RCC_CR) & (1 << 25)) == 0 } {} // PLLRDY

    // Switch SYSCLK to PLL: SW = 0b11 (bits 1:0 of CFGR).
    rmw(RCC_CFGR, 0x3, 0x3);
    while unsafe { (read_volatile(RCC_CFGR) & (0x3 << 2)) != (0x3 << 2) } {}

    // HCLK is now 80 MHz. Re-tune SysTick reload for 1 ms.
    unsafe {
        write_volatile(SYST_RVR, 80_000 - 1);
        write_volatile(SYST_CVR, 0);
    }
}

// ---------- MX_USART2_UART_Init ------------------------------------------

fn mx_usart2_init() {
    set(RCC_AHB2ENR, 1 << 0);
    set(RCC_APB1ENR1, 1 << 17);

    rmw(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw(GPIOA_AFRL, 0xF << 8, 0x7 << 8);

    unsafe {
        // 115200 baud @ 80 MHz: BRR = 80_000_000 / 115200 = 694.
        write_volatile(USART2_BRR, 694);
        // CR1: UE | TE.
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3));
    }
}

// ---------- UART helpers --------------------------------------------------

const TXE: u32 = 1 << 7;
const TC: u32 = 1 << 6;

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

fn uart_put_dec(mut v: u32) {
    if v == 0 {
        uart_putc(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut n = 0;
    while v > 0 {
        buf[n] = (v % 10) as u8 + b'0';
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        uart_putc(buf[n]);
    }
}

// ---------- main ---------------------------------------------------------

#[no_mangle]
pub extern "C" fn main() -> ! {
    system_init();
    hal_init();
    system_clock_config();
    mx_usart2_init();

    // Send a sync preamble so a USB-CDC receiver that drops the first
    // packet still picks up the locked content. Four back-to-back banners
    // give the J-Link OB VCP a stable burst to latch onto after USB
    // enumeration; spreading them out with HAL_Delay actually makes
    // things worse on this debugger because each packet boundary risks
    // truncation.
    for _ in 0..4 {
        uart_puts(b"HAL BOOT\r\n");
    }

    // Print HAL_GetTick() three times spaced by HAL_Delay(2).
    // The simulator runs faster than 1 ms per cycle, so the actual delay
    // value is small; the survival check is on the format and ordering.
    for _ in 0..3 {
        let n = unsafe {
            let c = read_volatile(&raw const COUNTER);
            write_volatile(&raw mut COUNTER, c + 1);
            c + 1
        };
        uart_puts(b"TICK ");
        uart_put_dec(n);
        uart_puts(b" T=");
        uart_put_dec(hal_get_tick());
        uart_puts(b"\r\n");
        hal_delay(2);
    }

    uart_puts(b"DONE\r\n");

    unsafe {
        while (read_volatile(USART2_ISR) & TC) == 0 {}
    }
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}

// ---------- Reset_Handler ------------------------------------------------

extern "C" {
    static mut _sdata: u32;
    static _edata: u32;
    static _etext: u32;
    static mut _sbss: u32;
    static _ebss: u32;
}

#[no_mangle]
pub unsafe extern "C" fn reset_handler() {
    // Copy .data from flash to RAM.
    let mut src: *const u32 = &raw const _etext;
    let mut dst: *mut u32 = &raw mut _sdata;
    let edata: *const u32 = &raw const _edata;
    while (dst as usize) < (edata as usize) {
        write_volatile(dst, read_volatile(src));
        src = src.add(1);
        dst = dst.add(1);
    }
    // Zero .bss.
    let mut p: *mut u32 = &raw mut _sbss;
    let ebss: *const u32 = &raw const _ebss;
    while (p as usize) < (ebss as usize) {
        write_volatile(p, 0);
        p = p.add(1);
    }
    main();
}

#[no_mangle]
pub unsafe extern "C" fn default_handler() {
    loop {}
}
