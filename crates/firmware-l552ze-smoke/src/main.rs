// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! NUCLEO-L552ZE-Q (STM32L552ZET6Q, Cortex-M33) smoke firmware: a running
//! light over the three user LEDs plus a heartbeat line on LPUART1, in bare
//! registers.
//!
//! Board facts (UM2581; Zephyr `nucleo_l552ze_q`):
//!   LD1 (green) PC7, LD2 (blue) PB7, LD3 (red) PA9 — active high
//!   B1  (user)  PC13 — active high
//!   VCP console LPUART1 PG7 (TX) / PG8 (RX), AF8, 115200 8N1
//!
//! The firmware enables the L5 clock gates (RCC_AHB2ENR for GPIOA/B/C/G,
//! RCC_APB1ENR2.LPUART1EN), enables VDDIO2 (PG[15:2] sit on the VDDIO2 rail
//! and stay isolated until PWR_CR2.IOSV is set), configures PG7/PG8 as AF8,
//! walks the LEDs and prints the actual GPIO ODR readback, then reads the
//! button and heartbeats.
//!
//! Only the reset MSI (4 MHz) is used: LPUART BRR = 256 * 4 MHz / 115200 =
//! 8889 = 0x22B9 (the LPUART uses a 256x oversampling divider).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (RM0438 RCC chapter): AHB2ENR @ 0x4C (GPIO), APB1ENR1 @ 0x58 (PWR),
// APB1ENR2 @ 0x5C (LPUART1).
const RCC: u32 = 0x4002_1000;
const RCC_AHB2ENR: *mut u32 = (RCC + 0x4C) as *mut u32;
const RCC_APB1ENR1: *mut u32 = (RCC + 0x58) as *mut u32;
const RCC_APB1ENR2: *mut u32 = (RCC + 0x5C) as *mut u32;
const RCC_APB1ENR1_PWREN: u32 = 1 << 28;

// PWR_CR2 @ 0x04: IOSV (bit 9) removes the VDDIO2 isolation on PG[15:2]
// (CMSIS `PWR_CR2_IOSV_Pos = 9`; HAL_PWREx_EnableVddIO2()).
const PWR_CR2: *mut u32 = (0x4000_7000 + 0x04) as *mut u32;
const PWR_CR2_IOSV: u32 = 1 << 9;

// GPIO register offsets (STM32 GPIO v2 block).
const MODER: u32 = 0x00;
const IDR: u32 = 0x10;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const AFRL: u32 = 0x20;
const AFRH: u32 = 0x24;

const GPIOA: u32 = 0x4202_0000;
const GPIOB: u32 = 0x4202_0400;
const GPIOC: u32 = 0x4202_0800;
const GPIOG: u32 = 0x4202_1800;

// LPUART1 (USART v2 register layout): CR1 @ 0x00, BRR @ 0x0C, ISR @ 0x1C,
// TDR @ 0x28.
const LPUART1: u32 = 0x4000_8000;
const LPUART_CR1: *mut u32 = LPUART1 as *mut u32;
const LPUART_CR3: *mut u32 = (LPUART1 + 0x08) as *mut u32;
const LPUART_BRR: *mut u32 = (LPUART1 + 0x0C) as *mut u32;
const LPUART_ISR: *const u32 = (LPUART1 + 0x1C) as *const u32;
const LPUART_RDR: *const u32 = (LPUART1 + 0x24) as *const u32;
const LPUART_TDR: *mut u32 = (LPUART1 + 0x28) as *mut u32;
const NVIC_ISPR2: *const u32 = 0xE000_E208 as *const u32;
const NVIC_ICPR2: *mut u32 = 0xE000_E288 as *mut u32;
const NVIC_LPUART1_PENDING: u32 = 1 << 2; // IRQ 66, from the full STM32L552 profile.

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
const TAILCHAIN_THREAD_R0: u32 = 0x5448_5244;
const TAILCHAIN_HANDLER_R0: u32 = 0x5045_4E44;

const USART_ISR_TXE: u32 = 1 << 7;
const USART_ISR_TC: u32 = 1 << 6;
const USART_ISR_RXNE: u32 = 1 << 5;
const USART_CR1_UE: u32 = 1 << 0;
const USART_CR1_RE: u32 = 1 << 2;
const USART_CR1_TE: u32 = 1 << 3;
const USART_CR1_TCIE: u32 = 1 << 6;
const USART_CR1_TXEIE: u32 = 1 << 7;
const USART_CR1_RXNEIE: u32 = 1 << 5;
const USART_CR1_FIFOEN: u32 = 1 << 29;
const USART_CR3_RXFTIE: u32 = 1 << 28;
const USART_CR3_RXFTCFG: u32 = 0b111 << 25;
const USART_ISR_RXFT: u32 = 1 << 26;

#[derive(Clone, Copy)]
struct Led {
    name: &'static [u8],
    port: u32,
    pin: u32,
}

const LEDS: [Led; 3] = [
    Led {
        name: b"ld1",
        port: GPIOC,
        pin: 7,
    },
    Led {
        name: b"ld2",
        port: GPIOB,
        pin: 7,
    },
    Led {
        name: b"ld3",
        port: GPIOA,
        pin: 9,
    },
];

fn putc(byte: u8) {
    unsafe {
        while read_volatile(LPUART_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(LPUART_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn put_hex32(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

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
        (AFRH, (pin - 8) * 4)
    };
    reg_rmw(base + reg, 0xF << shift, af << shift);
}

fn led_drive(led: Led, on: bool) {
    let bit = if on {
        1 << led.pin
    } else {
        1 << (led.pin + 16)
    };
    unsafe { write_volatile((led.port + BSRR) as *mut u32, bit) };
}

fn report(on: bool, led: Led) {
    puts(b"led=");
    puts(led.name);
    puts(if on { b" odr=" } else { b"_off odr=" });
    put_hex32(unsafe { read_volatile((led.port + ODR) as *const u32) });
    putc(b'\n');
}

fn button_level() -> u32 {
    unsafe { (read_volatile((GPIOC + IDR) as *const u32) >> 13) & 1 }
}

/// Exercise every STM32v2 RX FIFO threshold encoding and the threshold IRQ on
/// the imported LPUART1 profile. The injected stream starts with six bytes
/// for the threshold sweep and leaves `R`, `S` for the separate RXNE smoke.
fn smoke_lpuart_rx_fifo_threshold() {
    let cr1 = unsafe { read_volatile(LPUART_CR1) };
    unsafe { write_volatile(LPUART_CR1, cr1 | USART_CR1_FIFOEN) };

    let cr3 = unsafe { read_volatile(LPUART_CR3) } & !(USART_CR3_RXFTCFG | USART_CR3_RXFTIE);
    // The model maps RXFTCFG 5/4/3/2/1/0 to thresholds 8/7/6/4/2/1.
    unsafe { write_volatile(LPUART_CR3, cr3 | (5 << 25)) };
    spin(4);
    let threshold_8 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    let mut bytes = [0u8; 6];
    bytes[0] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    spin(4);
    let below_8 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT == 0;

    unsafe { write_volatile(LPUART_CR3, cr3 | (4 << 25)) };
    spin(4);
    let threshold_7 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    bytes[1] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    spin(4);
    let below_7 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT == 0;

    // At six queued bytes, check RXFTIE isolation and level re-pending.
    unsafe { write_volatile(LPUART_CR3, cr3 | (3 << 25)) };
    spin(4);
    let threshold_6 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let irq_isolated = unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING == 0;

    unsafe { write_volatile(LPUART_CR3, cr3 | (3 << 25) | USART_CR3_RXFTIE) };
    spin(4);
    let irq_asserted = unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0;
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let irq_repended = unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0;

    bytes[2] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    spin(4);
    let below_6 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT == 0;
    // Clear NVIC after the UART source went low; it must stay deasserted.
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let irq_released = unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING == 0;

    unsafe { write_volatile(LPUART_CR3, cr3 | (2 << 25)) };
    spin(4);
    let threshold_4 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    bytes[3] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    bytes[4] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    spin(4);
    let below_4 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT == 0;

    unsafe { write_volatile(LPUART_CR3, cr3 | (1 << 25)) };
    spin(4);
    let threshold_2 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;

    unsafe { write_volatile(LPUART_CR3, cr3) };
    spin(4);
    let threshold_1 = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    bytes[5] = unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 };
    spin(4);
    let threshold_1_after_pop = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT != 0;
    unsafe { write_volatile(LPUART_CR1, cr1 & !USART_CR1_FIFOEN) };
    spin(4);
    let fifo_disabled_clears = unsafe { read_volatile(LPUART_ISR) } & USART_ISR_RXFT == 0;
    unsafe { write_volatile(LPUART_CR1, cr1) };

    let bytes_ok = bytes == *b"abcdef";
    let threshold_checks = [
        threshold_8,
        below_8,
        threshold_7,
        below_7,
        threshold_6,
        below_6,
        threshold_4,
        below_4,
        threshold_2,
        threshold_1,
        threshold_1_after_pop,
        fifo_disabled_clears,
    ];
    let thresholds_ok = threshold_checks.iter().all(|passed| *passed);
    let ok =
        thresholds_ok && irq_isolated && irq_asserted && irq_repended && irq_released && bytes_ok;
    puts(b"rxft=lpuart1 thresholds=8,7,6,4,2,1 boundary=");
    putc(if thresholds_ok { b'1' } else { b'0' });
    puts(b" states=");
    for passed in threshold_checks {
        putc(if passed { b'1' } else { b'0' });
    }
    puts(b" irq=");
    putc(if irq_isolated { b'1' } else { b'0' });
    putc(if irq_asserted { b'1' } else { b'0' });
    putc(if irq_repended { b'1' } else { b'0' });
    putc(if irq_released { b'1' } else { b'0' });
    puts(b" bytes=");
    for byte in bytes {
        putc(byte);
    }
    puts(if ok { b" ok\n" } else { b" FAIL\n" });
}

fn smoke_lpuart_rx() {
    let rxne_before = unsafe { read_volatile(LPUART_ISR) & USART_ISR_RXNE != 0 };
    let irq_isolated = unsafe { read_volatile(NVIC_ISPR2) & NVIC_LPUART1_PENDING == 0 };
    unsafe {
        let cr1 = read_volatile(LPUART_CR1);
        write_volatile(LPUART_CR1, cr1 | USART_CR1_RXNEIE);
    }
    spin(4);
    let irq_asserted = unsafe { read_volatile(NVIC_ISPR2) & NVIC_LPUART1_PENDING != 0 };
    let first = if rxne_before {
        unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 }
    } else {
        0
    };
    let second_still_pending =
        rxne_before && unsafe { read_volatile(LPUART_ISR) & USART_ISR_RXNE != 0 };

    // RXNE is a level while the second byte remains queued. Clearing the NVIC
    // latch must therefore let the LPUART assert IRQ 66 again.
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let irq_reasserted = unsafe { read_volatile(NVIC_ISPR2) & NVIC_LPUART1_PENDING != 0 };

    let second = if second_still_pending {
        unsafe { (read_volatile(LPUART_RDR) & 0xFF) as u8 }
    } else {
        0
    };
    let rxne_after = unsafe { read_volatile(LPUART_ISR) & USART_ISR_RXNE != 0 };
    // With the queue drained, clearing the captured interrupt must stay clear
    // even though RXNEIE remains enabled.
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let irq_released = unsafe { read_volatile(NVIC_ISPR2) & NVIC_LPUART1_PENDING == 0 };
    unsafe {
        let cr1 = read_volatile(LPUART_CR1);
        write_volatile(LPUART_CR1, cr1 & !USART_CR1_RXNEIE);
    }
    let ok = rxne_before
        && irq_isolated
        && irq_asserted
        && first == b'R'
        && second_still_pending
        && irq_reasserted
        && second == b'S'
        && !rxne_after
        && irq_released;

    puts(b"rx=lpuart1 rxne_before=");
    putc(if rxne_before { b'1' } else { b'0' });
    puts(b" irq_isolated=");
    putc(if irq_isolated { b'1' } else { b'0' });
    puts(b" irq_asserted=");
    putc(if irq_asserted { b'1' } else { b'0' });
    puts(b" reasserted=");
    putc(if irq_reasserted { b'1' } else { b'0' });
    puts(b" data=");
    put_hex32(u32::from(first));
    putc(b',');
    put_hex32(u32::from(second));
    puts(b" rxne_after=");
    putc(if rxne_after { b'1' } else { b'0' });
    puts(b" irq_released=");
    putc(if irq_released { b'1' } else { b'0' });
    puts(if ok { b" ok\n" } else { b" FAIL\n" });
}

/// Check that LPUART1 TXE and TC independently drive IRQ 66, re-pend while
/// their source remains active, and combine on the same NVIC line.
fn smoke_lpuart_tx_irq() {
    let cr1 = unsafe { read_volatile(LPUART_CR1) } & !(USART_CR1_TXEIE | USART_CR1_TCIE);
    unsafe { write_volatile(LPUART_CR1, cr1) };

    let mut txe_ready = false;
    for _ in 0..100_000 {
        if unsafe { read_volatile(LPUART_ISR) } & USART_ISR_TXE != 0 {
            txe_ready = true;
            break;
        }
    }
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let txe_isolated =
        txe_ready && unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING == 0;
    unsafe { write_volatile(LPUART_CR1, cr1 | USART_CR1_TXEIE) };
    let mut txe_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            txe_asserted = true;
            break;
        }
    }
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    let mut txe_repended = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            txe_repended = true;
            break;
        }
    }
    unsafe {
        write_volatile(LPUART_CR1, cr1);
        write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING);
    }
    let mut txe_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            txe_released = false;
            break;
        }
    }

    let mut tc_ready = false;
    for _ in 0..100_000 {
        if unsafe { read_volatile(LPUART_ISR) } & USART_ISR_TC != 0 {
            tc_ready = true;
            break;
        }
    }
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    spin(4);
    let tc_isolated = tc_ready && unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING == 0;
    unsafe { write_volatile(LPUART_CR1, cr1 | USART_CR1_TCIE) };
    let mut tc_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            tc_asserted = true;
            break;
        }
    }
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    let mut tc_repended = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            tc_repended = true;
            break;
        }
    }
    unsafe {
        write_volatile(LPUART_CR1, cr1);
        write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING);
    }
    let mut tc_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            tc_released = false;
            break;
        }
    }

    unsafe { write_volatile(LPUART_CR1, cr1 | USART_CR1_TXEIE | USART_CR1_TCIE) };
    let mut both_asserted = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            both_asserted = true;
            break;
        }
    }
    unsafe { write_volatile(LPUART_CR1, cr1 | USART_CR1_TCIE) };
    unsafe { write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING) };
    let mut tc_survives_txe_disable = false;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            tc_survives_txe_disable = true;
            break;
        }
    }
    unsafe {
        write_volatile(LPUART_CR1, cr1);
        write_volatile(NVIC_ICPR2, NVIC_LPUART1_PENDING);
    }
    let mut combined_released = true;
    for _ in 0..10_000 {
        if unsafe { read_volatile(NVIC_ISPR2) } & NVIC_LPUART1_PENDING != 0 {
            combined_released = false;
            break;
        }
    }

    let txe_ok = txe_isolated && txe_asserted && txe_repended && txe_released;
    let tc_ok = tc_isolated && tc_asserted && tc_repended && tc_released;
    let combined_ok = both_asserted && tc_survives_txe_disable && combined_released;
    puts(b"tx_irq lpuart1 txe=");
    putc(if txe_ok { b'1' } else { b'0' });
    puts(b" tc=");
    putc(if tc_ok { b'1' } else { b'0' });
    puts(b" both=");
    putc(if combined_ok { b'1' } else { b'0' });
    puts(if txe_ok && tc_ok && combined_ok {
        b" ok\n"
    } else {
        b" FAIL\n"
    });
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

/// Verify the Cortex-M33 tail-chains from SysTick into lower-priority PendSV
/// while retaining the exception frame and SysTick's live R0 value.
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
            (m33_tailchain_pendsv_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            (TEST_VECTOR_TABLE + 15 * 4) as *mut u32,
            (m33_tailchain_systick_handler as *const () as usize as u32) | 1,
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
        unsafe { m33_pend_systick_with_thread_r0() };
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

    puts(b"systick pendsv m33 tailchain frame=");
    putc(if frame_reused { b'1' } else { b'0' });
    puts(b" r0=");
    putc(if r0_live { b'1' } else { b'0' });
    puts(b" pending=");
    putc(if deferred && child_is_pendsv && cleanup {
        b'1'
    } else {
        b'0'
    });
    puts(b" ipsr=");
    put_dec32(child_ipsr);
    puts(if passed { b" ok\n" } else { b" FAIL\n" });
    passed
}

/// Set a Thread-mode R0 marker and software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn m33_pend_systick_with_thread_r0() {
    core::arch::naked_asm!(
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "bx lr",
    );
}

/// Record SysTick's active frame, pend PendSV, and return with a new R0 marker.
#[unsafe(naked)]
unsafe extern "C" fn m33_tailchain_systick_handler() {
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

/// Capture the tail-chained PendSV state before the Thread frame is unstacked.
#[unsafe(naked)]
unsafe extern "C" fn m33_tailchain_pendsv_handler() {
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

#[entry]
fn main() -> ! {
    unsafe {
        // GPIOA (LD3), GPIOB (LD2), GPIOC (LD1 + button), GPIOG (LPUART1).
        write_volatile(
            RCC_AHB2ENR,
            read_volatile(RCC_AHB2ENR) | (1 << 0) | (1 << 1) | (1 << 2) | (1 << 6),
        );
        // LPUART1EN: APB1ENR2 bit 0.
        write_volatile(RCC_APB1ENR2, read_volatile(RCC_APB1ENR2) | 1);

        // PG7/PG8 are VDDIO2 pins: clock the PWR block, then set IOSV so
        // the port G pads are no longer isolated. Without this the VCP
        // stays silent on the real board.
        write_volatile(
            RCC_APB1ENR1,
            read_volatile(RCC_APB1ENR1) | RCC_APB1ENR1_PWREN,
        );
        write_volatile(PWR_CR2, read_volatile(PWR_CR2) | PWR_CR2_IOSV);

        // PG7/PG8 as AF8 (LPUART1_TX/RX).
        pin_af(GPIOG, 7, 8);
        pin_af(GPIOG, 8, 8);

        // The three LEDs as push-pull outputs, initially low.
        for led in LEDS {
            reg_rmw(
                led.port + MODER,
                0b11 << (led.pin * 2),
                0b01 << (led.pin * 2),
            );
            led_drive(led, false);
        }
        // PC13 as input (B1).
        let moder = (GPIOC + MODER) as *mut u32;
        write_volatile(moder, read_volatile(moder) & !(0b11 << 26));

        // MSI 4 MHz / 115200 with the LPUART 256x divider.
        write_volatile(LPUART_BRR, 0x22B9);
        // The UART model must not capture a TDR write until UE and TE are set.
        write_volatile(LPUART_TDR, b'!' as u32);
        write_volatile(LPUART_CR1, USART_CR1_UE | USART_CR1_TE | USART_CR1_RE);
    }

    puts(b"NUCLEO-L552ZE-Q SMOKE\n");
    puts(b"cpu=cortex-m33 uart=lpuart1\n");
    smoke_lpuart_rx_fifo_threshold();
    smoke_lpuart_rx();
    smoke_lpuart_tx_irq();
    let _ = smoke_systick_pendsv_tailchain();

    for led in LEDS {
        led_drive(led, true);
        report(true, led);
        led_drive(led, false);
        report(false, led);
    }

    // Leave LD1 lit so the heartbeat's ODR readback is non-zero.
    led_drive(LEDS[0], true);

    puts(b"btn=");
    put_dec32(button_level());
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" odr=");
        put_hex32(unsafe { read_volatile((GPIOC + ODR) as *const u32) });
        puts(b" btn=");
        put_dec32(button_level());
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
