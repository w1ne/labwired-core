// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! NUCLEO-F767ZI smoke firmware.
//!
//! The board's STM32F767ZIT6 is a Cortex-M7 with 2 MiB of flash and 512 KiB
//! of SRAM. The console here is USART1 (the board routes the ST-LINK VCP to
//! USART3 on PD8/PD9; every U(S)ART is brought up and USART3 prints too).
//! LD1 (green) PB0, LD2 (blue) PB7 and LD3 (red) PB14 are active high, and
//! B1 USER on PC13 is active high (UM1974 §6.5/§6.6).
//!
//! Renode builds the board as `using "platforms/cpus/stm32f777.repl"`, so
//! the row loads the stm32f777 silicon profile and this checks that surface
//! through the shared `f746_checks` / `f767_checks` modules: RCC reset state,
//! clock gate and the F767 APB1/APB2 masks, the 512 KiB / 2 MiB geometry,
//! all eight U(S)ART lines, GPIO reset state, SYSCFG (SWP_FB), EXTI software
//! triggers and line 24, TIM1..14, SPI4/5/6 interrupt lines, DMA2
//! memory-to-memory, I2C1..4 NACK lines, CRC and RNG. The board IO adds
//! the three LEDs and B1 (rising edge through SYSCFG EXTICR4 into
//! EXTI15_10). SPI frames, SPI DMA and the TMP102 need slaves the board does
//! not have and are proven by the stm32f777 fixture smoke instead.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's ISPR
//! bit. Every check prints one line and sets one bit of the summary word at
//! `SUMMARY`.

#![no_std]
#![no_main]

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../../firmware-stm32f746-smoke/src/f746_checks.rs"]
mod f746_checks;
#[path = "../f767_checks.rs"]
mod f767_checks;
use f746_checks::*;
use f767_checks::*;

#[cfg(feature = "nucleo-f767zi-m7-fpu")]
#[path = "../m7_fpu_smoke.rs"]
mod m7_fpu_smoke;

const E_RTSR: u32 = 0x08;
const EXTI15_10_IRQ: u32 = 40;
/// LD1 (green) PB0, LD2 (blue) PB7, LD3 (red) PB14.
const LEDS: [(&str, u32); 3] = [("ld1", 0), ("ld2", 7), ("ld3", 14)];
/// B1 USER on PC13.
const B1: u32 = 13;

/// STM32v2 USART3 RX data and status registers. The board VCP is USART3 and
/// its NVIC line is 39 on the imported STM32F777-backed profile.
const USART3: u32 = 0x4000_4800;
const USART3_RDR: u32 = USART3 + 0x24;
const USART3_ISR: u32 = USART3 + 0x1C;
const USART3_CR1: u32 = USART3;
const USART3_IRQ: u32 = 39;
const USART_UE: u32 = 1 << 0;
const USART_RXNE: u32 = 1 << 5;
const USART_RXNEIE: u32 = 1 << 5;
const USART_RE: u32 = 1 << 2;
const USART_TXE: u32 = 1 << 7;
const USART_TC: u32 = 1 << 6;
const USART_TXEIE: u32 = 1 << 7;
const USART_TCIE: u32 = 1 << 6;

const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const TEST_VECTOR_TABLE: u32 = 0x2000_0080;
const PENDSV_HANDLER_COUNT: u32 = 0x2000_3010;
const PENDSV_HANDLER_IPSR: u32 = 0x2000_3014;
const SYSTICK_HANDLER_COUNT: u32 = 0x2000_3018;
const SYSTICK_HANDLER_IPSR: u32 = 0x2000_301C;
const SYSTICK_PENDSV_ARMED: u32 = 0x2000_3020;
const SYSTICK_PENDSV_FLAGS: u32 = 0x2000_3024;
const PENDSV_SYSTICK_COUNT_AT_ENTRY: u32 = 0x2000_3028;
const TAILCHAIN_RECORD: u32 = 0x2000_3040;
const TAILCHAIN_THREAD_R0: u32 = 0x5448_5244;
const TAILCHAIN_HANDLER_R0: u32 = 0x5045_4E44;
const PSP_TAILCHAIN_RECORD: u32 = 0x2000_3100;
const PSP_TAILCHAIN_STACK_TOP: u32 = 0x2000_3200;

/// Each LED as a push-pull output: three blinks through BSRR, each level
/// visible in ODR and IDR, and only its own ODR bit set while lit.
fn check_leds() -> bool {
    let mut ok = true;
    puts("leds ");
    for &(name, pin) in &LEDS {
        let moder = rd(GPIOB + G_MODER) & !(0b11 << (2 * pin));
        wr(GPIOB + G_MODER, moder | (0b01 << (2 * pin)));
        let mut blink = true;
        for _ in 0..3 {
            wr(GPIOB + G_BSRR, 1 << pin);
            blink &= rd(GPIOB + G_ODR) & (1 << pin) != 0 && rd(GPIOB + G_IDR) & (1 << pin) != 0;
            wr(GPIOB + G_BSRR, 1 << (pin + 16));
            blink &= rd(GPIOB + G_ODR) & (1 << pin) == 0 && rd(GPIOB + G_IDR) & (1 << pin) == 0;
        }
        wr(GPIOB + G_BSRR, 1 << pin);
        let lit = rd(GPIOB + G_ODR) == 1 << pin;
        wr(GPIOB + G_BSRR, 1 << (pin + 16));
        let this = blink && lit && rd(GPIOB + G_ODR) == 0;
        flag(name, this);
        ok &= this;
    }
    verdict(ok);
    ok
}

/// B1 on PC13 (pressed = high; input with pull-down) through the SYSCFG
/// port mux into EXTI line 13 on the rising edge. With EXTICR4 selecting
/// port A the first press must not pend line 13; with port C the second
/// press must, on NVIC EXTI15_10 = 40 and no other EXTI vector.
fn check_b1() -> bool {
    let pupdr = rd(GPIOC + G_PUPDR) & !(0b11 << (2 * B1));
    wr(GPIOC + G_PUPDR, pupdr | (0b10 << (2 * B1)));
    let moder = rd(GPIOC + G_MODER) & !(0b11 << (2 * B1));
    wr(GPIOC + G_MODER, moder);
    wr(SYSCFG_EXTICR4, CODE_PA << 4);
    wr(EXTI + E_RTSR, 1 << B1);
    wr(EXTI + E_IMR, 1 << B1);
    unpend(EXTI15_10_IRQ);

    let idle = wait_level(GPIOC, B1, false);
    let press1 = wait_level(GPIOC, B1, true) && wait_level(GPIOC, B1, false);
    let other_port_quiet = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);

    wr(SYSCFG_EXTICR4, CODE_PC << 4);
    let press2 = wait_level(GPIOC, B1, true) && wait_level(GPIOC, B1, false);
    let pc13 = rd(EXTI + E_PR) & (1 << B1) != 0
        && pending(EXTI15_10_IRQ)
        && EXTI_LINES
            .iter()
            .all(|&o| o == EXTI15_10_IRQ || !pending(o));
    wr(EXTI + E_PR, 1 << B1);
    unpend(EXTI15_10_IRQ);
    let cleared = rd(EXTI + E_PR) & (1 << B1) == 0 && !pending(EXTI15_10_IRQ);
    wr(EXTI + E_IMR, 0);
    wr(EXTI + E_RTSR, 0);
    wr(SYSCFG_EXTICR4, 0);

    puts("b1 ");
    flag("idle", idle);
    flag("presses", press1 && press2);
    flag("pa_quiet", other_port_quiet);
    flag("pc13", pc13);
    flag("cleared", cleared);
    let ok = idle && press1 && press2 && other_port_quiet && pc13 && cleared;
    verdict(ok);
    ok
}

/// Receive the two start-injected bytes on the board's USART3 VCP. First
/// verify they remain on the wire while RE is clear, then enable RE and check
/// RXNE isolation, assertion, re-pending, and release after both bytes drain.
fn check_usart3_rx() -> bool {
    let receiver_disabled = rd(USART3_CR1) & (USART_UE | USART_RE) == USART_UE;
    unpend(USART3_IRQ);
    let rxne_held_while_disabled = rd(USART3_ISR) & USART_RXNE == 0;
    let irq_held_while_disabled = !pending(USART3_IRQ);
    wr(USART3_CR1, rd(USART3_CR1) | USART_RE);

    let mut first_ready = false;
    for _ in 0..100_000 {
        if rd(USART3_ISR) & USART_RXNE != 0 {
            first_ready = true;
            break;
        }
    }

    unpend(USART3_IRQ);
    let rxne_irq_isolated = first_ready && !pending(USART3_IRQ);
    wr(USART3_CR1, rd(USART3_CR1) | USART_RXNEIE);
    let mut rxne_irq_asserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            rxne_irq_asserted = true;
            break;
        }
    }

    let first = if first_ready {
        rd(USART3_RDR) & 0xFF
    } else {
        0
    };
    let second_still_pending = first_ready && rd(USART3_ISR) & USART_RXNE != 0;
    unpend(USART3_IRQ);
    let mut second_irq_reasserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            second_irq_reasserted = true;
            break;
        }
    }

    let mut second = 0;
    let mut second_ready = false;
    for _ in 0..100_000 {
        if rd(USART3_ISR) & USART_RXNE != 0 {
            second = rd(USART3_RDR) & 0xFF;
            second_ready = true;
            break;
        }
    }
    let cleared = rd(USART3_ISR) & USART_RXNE == 0;
    wr(USART3_CR1, rd(USART3_CR1) & !USART_RXNEIE);
    unpend(USART3_IRQ);
    let mut rxne_irq_released = true;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            rxne_irq_released = false;
            break;
        }
    }

    let ok = first_ready
        && first == b'R' as u32
        && second_ready
        && second == b'S' as u32
        && cleared
        && rxne_irq_isolated
        && receiver_disabled
        && rxne_held_while_disabled
        && irq_held_while_disabled
        && rxne_irq_asserted
        && second_still_pending
        && second_irq_reasserted
        && rxne_irq_released;
    puts("usart3 rxne ");
    flag(
        "re_off",
        receiver_disabled && rxne_held_while_disabled && irq_held_while_disabled,
    );
    flag("isolated", rxne_irq_isolated);
    flag("asserted", rxne_irq_asserted);
    flag("reasserted", second_irq_reasserted);
    flag("released", rxne_irq_released);
    flag(
        "data",
        first == b'R' as u32 && second == b'S' as u32 && cleared,
    );
    verdict(ok);
    ok
}

/// Check USART3 TXE and TC interrupt levels independently. Each source must
/// stay isolated while its enable is clear, assert when enabled, re-pend after
/// NVIC pending is cleared while the source remains active, and release when
/// its enable is cleared.
fn check_usart3_tx_irq() -> bool {
    let mut txe_ready = false;
    for _ in 0..100_000 {
        if rd(USART3_ISR) & USART_TXE != 0 {
            txe_ready = true;
            break;
        }
    }

    let cr1 = rd(USART3_CR1) & !(USART_TXEIE | USART_TCIE);
    wr(USART3_CR1, cr1);
    unpend(USART3_IRQ);
    let txe_irq_isolated = txe_ready && !pending(USART3_IRQ);
    wr(USART3_CR1, cr1 | USART_TXEIE);
    let mut txe_irq_asserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            txe_irq_asserted = true;
            break;
        }
    }
    unpend(USART3_IRQ);
    let mut txe_irq_reasserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            txe_irq_reasserted = true;
            break;
        }
    }
    wr(USART3_CR1, cr1);
    unpend(USART3_IRQ);
    let mut txe_irq_released = true;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            txe_irq_released = false;
            break;
        }
    }

    let mut tc_ready = false;
    for _ in 0..100_000 {
        if rd(USART3_ISR) & USART_TC != 0 {
            tc_ready = true;
            break;
        }
    }
    unpend(USART3_IRQ);
    let tc_irq_isolated = tc_ready && !pending(USART3_IRQ);
    wr(USART3_CR1, cr1 | USART_TCIE);
    let mut tc_irq_asserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            tc_irq_asserted = true;
            break;
        }
    }
    unpend(USART3_IRQ);
    let mut tc_irq_reasserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            tc_irq_reasserted = true;
            break;
        }
    }
    wr(USART3_CR1, cr1);
    unpend(USART3_IRQ);
    let mut tc_irq_released = true;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            tc_irq_released = false;
            break;
        }
    }

    let both = cr1 | USART_TXEIE | USART_TCIE;
    wr(USART3_CR1, both);
    let mut both_irq_asserted = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            both_irq_asserted = true;
            break;
        }
    }
    wr(USART3_CR1, both & !USART_TXEIE);
    unpend(USART3_IRQ);
    let mut tc_irq_survives_txe_disable = false;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            tc_irq_survives_txe_disable = true;
            break;
        }
    }
    wr(USART3_CR1, cr1);
    unpend(USART3_IRQ);
    let mut combined_irq_released = true;
    for _ in 0..10_000 {
        if pending(USART3_IRQ) {
            combined_irq_released = false;
            break;
        }
    }

    let ok = txe_irq_isolated
        && txe_irq_asserted
        && txe_irq_reasserted
        && txe_irq_released
        && tc_irq_isolated
        && tc_irq_asserted
        && tc_irq_reasserted
        && tc_irq_released
        && both_irq_asserted
        && tc_irq_survives_txe_disable
        && combined_irq_released;
    puts("usart3 tx_irq ");
    flag(
        "txe",
        txe_irq_isolated && txe_irq_asserted && txe_irq_reasserted && txe_irq_released,
    );
    flag(
        "tc",
        tc_irq_isolated && tc_irq_asserted && tc_irq_reasserted && tc_irq_released,
    );
    flag(
        "both",
        both_irq_asserted && tc_irq_survives_txe_disable && combined_irq_released,
    );
    verdict(ok);
    ok
}

/// PENDSVCLR cancels a masked request, a fresh request enters vector 14, and
/// lowest-priority PendSV requested by SysTick waits for the SysTick return.
fn check_pendsv_m7() -> bool {
    const RELOAD: u32 = 63;
    const PENDSV_EXCEPTION: u32 = 14;

    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        PendSV_Handler as *const () as usize as u32,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        SysTick_Handler as *const () as usize as u32,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    // PendSV is lowest priority; SysTick has the highest configurable priority.
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000);
    wr(PENDSV_HANDLER_COUNT, 0);
    wr(PENDSV_HANDLER_IPSR, 0);
    wr(SYSTICK_HANDLER_COUNT, 0);
    wr(SYSTICK_HANDLER_IPSR, 0);
    wr(SYSTICK_PENDSV_ARMED, 0);
    wr(SYSTICK_PENDSV_FLAGS, 0);
    wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, 0);

    wr(SCB_ICSR, ICSR_PENDSVSET);
    let pending_before_clear = rd(SCB_ICSR) & ICSR_PENDSVSET != 0;
    wr(SCB_ICSR, ICSR_PENDSVCLR);
    let pending_after_clear = rd(SCB_ICSR) & ICSR_PENDSVSET == 0;
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..32 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let clear_cancels_pending = prior_primask == 0
        && pending_before_clear
        && pending_after_clear
        && rd(PENDSV_HANDLER_COUNT) == 0;

    // A clear must not poison a later set; the request should enter once.
    unsafe { asm!("cpsid i", options(nomem, nostack, preserves_flags)) };
    wr(SCB_ICSR, ICSR_PENDSVCLR);
    wr(SCB_ICSR, ICSR_PENDSVSET);
    let fresh_pending_visible = rd(SCB_ICSR) & ICSR_PENDSVSET != 0;
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..4_096 {
        if rd(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    for _ in 0..32 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let fresh_repend_dispatches_once = fresh_pending_visible
        && rd(PENDSV_HANDLER_COUNT) == 1
        && rd(PENDSV_HANDLER_IPSR) == PENDSV_EXCEPTION;

    // SysTick requests PendSV while active. The lower-priority handler must
    // run only after exception 15 returns to thread mode.
    unsafe { asm!("cpsid i", options(nomem, nostack, preserves_flags)) };
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(PENDSV_HANDLER_COUNT, 0);
    wr(PENDSV_HANDLER_IPSR, 0);
    wr(SYSTICK_HANDLER_COUNT, 0);
    wr(SYSTICK_HANDLER_IPSR, 0);
    wr(SYSTICK_PENDSV_FLAGS, 0);
    wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, 0);
    wr(SYSTICK_PENDSV_ARMED, 1);
    wr(SYSTICK_RVR, RELOAD);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock.
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..4_096 {
        if rd(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let masked_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
    }
    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_PENDSV_ARMED, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    let systick_entries = rd(SYSTICK_HANDLER_COUNT);
    let systick_ipsr = rd(SYSTICK_HANDLER_IPSR);
    let pendsv_entries = rd(PENDSV_HANDLER_COUNT);
    let pendsv_ipsr = rd(PENDSV_HANDLER_IPSR);
    let priority_flags = rd(SYSTICK_PENDSV_FLAGS);
    let systick_entries_at_pendsv = rd(PENDSV_SYSTICK_COUNT_AT_ENTRY);
    let priority_configured = rd(SCB_SHPR3) & 0xFFFF_0000 == 0x00FF_0000;
    let vector_installed = rd(SCB_VTOR) == TEST_VECTOR_TABLE;
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..32 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let stable_after_disable = rd(PENDSV_HANDLER_COUNT) == pendsv_entries;

    unsafe { asm!("cpsid i", options(nomem, nostack, preserves_flags)) };
    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_PENDSV_ARMED, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SCB_VTOR, prior_vtor);
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let ok = prior_primask == 0
        && clear_cancels_pending
        && fresh_repend_dispatches_once
        && systick_entries == 1
        && systick_ipsr == 15
        && pendsv_entries == 1
        && pendsv_ipsr == PENDSV_EXCEPTION
        && priority_flags == 0b11
        && systick_entries_at_pendsv == 1
        && priority_configured
        && vector_installed
        && stable_after_disable;
    puts("pendsv m7 ");
    flag("clear", clear_cancels_pending);
    flag("repend", fresh_repend_dispatches_once);
    flag("systick", systick_entries == 1 && systick_ipsr == 15);
    flag(
        "priority",
        priority_flags == 0b11 && systick_entries_at_pendsv == 1,
    );
    verdict(ok);
    ok
}

/// Verify a lower-priority PendSV takes SysTick's live context through the
/// M7 tail-chain path instead of unstacking to Thread mode first.
fn check_systick_pendsv_tailchain_m7() -> bool {
    const PENDSTSET: u32 = 1 << 26;
    const PENDSTCLR: u32 = 1 << 25;
    const PENDSVSET: u32 = 1 << 28;
    const PENDSVCLR: u32 = 1 << 27;

    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }
    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    let prior_rvr = rd(SYSTICK_RVR);
    let prior_csr = rd(SYSTICK_CSR) & 0x7;
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, PENDSTCLR | PENDSVCLR);
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        (m7_tailchain_pendsv_handler as *const () as usize as u32) | 1,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        (m7_tailchain_systick_handler as *const () as usize as u32) | 1,
    );
    for word in 0..12 {
        wr(TAILCHAIN_RECORD + word * 4, 0);
    }
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    unsafe {
        asm!("dsb", "isb", options(nostack, preserves_flags));
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { pend_systick_with_thread_r0_m7() };
    }
    for _ in 0..4_096 {
        if rd(TAILCHAIN_RECORD + 11 * 4) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let parent_ipsr = rd(TAILCHAIN_RECORD);
    let parent_msp = rd(TAILCHAIN_RECORD + 4);
    let parent_exc_return = rd(TAILCHAIN_RECORD + 8);
    let parent_r0 = rd(TAILCHAIN_RECORD + 12);
    let parent_icsr_before = rd(TAILCHAIN_RECORD + 16);
    let parent_icsr_after = rd(TAILCHAIN_RECORD + 20);
    let child_ipsr = rd(TAILCHAIN_RECORD + 24);
    let child_msp = rd(TAILCHAIN_RECORD + 28);
    let child_exc_return = rd(TAILCHAIN_RECORD + 32);
    let child_r0 = rd(TAILCHAIN_RECORD + 36);
    let child_icsr = rd(TAILCHAIN_RECORD + 40);
    let child_hit = rd(TAILCHAIN_RECORD + 44) == 1;
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
    }

    let masked_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
    }
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, PENDSTCLR | PENDSVCLR);
    wr(SCB_VTOR, prior_vtor);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SYSTICK_RVR, prior_rvr);
    wr(SYSTICK_CSR, prior_csr);
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_before & PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFF9
        && child_exc_return == parent_exc_return;
    let child_is_pendsv =
        child_hit && child_ipsr == 14 && child_icsr & 0x1FF == 14 && child_icsr & PENDSVSET == 0;
    let r0_live = parent_r0 == TAILCHAIN_THREAD_R0
        && child_r0 == TAILCHAIN_HANDLER_R0
        && child_r0 != parent_r0;
    let cleanup = rd(SCB_VTOR) == prior_vtor
        && rd(SCB_SHPR3) == prior_shpr3
        && rd(SCB_ICSR) & (PENDSTSET | PENDSVSET) == 0;
    let passed = prior_primask == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && thread_ipsr == 0
        && cleanup;
    puts("pendsv m7 tailchain ");
    flag("deferred", deferred);
    flag("frame_reused", frame_reused);
    flag("r0_live", r0_live);
    flag("pending_clear", child_is_pendsv && cleanup);
    verdict(passed);
    passed
}

/// Verify Cortex-M7 tail chaining when Thread mode uses the RTOS task stack.
fn check_systick_pendsv_psp_tailchain_m7() -> bool {
    const PENDSTSET: u32 = 1 << 26;
    const PENDSTCLR: u32 = 1 << 25;
    const PENDSVSET: u32 = 1 << 28;
    const PENDSVCLR: u32 = 1 << 27;

    let prior_primask: u32;
    let source_control: u32;
    let source_psp: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "mrs {control}, CONTROL",
            "mrs {psp}, PSP",
            "cpsid i",
            primask = out(reg) prior_primask,
            control = out(reg) source_control,
            psp = out(reg) source_psp,
            options(nomem, nostack, preserves_flags),
        );
    }
    let source_msp: u32;
    unsafe { asm!("mrs {}, MSP", out(reg) source_msp, options(nomem, nostack, preserves_flags)) };
    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    let prior_rvr = rd(SYSTICK_RVR);
    let prior_csr = rd(SYSTICK_CSR) & 0x7;

    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, PENDSTCLR | PENDSVCLR);
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x80C0_0000);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        (m7_psp_tailchain_pendsv_handler as *const () as usize as u32) | 1,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        (m7_psp_tailchain_systick_handler as *const () as usize as u32) | 1,
    );
    for word in 0..17 {
        wr(PSP_TAILCHAIN_RECORD + word * 4, 0);
    }
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    unsafe { asm!("dsb", "isb", options(nostack, preserves_flags)) };
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { pend_systick_from_psp_m7() };
    }

    let record = PSP_TAILCHAIN_RECORD;
    let parent_ipsr = rd(record);
    let parent_icsr_before = rd(record + 4);
    let parent_icsr_after = rd(record + 8);
    let parent_msp = rd(record + 12);
    let parent_exc_return = rd(record + 16);
    let thread_r0 = rd(record + 20);
    let child_ipsr = rd(record + 24);
    let child_msp = rd(record + 28);
    let child_exc_return = rd(record + 32);
    let child_r0 = rd(record + 36);
    let child_icsr = rd(record + 40);
    let child_hit = rd(record + 44) == 1;
    let parent_psp = rd(record + 48);
    let child_psp = rd(record + 52);
    let wrapper_msp = rd(record + 56);
    let wrapper_psp = rd(record + 60);
    let wrapper_control = rd(record + 64);
    let thread_ipsr: u32;
    let msp_after: u32;
    let psp_after: u32;
    let control_after: u32;
    unsafe {
        asm!(
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

    unsafe { asm!("cpsid i", options(nomem, nostack, preserves_flags)) };
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, PENDSTCLR | PENDSVCLR);
    wr(SCB_VTOR, prior_vtor);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SYSTICK_RVR, prior_rvr);
    wr(SYSTICK_CSR, prior_csr);
    unsafe {
        asm!(
            "msr PSP, {psp}",
            "msr CONTROL, {control}",
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            psp = in(reg) source_psp,
            control = in(reg) source_control,
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let deferred = parent_ipsr == 15
        && parent_icsr_before & 0x1FF == 15
        && parent_icsr_before & PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFFD
        && child_exc_return == parent_exc_return
        && parent_psp == child_psp
        && parent_psp == PSP_TAILCHAIN_STACK_TOP.wrapping_sub(32)
        && wrapper_msp == source_msp
        && parent_msp == source_msp.wrapping_sub(16);
    let child_is_pendsv =
        child_hit && child_ipsr == 14 && child_icsr & 0x1FF == 14 && child_icsr & PENDSVSET == 0;
    let r0_live = thread_r0 == TAILCHAIN_THREAD_R0
        && child_r0 == TAILCHAIN_HANDLER_R0
        && child_r0 != thread_r0;
    let cleanup = rd(SCB_VTOR) == prior_vtor
        && rd(SCB_SHPR3) == prior_shpr3
        && rd(SCB_ICSR) & (PENDSTSET | PENDSVSET) == 0
        && msp_after == source_msp
        && psp_after == source_psp
        && control_after == source_control
        && wrapper_psp == source_psp
        && wrapper_control == source_control
        && thread_ipsr == 0;
    let passed = prior_primask == 0
        && source_control & 2 == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && cleanup;
    puts("pendsv m7 psp tailchain ");
    flag("deferred", deferred);
    flag("frame_reused", frame_reused);
    flag("r0_live", r0_live);
    flag("pending_clear", child_is_pendsv && cleanup);
    verdict(passed);
    passed
}

/// Set Thread mode to use a dedicated PSP, then pend SysTick with a marker.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_from_psp_m7() {
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
        "1:",
        "ldr r2, [r1, #44]",
        "cmp r2, #0",
        "beq 1b",
        "msr psp, r5",
        "msr control, r4",
        "isb",
        "pop {{r4, r5, r6, pc}}",
    );
}

/// Pend PendSV while SysTick is active; the core should tail-chain on PSP.
#[unsafe(naked)]
unsafe extern "C" fn m7_psp_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r3, r0",
        "ldr r0, =0x20003100",
        "str r3, [r0, #20]",
        "mrs r3, ipsr",
        "str r3, [r0, #0]",
        "mrs r3, msp",
        "str r3, [r0, #12]",
        "mrs r3, psp",
        "str r3, [r0, #48]",
        "mov r3, lr",
        "str r3, [r0, #16]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "movs r2, #1",
        "lsls r2, r2, #28",
        "str r2, [r1]",
        "ldr r2, [r1]",
        "str r2, [r0, #8]",
        "ldr r0, =0x50454E44",
        "bx lr",
    );
}

/// Record PendSV state before the original Thread PSP frame is unstacked.
#[unsafe(naked)]
unsafe extern "C" fn m7_psp_tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "mrs r3, ipsr",
        "ldr r0, =0x20003100",
        "str r3, [r0, #24]",
        "mrs r3, msp",
        "str r3, [r0, #28]",
        "mov r3, lr",
        "str r3, [r0, #32]",
        "str r2, [r0, #36]",
        "mrs r3, psp",
        "str r3, [r0, #52]",
        "ldr r3, =0xE000ED04",
        "ldr r3, [r3]",
        "str r3, [r0, #40]",
        "movs r3, #1",
        "str r3, [r0, #44]",
        "bx lr",
    );
}

/// Set a distinct Thread-mode R0 marker and software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_with_thread_r0_m7() {
    core::arch::naked_asm!(
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "bx lr",
    );
}

/// Record SysTick's exception frame, pend lower-priority PendSV, and return
/// with a distinct live R0 value for the tail-chained handler to observe.
#[unsafe(naked)]
unsafe extern "C" fn m7_tailchain_systick_handler() {
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

/// Capture vector, stack, EXC_RETURN, live R0, and pending state before the
/// original Thread frame is unstacked.
#[unsafe(naked)]
unsafe extern "C" fn m7_tailchain_pendsv_handler() {
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

/// SysTick exception handler installed at vector-table slot 15.
#[no_mangle]
pub extern "C" fn SysTick_Handler() {
    unsafe {
        let ipsr: u32;
        asm!("mrs {ipsr}, IPSR", ipsr = out(reg) ipsr, options(nomem, nostack, preserves_flags));
        wr(SYSTICK_HANDLER_IPSR, ipsr);
        wr(
            SYSTICK_HANDLER_COUNT,
            rd(SYSTICK_HANDLER_COUNT).wrapping_add(1),
        );
        if rd(SYSTICK_PENDSV_ARMED) != 0 && rd(SYSTICK_HANDLER_COUNT) == 1 {
            wr(SYSTICK_CSR, 0);
            wr(SCB_ICSR, ICSR_PENDSVSET);
            let pending = u32::from(rd(SCB_ICSR) & ICSR_PENDSVSET != 0);
            let not_preempted = u32::from(rd(PENDSV_HANDLER_COUNT) == 0);
            wr(SYSTICK_PENDSV_FLAGS, pending | (not_preempted << 1));
        }
    }
}

/// PendSV exception handler installed at vector-table slot 14.
#[no_mangle]
pub extern "C" fn PendSV_Handler() {
    unsafe {
        let ipsr: u32;
        asm!("mrs {ipsr}, IPSR", ipsr = out(reg) ipsr, options(nomem, nostack, preserves_flags));
        wr(PENDSV_HANDLER_IPSR, ipsr);
        wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, rd(SYSTICK_HANDLER_COUNT));
        wr(
            PENDSV_HANDLER_COUNT,
            rd(PENDSV_HANDLER_COUNT).wrapping_add(1),
        );
    }
}

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup_f767();
    // A TDR write before UE/TE must not appear in the captured serial stream.
    // Exercise all eight STM32v2 UART instances in this imported profile.
    for &(_, base, _) in &UARTS {
        send(base, b"!");
    }
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    // The imported smoke injects two bytes before firmware starts. USART3 is
    // left with RE clear here so the RX check can verify receiver gating.
    puts("NUCLEO-F767ZI SMOKE\r\n");
    send(UARTS[2].1, b"usart3 vcp ok\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate, rcc_mask));
    mark(1, check_geometry());
    let usart_irqs = check_usart_irqs();
    let usart3_rx = check_usart3_rx();
    let usart3_tx = check_usart3_tx_irq();
    mark(2, usart_irqs && usart3_rx && usart3_tx);
    mark(3, check_leds());
    mark(4, check_syscfg_f767());
    mark(5, check_exti_swier() && check_exti_line24());
    mark(6, check_b1());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(7, timers);
    mark(8, check_spi456_lines());
    mark(9, check_dma_m2m());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(10 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(14, check_crc());
    mark(15, check_rng());
    mark(16, check_pendsv_m7());
    mark(17, check_systick_pendsv_tailchain_m7());
    mark(18, check_systick_pendsv_psp_tailchain_m7());
    #[cfg(feature = "nucleo-f767zi-m7-fpu")]
    m7_fpu_smoke::run();
    let summary = summary | 0x7670_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
