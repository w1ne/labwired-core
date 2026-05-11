// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! F407 survival-trace smoke firmware.
//!
//! Exercises the minimal CPU + RCC + GPIO + USART2 surface so any drift
//! in the F4 chip model breaks the survival test. Mirrors the L476
//! smoke firmware byte-for-byte in structure so the diff workflow is
//! consistent across boards.
//!
//! Pin map (per UM1472 / Nucleo-F407 user manual):
//!   PA2  — USART2_TX (AF7) → ST-LINK Virtual COM Port
//!   PA3  — USART2_RX (AF7)
//!   PA5  — LD2 user LED
//!
//! Default boot clock is HSI = 16 MHz; USART2 BRR = 139 for 115200 8N1.
//!
//! Output (captured for survival test):
//!   F407 SMOKE
//!   DEV=<DBGMCU IDCODE>
//!   MUL=<u32 product>
//!   DONE

#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};
use panic_halt as _;

// ── RCC (F4 family — base differs from L4) ────────────────────────────
const RCC_BASE: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC_BASE + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x40) as *mut u32;

// ── GPIOA (stm32v2 layout) ────────────────────────────────────────────
const GPIOA_BASE: u32 = 0x4002_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;

// ── USART2 (F4 classic layout — SR/DR not ISR/TDR) ────────────────────
const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: *const u32 = USART2_BASE as *const u32;
const USART2_DR: *mut u32 = (USART2_BASE + 0x04) as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x08) as *mut u32;
const USART2_CR1: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;

const SR_TXE: u32 = 1 << 7;
const SR_TC: u32 = 1 << 6;
const CR1_UE: u32 = 1 << 13;
const CR1_TE: u32 = 1 << 3;

// ── DBGMCU (Cortex-M private peripheral bus) ──────────────────────────
const DBGMCU_IDCODE: *const u32 = 0xE004_2000 as *const u32;

// ── Helpers ───────────────────────────────────────────────────────────

#[inline(always)]
fn rmw32(ptr: *mut u32, set: u32) {
    unsafe { write_volatile(ptr, read_volatile(ptr) | set) }
}

#[inline(always)]
fn rmw32_mask(ptr: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = read_volatile(ptr) & !clear;
        write_volatile(ptr, v | set);
    }
}

/// ARM semihosting WRITEC: write a single byte to the debugger's
/// host-side console. On the simulator this is a no-op (the byte still
/// flows out via USART2 in `uart_putc`); on real silicon openocd with
/// `arm semihosting enable` captures the byte. Dual-emit lets the same
/// firmware ELF produce identical byte streams on both sides.
#[inline(always)]
fn semihost_writec(byte: u8) {
    let p = core::ptr::addr_of!(byte);
    unsafe {
        core::arch::asm!(
            "bkpt #0xAB",
            inout("r0") 0x03_u32 => _,  // SYS_WRITEC
            in("r1") p,
            options(preserves_flags, nostack),
        );
    }
}

fn uart_putc(c: u8) {
    unsafe {
        while (read_volatile(USART2_SR) & SR_TXE) == 0 {}
        write_volatile(USART2_DR, c as u32);
    }
    semihost_writec(c);
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

// ── Bring-up ──────────────────────────────────────────────────────────

fn usart2_init() {
    // AHB1: enable GPIOA clock (bit 0).
    rmw32(RCC_AHB1ENR, 1 << 0);
    // APB1: enable USART2 clock (bit 17).
    rmw32(RCC_APB1ENR, 1 << 17);

    // PA2 → alternate function (MODER[5:4] = 0b10), AF7 USART2_TX
    // (AFRL[11:8] = 0x7). PA3 wired the same way for RX so a host can
    // talk back if needed — not exercised by this smoke trace.
    rmw32_mask(GPIOA_MODER, 0x3 << 4, 0x2 << 4);
    rmw32_mask(GPIOA_MODER, 0x3 << 6, 0x2 << 6);
    rmw32_mask(GPIOA_AFRL, 0xF << 8, 0x7 << 8);
    rmw32_mask(GPIOA_AFRL, 0xF << 12, 0x7 << 12);

    unsafe {
        // BRR for 115200 baud at HSI = 16 MHz: USARTDIV = 16e6 / (16 * 115200)
        // ≈ 8.681, encoded as DIV_Mantissa=8 (bits 15:4) + DIV_Fraction=11
        // (bits 3:0) → (8 << 4) | 11 = 0x8B = 139.
        write_volatile(USART2_BRR, 139);
        write_volatile(USART2_CR1, CR1_UE | CR1_TE);
    }

    // Small spin so the line settles before the first byte goes out.
    for _ in 0..2_000 {
        unsafe { core::arch::asm!("nop") }
    }
}

// ── Entry ─────────────────────────────────────────────────────────────

/// Catch-all handler for every exception other than Reset. The minimal
/// vector table in `minimal.ld` points NMI/HardFault/MemManage/BusFault/
/// UsageFault/SVCall/DebugMon/PendSV/SysTick all here, so any unexpected
/// fault sits in WFI rather than escalating into a double fault by
/// reading garbage as the handler PC. Useful in particular for catching
/// a BKPT-escalation if openocd's semihosting trap isn't installed yet.
#[no_mangle]
pub extern "C" fn default_handler() -> ! {
    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    usart2_init();

    uart_puts(b"F407 SMOKE\r\n");

    uart_puts(b"DEV=");
    uart_put_hex32(unsafe { read_volatile(DBGMCU_IDCODE) });
    uart_puts(b"\r\n");

    // ARM Thumb-2 32-bit multiply — surfaces decoder coverage on F4.
    // 0x12345678 * 3 = 0x36CF03268, low 32 bits = 0x6CF03268.
    let a: u32 = 0x1234_5678;
    let b: u32 = 3;
    let product = a.wrapping_mul(b);
    uart_puts(b"MUL=");
    uart_put_hex32(product);
    uart_puts(b"\r\n");

    uart_puts(b"DONE\r\n");

    // Drain the last byte from the shift register so a debugger reset
    // can't truncate the trailing newline.
    unsafe { while (read_volatile(USART2_SR) & SR_TC) == 0 {} }

    loop {
        unsafe { core::arch::asm!("wfi") }
    }
}
