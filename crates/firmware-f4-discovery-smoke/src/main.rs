// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! STM32F4-Discovery (MB997, STM32F407VGT6) smoke firmware: a running light
//! over the four user LEDs plus a heartbeat line on USART2, in bare registers.
//!
//! Board facts (UM1472; Zephyr `stm32f4_disco`):
//!   PD12..PD15 — user LEDs LD4 (green), LD3 (orange), LD5 (red), LD6 (blue),
//!                all active high (core configs/systems/stm32f4-discovery.yaml)
//!   PA0        — USER button (not driven here)
//!   PA2/PA3    — USART2 TX/RX at 115200 8N1 from the 16 MHz HSI. On this
//!                board the ST-LINK VCP is NOT wired to the MCU UART, so the
//!                console pins are read from the PA2/PA3 header pins.
//!
//! Each beat lights one LED, prints `hb=<n> <led>=on odr=<hex>`, switches it
//! off and prints `=off`. The `odr=` field is READ BACK from GPIOD_ODR after
//! the write, so a passing run reports the pad the firmware actually drove.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (F4): AHB1ENR gates the GPIO ports, APB1ENR gates USART2, APB2ENR gates SPI1.
const RCC: u32 = 0x4002_3800;
const RCC_AHB1ENR: u32 = RCC + 0x30;
const RCC_APB1ENR: u32 = RCC + 0x40;
const RCC_APB2ENR: u32 = RCC + 0x44;

// USART2 (F4 classic block: SR/DR).
const USART2: u32 = 0x4000_4400;
const USART_SR: *const u32 = USART2 as *const u32;
const USART_DR: *mut u32 = (USART2 + 0x04) as *mut u32;
const USART_BRR: *mut u32 = (USART2 + 0x08) as *mut u32;
const USART_CR1: *mut u32 = (USART2 + 0x0C) as *mut u32;
const USART_RXNE: u32 = 1 << 5;
const USART_RE: u32 = 1 << 2;
const USART_TE: u32 = 1 << 3;
const USART_UE: u32 = 1 << 13;
const TXE: u32 = 1 << 7;

const GPIOA: u32 = 0x4002_0000;
const GPIOD: u32 = 0x4002_0C00;

// SPI1 (F4 classic block: CR1/CR2/SR/DR).
const SPI1: u32 = 0x4001_3000;
const SPI1_CR1: *mut u32 = SPI1 as *mut u32;
const SPI1_CR2: *mut u32 = (SPI1 + 0x04) as *mut u32;
const SPI1_SR: *const u32 = (SPI1 + 0x08) as *const u32;
const SPI1_DR: *mut u32 = (SPI1 + 0x0C) as *mut u32;
const SPI1_IRQ: u32 = 35;
const SPI1_IRQ_BIT: u32 = 1 << (SPI1_IRQ % 32);
const NVIC_ISPR: *const u32 = 0xE000_E200 as *const u32;
const NVIC_ICPR: *mut u32 = 0xE000_E280 as *mut u32;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;
const SPI_SR_BSY: u32 = 1 << 7;

// Cortex-M system-control and SysTick registers used by the exception smoke.
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const TEST_VECTOR_TABLE: u32 = 0x2000_1000;
const TAILCHAIN_RECORD: u32 = 0x2000_3040;
const PSP_TAILCHAIN_RECORD: u32 = 0x2000_3100;
const PSP_TAILCHAIN_STACK_TOP: u32 = 0x2000_3200;
const TAILCHAIN_THREAD_R0: u32 = 0x5448_5244;
const TAILCHAIN_HANDLER_R0: u32 = 0x5045_4E44;

/// GPIO register offsets (STM32 GPIO v2 block: MODER/ODR/BSRR).
const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

/// User LEDs in running-light order: (console name, pin).
const LEDS: [(&[u8], u32); 4] = [(b"ld4", 12), (b"ld3", 13), (b"ld5", 14), (b"ld6", 15)];

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let p = addr as *mut u32;
        write_volatile(p, (read_volatile(p) & !clear) | set);
    }
}

/// Route `pin` of the GPIO port at `base` to alternate function `af`.
fn pin_af(base: u32, pin: u32, af: u32) {
    reg_rmw(base + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
    let (reg, shift) = if pin < 8 {
        (AFRL, pin * 4)
    } else {
        (0x24, (pin - 8) * 4) // AFRH
    };
    reg_rmw(base + reg, 0xF << shift, af << shift);
}

fn putc(b: u8) {
    unsafe {
        while read_volatile(USART_SR) & TXE == 0 {}
        write_volatile(USART_DR, b as u32);
    }
}

fn send(s: &[u8]) {
    for b in s {
        putc(*b);
    }
}

fn put_dec(mut v: u32) {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        putc(digits[n]);
    }
}

fn put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

fn led_drive(pin: u32, on: bool) {
    // BSRR: low half sets the pin, high half resets it.
    let bit = if on { 1 << pin } else { 1 << (pin + 16) };
    unsafe { write_volatile((GPIOD + BSRR) as *mut u32, bit) };
}

fn odr() -> u32 {
    unsafe { read_volatile((GPIOD + ODR) as *const u32) }
}

fn delay() {
    for _ in 0..200000u32 {
        let _ = odr();
    }
}

#[entry]
fn main() -> ! {
    // GPIOA (USART2 pins) + GPIOD (LEDs), then USART2.
    reg_rmw(RCC_AHB1ENR, 0, (1 << 0) | (1 << 3));
    reg_rmw(RCC_APB1ENR, 0, 1 << 17);
    reg_rmw(RCC_APB2ENR, 0, 1 << 12); // SPI1EN
    pin_af(GPIOA, 2, 7);
    pin_af(GPIOA, 3, 7);
    pin_af(GPIOA, 5, 5); // SPI1_SCK
    pin_af(GPIOA, 6, 5); // SPI1_MISO
    pin_af(GPIOA, 7, 5); // SPI1_MOSI
    unsafe {
        // HSI = 16 MHz: USARTDIV 8.6875 -> mantissa 8, fraction 11 -> 0x8B.
        write_volatile(USART_BRR, 0x8B);
        write_volatile(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }
    for (_, pin) in LEDS {
        reg_rmw(GPIOD + MODER, 0b11 << (pin * 2), 0b01 << (pin * 2));
        led_drive(pin, false);
    }
    send(b"F4-DISCO SMOKE\r\n");
    smoke_usart2_rx_enable_gate();
    smoke_spi1_irq_and_max31855();
    smoke_systick_pendsv_tailchain();
    smoke_systick_pendsv_psp_tailchain();

    let mut beat: u32 = 0;
    loop {
        let (name, pin) = LEDS[((beat / 2) as usize) % LEDS.len()];
        led_drive(pin, beat & 1 == 0);
        send(b"hb=");
        put_dec(beat);
        send(b" ");
        send(name);
        let lit = odr() & (1 << pin) != 0;
        send(if lit { b"=on odr=" } else { b"=off odr=" });
        put_hex32(odr());
        send(b"\r\n");
        beat = beat.wrapping_add(1);
        delay();
    }
}

/// Start-injected USART2 bytes remain pending until both legacy CR1 receiver
/// enables are set. This checks UE and RE independently before draining DR.
fn smoke_usart2_rx_enable_gate() {
    let re_off = unsafe { read_volatile(USART_SR) } & USART_RXNE == 0;
    unsafe { write_volatile(USART_CR1, USART_TE | USART_RE) };
    let ue_off = unsafe {
        read_volatile(USART_CR1) & USART_UE == 0 && read_volatile(USART_SR) & USART_RXNE == 0
    };
    unsafe { write_volatile(USART_CR1, USART_UE | USART_TE | USART_RE) };

    let mut received = [0u8; 2];
    let mut count = 0;
    for _ in 0..1_000_000 {
        if unsafe { read_volatile(USART_SR) } & USART_RXNE != 0 && count < received.len() {
            received[count] = unsafe { read_volatile(USART_DR) } as u8;
            count += 1;
        }
        if count == received.len() {
            break;
        }
        core::hint::spin_loop();
    }
    let released = unsafe { read_volatile(USART_SR) } & USART_RXNE == 0;
    let passed = re_off && ue_off && count == 2 && received == *b"RS" && released;

    send(b"USART2 RX GATE re_off=");
    put_dec(if re_off { 1 } else { 0 });
    send(b" ue_off=");
    put_dec(if ue_off { 1 } else { 0 });
    send(b" data=");
    put_hex32((u32::from(received[0]) << 8) | u32::from(received[1]));
    send(b" released=");
    put_dec(if released { 1 } else { 0 });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
}

fn smoke_spi1_irq_and_max31855() {
    unsafe {
        // PA4 is the GPIO chip select for the fixture MAX31855.
        reg_rmw(GPIOA + MODER, 0b11 << (4 * 2), 0b01 << (4 * 2));
        write_volatile((GPIOA + BSRR) as *mut u32, 1 << 4); // deselect

        // SPI1 master, software NSS, fPCLK/8, mode 0, 8-bit frames.
        write_volatile(SPI1_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
        write_volatile(SPI1_CR1, read_volatile(SPI1_CR1) | (1 << 6)); // SPE
        write_volatile(NVIC_ICPR.add((SPI1_IRQ / 32) as usize), SPI1_IRQ_BIT);

        // Idle TXE must assert and then release the SPI1 interrupt line.
        write_volatile(SPI1_CR2, 1 << 7); // TXEIE
    }
    let txe_pended =
        unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT != 0 };
    unsafe {
        write_volatile(SPI1_CR2, 0);
        write_volatile(NVIC_ICPR.add((SPI1_IRQ / 32) as usize), SPI1_IRQ_BIT);
    }
    let txe_released =
        unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT == 0 };

    unsafe {
        write_volatile(SPI1_CR2, 1 << 6); // RXNEIE
        write_volatile((GPIOA + BSRR) as *mut u32, 1 << (4 + 16)); // select PA4-low slave
    }
    let mut frame = 0u32;
    let mut rxne_pended = false;
    let mut transfer_ok = true;
    for _ in 0..4 {
        if !wait_spi(SPI1_SR, SPI_SR_TXE, SPI_SR_TXE) {
            transfer_ok = false;
            break;
        }
        unsafe { write_volatile(SPI1_DR, 0) };
        if !wait_spi(SPI1_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
            transfer_ok = false;
            break;
        }
        let rxne = unsafe { read_volatile(SPI1_SR) } & SPI_SR_RXNE != 0;
        let pending =
            unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT != 0 };
        rxne_pended |= rxne && pending;
        frame = (frame << 8) | (unsafe { read_volatile(SPI1_DR) } & 0xFF);
    }
    let idle = wait_spi(SPI1_SR, SPI_SR_BSY, 0);
    unsafe { write_volatile((GPIOA + BSRR) as *mut u32, 1 << 4) }; // deselect
    let rxne_released = unsafe {
        read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT == 0
            && read_volatile(SPI1_SR) & SPI_SR_RXNE == 0
    };
    unsafe { write_volatile(SPI1_CR2, 0) };

    send(b"SPI1 MAX31855=");
    put_hex32(frame);
    send(b" TXEIRQ=");
    put_dec(if txe_pended { 1 } else { 0 });
    send(b" REARM=");
    put_dec(if txe_released { 0 } else { 1 });
    send(b" RXNEIRQ=");
    put_dec(if rxne_pended { 1 } else { 0 });
    send(b" RELEASED=");
    put_dec(if rxne_released { 1 } else { 0 });
    send(b"\r\n");
    let checks_passed = transfer_ok
        && idle
        && frame == 0x0190_1600
        && txe_pended
        && txe_released
        && rxne_pended
        && rxne_released;
    send(b"SPI1 CHECKS=");
    put_dec(if checks_passed { 1 } else { 0 });
    send(b"\r\n");
}

fn wait_spi(addr: *const u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if unsafe { read_volatile(addr) } & mask == want {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Verify that the Cortex-M4 tail-chains from a pending SysTick into the
/// lower-priority PendSV handler while retaining the original exception frame
/// and the live R0 value from SysTick.
fn smoke_systick_pendsv_tailchain() -> bool {
    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_vtor = unsafe { read_volatile(SCB_VTOR as *const u32) };
    let prior_shpr3 = unsafe { read_volatile(SCB_SHPR3 as *const u32) };
    let prior_rvr = unsafe { read_volatile(SYSTICK_RVR as *const u32) };
    let prior_csr = unsafe { read_volatile(SYSTICK_CSR as *const u32) } & 0x7;
    unsafe {
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        // SysTick has the highest configurable priority; PendSV is lowest.
        write_volatile(
            SCB_SHPR3 as *mut u32,
            (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000,
        );
        for slot in 0..16 {
            write_volatile((TEST_VECTOR_TABLE + slot * 4) as *mut u32, 0);
        }
        write_volatile(
            (TEST_VECTOR_TABLE + 14 * 4) as *mut u32,
            (f4_tailchain_pendsv_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            (TEST_VECTOR_TABLE + 15 * 4) as *mut u32,
            (f4_tailchain_systick_handler as *const () as usize as u32) | 1,
        );
        for word in 0..12 {
            write_volatile((TAILCHAIN_RECORD + word * 4) as *mut u32, 0);
        }
        write_volatile(SCB_VTOR as *mut u32, TEST_VECTOR_TABLE);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { f4_pend_systick_with_thread_r0() };
    }
    for _ in 0..4_096 {
        if unsafe { read_volatile((TAILCHAIN_RECORD + 44) as *const u32) } != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let parent_ipsr = unsafe { read_volatile(TAILCHAIN_RECORD as *const u32) };
    let parent_msp = unsafe { read_volatile((TAILCHAIN_RECORD + 4) as *const u32) };
    let parent_exc_return = unsafe { read_volatile((TAILCHAIN_RECORD + 8) as *const u32) };
    let parent_r0 = unsafe { read_volatile((TAILCHAIN_RECORD + 12) as *const u32) };
    let parent_icsr_before = unsafe { read_volatile((TAILCHAIN_RECORD + 16) as *const u32) };
    let parent_icsr_after = unsafe { read_volatile((TAILCHAIN_RECORD + 20) as *const u32) };
    let child_ipsr = unsafe { read_volatile((TAILCHAIN_RECORD + 24) as *const u32) };
    let child_msp = unsafe { read_volatile((TAILCHAIN_RECORD + 28) as *const u32) };
    let child_exc_return = unsafe { read_volatile((TAILCHAIN_RECORD + 32) as *const u32) };
    let child_r0 = unsafe { read_volatile((TAILCHAIN_RECORD + 36) as *const u32) };
    let child_icsr = unsafe { read_volatile((TAILCHAIN_RECORD + 40) as *const u32) };
    let child_hit = unsafe { read_volatile((TAILCHAIN_RECORD + 44) as *const u32) } == 1;
    let thread_ipsr: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
    }

    let masked_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        write_volatile(SCB_VTOR as *mut u32, prior_vtor);
        write_volatile(SCB_SHPR3 as *mut u32, prior_shpr3);
        write_volatile(SYSTICK_RVR as *mut u32, prior_rvr);
        write_volatile(SYSTICK_CSR as *mut u32, prior_csr);
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_before & ICSR_PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & ICSR_PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFF9
        && child_exc_return == parent_exc_return;
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let r0_live = parent_r0 == TAILCHAIN_THREAD_R0
        && child_r0 == TAILCHAIN_HANDLER_R0
        && child_r0 != parent_r0;
    let cleanup = unsafe {
        read_volatile(SCB_VTOR as *const u32) == prior_vtor
            && read_volatile(SCB_SHPR3 as *const u32) == prior_shpr3
            && read_volatile(SCB_ICSR as *const u32) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0
    };
    let passed = prior_primask == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && thread_ipsr == 0
        && cleanup;

    send(b"SysTick PendSV tailchain frame=");
    put_dec(if frame_reused { 1 } else { 0 });
    send(b" r0=");
    put_dec(if r0_live { 1 } else { 0 });
    send(b" pending=");
    put_dec(if deferred && child_is_pendsv && cleanup {
        1
    } else {
        0
    });
    send(b" ipsr=");
    put_dec(child_ipsr);
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Run the SysTick-to-PendSV tail chain with Thread mode on PSP. Check the
/// exception frame, Handler-mode MSP, live R0 value, and restored context.
fn smoke_systick_pendsv_psp_tailchain() -> bool {
    let prior_primask: u32;
    let prior_msp: u32;
    let prior_psp: u32;
    let prior_control: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            "mrs {msp}, MSP",
            "mrs {psp}, PSP",
            "mrs {control}, CONTROL",
            primask = out(reg) prior_primask,
            msp = out(reg) prior_msp,
            psp = out(reg) prior_psp,
            control = out(reg) prior_control,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_vtor = unsafe { read_volatile(SCB_VTOR as *const u32) };
    let prior_shpr3 = unsafe { read_volatile(SCB_SHPR3 as *const u32) };
    let prior_rvr = unsafe { read_volatile(SYSTICK_RVR as *const u32) };
    let prior_csr = unsafe { read_volatile(SYSTICK_CSR as *const u32) } & 0x7;
    unsafe {
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        // SysTick has the higher configurable priority; PendSV is lowest.
        write_volatile(
            SCB_SHPR3 as *mut u32,
            (prior_shpr3 & 0x0000_FFFF) | 0x80C0_0000,
        );
        for slot in 0..16 {
            write_volatile((TEST_VECTOR_TABLE + slot * 4) as *mut u32, 0);
        }
        write_volatile(
            (TEST_VECTOR_TABLE + 14 * 4) as *mut u32,
            (f4_psp_tailchain_pendsv_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            (TEST_VECTOR_TABLE + 15 * 4) as *mut u32,
            (f4_psp_tailchain_systick_handler as *const () as usize as u32) | 1,
        );
        for word in 0..17 {
            write_volatile((PSP_TAILCHAIN_RECORD + word * 4) as *mut u32, 0);
        }
        write_volatile(SCB_VTOR as *mut u32, TEST_VECTOR_TABLE);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { f4_pend_systick_from_psp() };
    }

    let parent_ipsr = unsafe { read_volatile(PSP_TAILCHAIN_RECORD as *const u32) };
    let parent_icsr_before = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 4) as *const u32) };
    let parent_icsr_after = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 8) as *const u32) };
    let parent_msp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 12) as *const u32) };
    let parent_exc_return = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 16) as *const u32) };
    let thread_r0 = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 20) as *const u32) };
    let child_ipsr = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 24) as *const u32) };
    let child_msp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 28) as *const u32) };
    let child_exc_return = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 32) as *const u32) };
    let child_r0 = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 36) as *const u32) };
    let child_icsr = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 40) as *const u32) };
    let child_hit = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 44) as *const u32) } == 1;
    let parent_psp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 48) as *const u32) };
    let child_psp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 52) as *const u32) };
    let wrapper_msp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 56) as *const u32) };
    let wrapper_psp = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 60) as *const u32) };
    let wrapper_control = unsafe { read_volatile((PSP_TAILCHAIN_RECORD + 64) as *const u32) };

    let thread_ipsr: u32;
    let msp_after: u32;
    let psp_after: u32;
    let control_after: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            "mrs {msp}, MSP",
            "mrs {psp}, PSP",
            "mrs {control}, CONTROL",
            ipsr = out(reg) thread_ipsr,
            msp = out(reg) msp_after,
            psp = out(reg) psp_after,
            control = out(reg) control_after,
            options(nomem, nostack, preserves_flags),
        );
    }

    let masked_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
        write_volatile(SYSTICK_CSR as *mut u32, 0);
        write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
        write_volatile(SCB_VTOR as *mut u32, prior_vtor);
        write_volatile(SCB_SHPR3 as *mut u32, prior_shpr3);
        write_volatile(SYSTICK_RVR as *mut u32, prior_rvr);
        write_volatile(SYSTICK_CSR as *mut u32, prior_csr);
        core::arch::asm!(
            "msr PSP, {psp}",
            "msr CONTROL, {control}",
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            psp = in(reg) prior_psp,
            control = in(reg) prior_control,
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_before & ICSR_PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & ICSR_PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFFD
        && child_exc_return == parent_exc_return
        && parent_psp == child_psp
        && parent_psp == PSP_TAILCHAIN_STACK_TOP - 32
        && wrapper_msp == prior_msp
        && parent_msp == prior_msp - 16;
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let r0_live = thread_r0 == TAILCHAIN_THREAD_R0
        && child_r0 == TAILCHAIN_HANDLER_R0
        && child_r0 != thread_r0;
    let cleanup = unsafe { read_volatile(SCB_VTOR as *const u32) == prior_vtor }
        && unsafe { read_volatile(SCB_SHPR3 as *const u32) == prior_shpr3 }
        && unsafe {
            read_volatile(SCB_ICSR as *const u32) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0
        }
        && msp_after == prior_msp
        && psp_after == prior_psp
        && control_after == prior_control
        && wrapper_psp == prior_psp
        && wrapper_control == prior_control
        && thread_ipsr == 0;
    let passed = prior_primask == 0
        && prior_control & 2 == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && cleanup;

    send(b"SysTick PendSV PSP tailchain frame=");
    put_dec(if frame_reused { 1 } else { 0 });
    send(b" r0=");
    put_dec(if r0_live { 1 } else { 0 });
    send(b" pending=");
    put_dec(if deferred && child_is_pendsv && cleanup {
        1
    } else {
        0
    });
    send(b" ipsr=");
    put_dec(child_ipsr);
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Switch Thread mode to a dedicated PSP stack, then pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn f4_pend_systick_from_psp() {
    core::arch::naked_asm!(
        "mrs r3, msp",
        "push {{r4, r5, r6, lr}}",
        "mrs r4, control",
        "mrs r5, psp",
        "ldr r0, =0x20003100",
        "str r3, [r0, #56]",
        "str r5, [r0, #60]",
        "str r4, [r0, #64]",
        "ldr r1, =0x20003200",
        "msr psp, r1",
        "movs r1, #2",
        "orrs r1, r4",
        "msr control, r1",
        "isb",
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "ldr r1, =0x20003100",
        "movw r6, #4096",
        "1:",
        "ldr r2, [r1, #44]",
        "cmp r2, #0",
        "bne 2f",
        "subs r6, r6, #1",
        "bne 1b",
        "2:",
        "msr psp, r5",
        "msr control, r4",
        "isb",
        "pop {{r4, r5, r6, pc}}",
    );
}

/// Record the PSP frame, pend PendSV, and leave a distinct live R0 marker.
#[unsafe(naked)]
unsafe extern "C" fn f4_psp_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20003100",
        "mrs r3, ipsr",
        "str r3, [r0, #0]",
        "ldr r3, =0xE000ED04",
        "ldr r1, [r3]",
        "str r1, [r0, #4]",
        "movs r1, #1",
        "lsls r1, r1, #28",
        "str r1, [r3]",
        "ldr r1, [r3]",
        "str r1, [r0, #8]",
        "mrs r1, msp",
        "str r1, [r0, #12]",
        "mov r1, lr",
        "str r1, [r0, #16]",
        "mrs r1, psp",
        "str r1, [r0, #48]",
        "str r2, [r0, #20]",
        "ldr r0, =0x50454E44",
        "bx lr",
    );
}

/// Capture the tail-chained PendSV state before Thread mode resumes.
#[unsafe(naked)]
unsafe extern "C" fn f4_psp_tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20003100",
        "mrs r3, ipsr",
        "str r3, [r0, #24]",
        "mrs r3, msp",
        "str r3, [r0, #28]",
        "mov r3, lr",
        "str r3, [r0, #32]",
        "str r2, [r0, #36]",
        "ldr r3, =0xE000ED04",
        "ldr r3, [r3]",
        "str r3, [r0, #40]",
        "mrs r3, psp",
        "str r3, [r0, #52]",
        "movs r3, #1",
        "str r3, [r0, #44]",
        "bx lr",
    );
}

/// Set a Thread-mode R0 marker and software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn f4_pend_systick_with_thread_r0() {
    core::arch::naked_asm!(
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "bx lr",
    );
}

/// Record the active SysTick frame, pend PendSV, and return with a new R0 marker.
#[unsafe(naked)]
unsafe extern "C" fn f4_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20003040",
        "str r2, [r0, #12]",
        "mrs r2, ipsr",
        "str r2, [r0, #0]",
        "mrs r2, msp",
        "str r2, [r0, #4]",
        "mov r2, lr",
        "str r2, [r0, #8]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #16]",
        "movs r2, #1",
        "lsls r2, r2, #28",
        "str r2, [r1]",
        "ldr r2, [r1]",
        "str r2, [r0, #20]",
        "ldr r0, =0x50454E44",
        "bx lr",
    );
}

/// Capture the tail-chained PendSV state before the original Thread frame is
/// unstacked.
#[unsafe(naked)]
unsafe extern "C" fn f4_tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20003040",
        "mrs r3, ipsr",
        "str r3, [r0, #24]",
        "mrs r3, msp",
        "str r3, [r0, #28]",
        "mov r3, lr",
        "str r3, [r0, #32]",
        "str r2, [r0, #36]",
        "ldr r3, =0xE000ED04",
        "ldr r3, [r3]",
        "str r3, [r0, #40]",
        "movs r3, #1",
        "str r3, [r0, #44]",
        "bx lr",
    );
}
