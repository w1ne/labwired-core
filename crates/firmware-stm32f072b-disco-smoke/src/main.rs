// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! STM32F072B-DISCO smoke firmware.
//!
//! The board's STM32F072RBT6 is a Cortex-M0 with 128 KiB of flash at
//! 0x08000000 and 16 KiB of SRAM at 0x20000000. The simulator console is
//! USART1 (PA9/PA10, AF1); the on-board ST-LINK/V2 is SWD-only. The four user
//! LEDs are PC6 (LD3 red), PC8 (LD4 orange), PC9 (LD5 green) and PC7 (LD6
//! blue), all active high; the user button B1 drives PA0 high when pressed
//! (UM1690 §6.4/§6.5).
//!
//! The chip surface is the stm32f072 profile (Renode builds the board on
//! `stm32f072.repl`), so besides the board IO this checks that the board row
//! loads that surface: RCC clock gates, USART2..4 lines and, in the onboarding
//! build, USART2 RX queued until both UE and RE are enabled. It also checks
//! the B1 edge through SYSCFG EXTICR into EXTI on the grouped vector, all nine
//! timers on their own
//! lines, DMA1 memory-to-memory on channel 1, CRC and the ADC on its internal
//! channels (the board wires no analog source). SPI1/2 and I2C1 have no
//! on-board slave and are proven by the stm32f072 fixture smoke instead.
//! Bases and IRQ numbers are RM0091 and ST's CMSIS `stm32f072xb.h`.
//!
//! Peripheral IRQ checks read each line's ISPR bit without enabling it. The
//! final software-pended IRQ 5 takes a real Cortex-M0 exception through a RAM
//! vector table and checks IPSR, IABR, the basic stacked frame, and exception
//! return. It also pends NMI from that active IRQ while PRIMASK is set and
//! checks NMI entry and return to the IRQ. The NMI handler queues a lower
//! priority IRQ, which must remain pending through both NMI entries and the
//! active IRQ before tail-chaining after that IRQ returns. Every check prints
//! one line and sets one bit of the summary word at `SUMMARY`. AIRCR.PRIGROUP
//! is also checked as reserved on this ARMv6-M core. A short SysTick period
//! verifies core exception 15, COUNTFLAG read-clear behavior, and PRIMASK's
//! pending-then-deliver behavior.

#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const SUMMARY: u32 = 0x2000_3000;
const SCB_AIRCR: u32 = 0xE000_ED0C;
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const AIRCR_VECTKEY: u32 = 0x05FA << 16;
const ICSR_NMIPENDSET: u32 = 1 << 31;
const RAM_VECTOR_BASE: u32 = 0x2000_1000;
const VECTOR_TABLE_WORDS: usize = 48;
const NMI_VECTOR_INDEX: usize = 2;
const EXTI0_1_VECTOR_INDEX: usize = 16 + EXTI0_1_IRQ as usize;
const IRQ6_VECTOR_INDEX: usize = 16 + 6;
const SYSTICK_VECTOR_INDEX: usize = 15;
const IRQ_HIT: u32 = SUMMARY + 4;
const IRQ_IPSR: u32 = SUMMARY + 8;
const IRQ_ACTIVE: u32 = SUMMARY + 12;
const IRQ_PENDING: u32 = SUMMARY + 16;
const IRQ_FRAME: u32 = SUMMARY + 20;
const IRQ_EXCEPTION_MSP: u32 = SUMMARY + 60;
const NMI_RECORD: u32 = SUMMARY + 64;
const NMI_COUNT: u32 = NMI_RECORD + 36;
const NMI_DEFERRED_IRQ: u32 = SUMMARY + 104;
const SYSTICK_RECORD: u32 = SUMMARY + 176;
const SYSTICK_MASK_RECORD: u32 = SUMMARY + 192;
const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;

// RCC (RM0091 §6.4).
const RCC: u32 = 0x4002_1000;
const RCC_CR: u32 = RCC;
const RCC_AHBENR: u32 = RCC + 0x14;
const RCC_APB2ENR: u32 = RCC + 0x18;
const RCC_APB1ENR: u32 = RCC + 0x1C;
// DMA, SRAM, FLITF, CRC, IOPA..IOPF.
const AHB_ON: u32 = (1 << 0) | (1 << 2) | (1 << 4) | (1 << 6) | (0x3F << 17);
// SYSCFG, ADC, TIM1, SPI1, USART1, TIM15/16/17.
const APB2_ON: u32 = (1 << 0) | (1 << 9) | (1 << 11) | (1 << 12) | (1 << 14) | (0b111 << 16);
// TIM2/3, TIM6/7, TIM14, SPI2, USART2/3/4, I2C1.
const APB1_ON: u32 = 0b11 | (0b11 << 4) | (1 << 8) | (1 << 14) | (0b111 << 17) | (1 << 21);

// USARTx ("v2" layout).
const USART1: u32 = 0x4001_3800;
const USART2: u32 = 0x4000_4400;
const USART3: u32 = 0x4000_4800;
const USART4: u32 = 0x4000_4C00;
const U_CR1: u32 = 0x00;
const U_BRR: u32 = 0x0C;
const U_ISR: u32 = 0x1C;
const U_RDR: u32 = 0x24;
const U_TDR: u32 = 0x28;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_ISR_RXNE: u32 = 1 << 5;
const USART_CR1_TXEIE: u32 = 1 << 7;

// GPIO.
const GPIOA: u32 = 0x4800_0000;
const GPIOC: u32 = 0x4800_0800;
const GPIOD: u32 = 0x4800_0C00;
const G_MODER: u32 = 0x00;
const G_PUPDR: u32 = 0x0C;
const G_IDR: u32 = 0x10;
const G_ODR: u32 = 0x14;
const G_BSRR: u32 = 0x18;
const G_BRR: u32 = 0x28;
const G_AFRH: u32 = 0x24;

const LED_LD3: u32 = 1 << 6; // PC6, red
const LED_LD4: u32 = 1 << 8; // PC8, orange
const LED_LD5: u32 = 1 << 9; // PC9, green
const LED_LD6: u32 = 1 << 7; // PC7, blue
const LEDS: [(&str, u32); 4] = [
    ("ld3", LED_LD3),
    ("ld4", LED_LD4),
    ("ld5", LED_LD5),
    ("ld6", LED_LD6),
];

// SYSCFG / EXTI.
const SYSCFG_EXTICR1: u32 = 0x4001_0008;
const EXTI: u32 = 0x4001_0400;
const E_IMR: u32 = 0x00;
const E_RTSR: u32 = 0x08;
const E_PR: u32 = 0x14;
const CODE_PA: u32 = 0;
const CODE_PC: u32 = 2;
const EXTI0_1_IRQ: u32 = 5;

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

// DMA1 (channel n registers at 0x08 + 20 * (n - 1)).
const DMA1: u32 = 0x4002_0000;
const D_ISR: u32 = 0x00;
const D_IFCR: u32 = 0x04;
const DMA1_CH1: u32 = DMA1 + 0x08;
const C_CCR: u32 = 0x00;
const C_CNDTR: u32 = 0x04;
const C_CPAR: u32 = 0x08;
const C_CMAR: u32 = 0x0C;
const DMA_SRC: u32 = 0x2000_3100;
const DMA_DST: u32 = 0x2000_3200;
/// DMA1_Channel1_IRQn, DMA1_Channel2_3_IRQn, DMA1_Channel4_5_6_7_IRQn.
const DMA_LINES: [u32; 3] = [9, 10, 11];

// CRC.
const CRC: u32 = 0x4002_3000;
const CRC_DR: u32 = CRC;
const CRC_CR: u32 = CRC + 0x08;

// ADC (RM0091 §13.12).
const ADC: u32 = 0x4001_2400;
const ADC_ISR: u32 = ADC;
const ADC_IER: u32 = ADC + 0x04;
const ADC_CR: u32 = ADC + 0x08;
const ADC_SMPR: u32 = ADC + 0x14;
const ADC_CHSELR: u32 = ADC + 0x28;
const ADC_DR: u32 = ADC + 0x40;
const ADC_CCR: u32 = ADC + 0x308;
/// ADC1_COMP_IRQn.
const ADC_LINE: u32 = 12;
const ADC_ISR_ADRDY: u32 = 1 << 0;
const ADC_ISR_EOC: u32 = 1 << 2;
const ADC_ISR_EOS: u32 = 1 << 3;
const ADC_CR_ADEN: u32 = 1 << 0;
const ADC_CR_ADDIS: u32 = 1 << 1;
const ADC_CR_ADSTART: u32 = 1 << 2;
const ADC_CR_ADCAL: u32 = 1 << 31;
const ADC_CCR_VREFEN: u32 = 1 << 22;
const ADC_CCR_TSEN: u32 = 1 << 23;
/// 12-bit TS (IN16, 25 °C: 1.43 V + 5 K * 4.3 mV/K) and V_REFINT (IN17,
/// 1.23 V) at V_DDA = 3.3 V (DS9826).
const ADC_WANT: [u32; 2] = [1801, 1526];

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ICPR0: u32 = 0xE000_E280;
const NVIC_IABR0: u32 = 0xE000_E300;
const NVIC_IPR1: u32 = 0xE000_E404;

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

/// RAM vector used by `check_exception_entry`. Capture the hardware-created
/// frame before a compiler prologue can move MSP, and leave UART work to the
/// thread-mode caller.
#[unsafe(naked)]
unsafe extern "C" fn smoke_exti0_1_handler() {
    core::arch::naked_asm!(
        "mrs r3, ipsr",
        "ldr r0, =0x20003008",
        "str r3, [r0]",
        "mrs r1, msp",
        "ldr r0, =0x20003014",
        "ldr r2, [r1, #0]",
        "str r2, [r0, #0]",
        "ldr r2, [r1, #4]",
        "str r2, [r0, #4]",
        "ldr r2, [r1, #8]",
        "str r2, [r0, #8]",
        "ldr r2, [r1, #12]",
        "str r2, [r0, #12]",
        "ldr r2, [r1, #16]",
        "str r2, [r0, #16]",
        "ldr r2, [r1, #20]",
        "str r2, [r0, #20]",
        "ldr r2, [r1, #24]",
        "str r2, [r0, #24]",
        "ldr r2, [r1, #28]",
        "str r2, [r0, #28]",
        "mov r2, lr",
        "str r2, [r0, #32]",
        "str r1, [r0, #36]",
        "ldr r0, =0xe000e300",
        "ldr r2, [r0]",
        "movs r3, #0x20",
        "ands r2, r3",
        "ldr r0, =0x2000300c",
        "str r2, [r0]",
        "ldr r0, =0xe000e200",
        "ldr r2, [r0]",
        "ands r2, r3",
        "ldr r0, =0x20003010",
        "str r2, [r0]",
        "ldr r0, =0xe000e180",
        "movs r2, #0x20",
        "str r2, [r0]",
        "ldr r0, =0x20003004",
        "movs r2, #1",
        "str r2, [r0]",
        "bx lr",
    );
}

/// Pend NMI from an active external interrupt with PRIMASK set. NMI must
/// preempt; after its exception return the IRQ handler must resume masked.
#[unsafe(naked)]
unsafe extern "C" fn smoke_irq_pend_nmi_handler() {
    core::arch::naked_asm!(
        "mrs r3, ipsr",
        "ldr r0, =0x20003040",
        "str r3, [r0, #0]",
        "cpsid i",
        "ldr r0, =0xe000ed04",
        "ldr r1, =0x80000000",
        "str r1, [r0]",
        "dsb",
        "isb",
        "mrs r3, ipsr",
        "ldr r0, =0x20003040",
        "str r3, [r0, #8]",
        "mrs r3, PRIMASK",
        "str r3, [r0, #12]",
        "cpsie i",
        "mrs r3, ipsr",
        "ldr r0, =0x20003070",
        "str r3, [r0, #0]",
        "ldr r1, =0xe000e300",
        "ldr r2, [r1]",
        "movs r3, #0x40",
        "ands r2, r3",
        "str r2, [r0, #4]",
        "ldr r1, =0xe000e200",
        "ldr r2, [r1]",
        "movs r3, #0x40",
        "ands r2, r3",
        "str r2, [r0, #8]",
        "bx lr",
    );
}

/// Capture NMI state, pend lower-priority IRQ 6, then pend NMI again once while
/// NMI is active. IRQ 6 must remain pending through both NMI entries and the
/// active IRQ 5, then tail-chain after IRQ 5 returns.
#[unsafe(naked)]
unsafe extern "C" fn smoke_nmi_handler() {
    core::arch::naked_asm!(
        "mrs r3, ipsr",
        "ldr r0, =0x20003040",
        "str r3, [r0, #4]",
        "ldr r1, =0xe000ed04",
        "ldr r2, [r1]",
        "str r2, [r0, #16]",
        "ldr r1, =0xe000e300",
        "ldr r2, [r1]",
        "movs r3, #0x20",
        "ands r2, r3",
        "str r2, [r0, #20]",
        "mrs r2, PRIMASK",
        "str r2, [r0, #24]",
        "ldr r3, =0x20003064",
        "ldr r1, [r3]",
        "adds r1, #1",
        "str r1, [r3]",
        "cmp r1, #1",
        "bne 1f",
        "ldr r3, =0xe000e200",
        "movs r2, #0x40",
        "str r2, [r3]",
        "dsb",
        "isb",
        "ldr r2, [r3]",
        "movs r3, #0x40",
        "ands r2, r3",
        "ldr r3, =0x20003068",
        "str r2, [r3]",
        "ldr r3, =0xe000e300",
        "ldr r2, [r3]",
        "movs r3, #0x40",
        "ands r2, r3",
        "ldr r3, =0x2000306c",
        "str r2, [r3]",
        "ldr r3, =0xe000ed04",
        "ldr r2, =0x80000000",
        "str r2, [r3]",
        "dsb",
        "isb",
        "mrs r2, ipsr",
        "str r2, [r0, #28]",
        "str r1, [r0, #32]",
        "1:",
        "bx lr",
    );
}

/// Record IRQ 6 after IRQ 5 returns. Its lower priority prevents preemption of
/// IRQ 5 even after PRIMASK is cleared, but it must run before thread resumes.
#[unsafe(naked)]
unsafe extern "C" fn smoke_irq6_deferred_handler() {
    core::arch::naked_asm!(
        "mrs r3, ipsr",
        "ldr r0, =0x2000307c",
        "str r3, [r0, #4]",
        "ldr r3, =0xe000e300",
        "ldr r2, [r3]",
        "str r2, [r0, #8]",
        "ldr r3, =0xe000e200",
        "ldr r2, [r3]",
        "str r2, [r0, #12]",
        "ldr r3, =0x20003064",
        "ldr r2, [r3]",
        "str r2, [r0, #16]",
        "movs r2, #1",
        "str r2, [r0, #0]",
        "bx lr",
    );
}

/// Capture the SysTick system exception and stop the timer before it can
/// re-pend. The caller checks COUNTFLAG after exception return.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_handler() {
    core::arch::naked_asm!(
        "mrs r2, ipsr",
        "ldr r0, =0x200030B0",
        "str r2, [r0, #0]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "ldr r1, =0xE000E010",
        "movs r2, #0",
        "str r2, [r1]",
        "movs r2, #1",
        "str r2, [r0, #8]",
        "bx lr",
    );
}

/// Capture a SysTick exception after it pends while PRIMASK is set.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_masked_handler() {
    core::arch::naked_asm!(
        "mrs r2, ipsr",
        "ldr r0, =0x200030C0",
        "str r2, [r0, #0]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "mrs r2, PRIMASK",
        "str r2, [r0, #12]",
        "ldr r1, =0xE000E010",
        "movs r2, #0",
        "str r2, [r1]",
        "movs r2, #1",
        "str r2, [r0, #8]",
        "bx lr",
    );
}

/// Exercise the imported Cortex-M SysTick model. Its zero-hit must enter
/// system exception 15, set COUNTFLAG, and clear the flag on a word CSR read.
fn check_systick() -> bool {
    let source_vtor = rd(SCB_VTOR);
    let table = RAM_VECTOR_BASE as *mut u32;
    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        wr(SYSTICK_CSR, 0);
        wr(SYSTICK_RVR, 64);
        wr(SYSTICK_CVR, 0);
        write_volatile(SYSTICK_RECORD as *mut u32, 0);
        write_volatile((SYSTICK_RECORD + 4) as *mut u32, 0);
        write_volatile((SYSTICK_RECORD + 8) as *mut u32, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    for _ in 0..100_000 {
        if rd(SYSTICK_RECORD + 8) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_ipsr = rd(SYSTICK_RECORD);
    let icsr_in_handler = rd(SYSTICK_RECORD + 4);
    let handler_hit = rd(SYSTICK_RECORD + 8) == 1;
    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SYSTICK_CSR, 0);
    }
    let csr_first = rd(SYSTICK_CSR);
    let csr_second = rd(SYSTICK_CSR);
    let icsr_after = rd(SCB_ICSR);
    let ipsr_after: u32;
    unsafe {
        asm!("mrs {value}, ipsr", value = out(reg) ipsr_after, options(nomem, nostack, preserves_flags));
        wr(SCB_ICSR, 1 << 25); // Clear a pending SysTick on the failure path.
        wr(SYSTICK_CVR, 0);
        wr(SCB_VTOR, source_vtor);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    let vtor_restored = rd(SCB_VTOR) == source_vtor;

    let countflag = csr_first & (1 << 16) != 0;
    let countflag_cleared = csr_second & (1 << 16) == 0;
    let active_exception = icsr_in_handler & 0x1FF == 15;
    let pending_cleared = icsr_after & (1 << 26) == 0;
    let ok = handler_hit
        && handler_ipsr == 15
        && active_exception
        && countflag
        && countflag_cleared
        && pending_cleared
        && ipsr_after == 0
        && vtor_restored;

    puts("systick ipsr=");
    hex(handler_ipsr);
    puts(" active=");
    hex(icsr_in_handler & 0x1FF);
    puts(" ");
    flag("countflag", countflag);
    flag("read_clear", countflag_cleared);
    puts("pending_after=");
    hex((icsr_after >> 26) & 1);
    puts(" thread_ipsr=");
    hex(ipsr_after);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// SysTick must become pending while PRIMASK blocks delivery, then enter once
/// interrupts are unmasked. This checks the core exception path against the
/// imported target rather than only checking the timer's COUNTFLAG registers.
fn check_systick_primask() -> bool {
    let source_vtor = rd(SCB_VTOR);
    let table = RAM_VECTOR_BASE as *mut u32;
    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, 1 << 25);
        wr(SYSTICK_RVR, 64);
        wr(SYSTICK_CVR, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        for index in 0..4 {
            write_volatile((SYSTICK_MASK_RECORD as *mut u32).add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_masked_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SYSTICK_CSR, 0x7);
    }

    for _ in 0..100_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let first_pending = rd(SCB_ICSR) & (1 << 26) != 0;
    unsafe {
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, 1 << 25);
    }
    let icsr_after_clear = rd(SCB_ICSR);
    unsafe {
        asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags));
    }
    for _ in 0..64 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let handler_hit_after_clear = rd(SYSTICK_MASK_RECORD + 8) != 0;

    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SYSTICK_RVR, 64);
        wr(SYSTICK_CVR, 0);
        wr(SYSTICK_CSR, 0x7);
    }
    for _ in 0..100_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_hit_while_masked = rd(SYSTICK_MASK_RECORD + 8) != 0;
    let icsr_while_masked = rd(SCB_ICSR);
    unsafe {
        asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags));
    }
    for _ in 0..100_000 {
        if rd(SYSTICK_MASK_RECORD + 8) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_ipsr = rd(SYSTICK_MASK_RECORD);
    let icsr_in_handler = rd(SYSTICK_MASK_RECORD + 4);
    let handler_hit = rd(SYSTICK_MASK_RECORD + 8) == 1;
    let primask_in_handler = rd(SYSTICK_MASK_RECORD + 12);
    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, 1 << 25);
        wr(SYSTICK_CVR, 0);
        wr(SCB_VTOR, source_vtor);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let icsr_after = rd(SCB_ICSR);
    let ipsr_after: u32;
    unsafe {
        asm!("mrs {value}, ipsr", value = out(reg) ipsr_after, options(nomem, nostack, preserves_flags));
    }
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let pending_while_masked = icsr_while_masked & (1 << 26) != 0;
    let thread_while_masked = icsr_while_masked & 0x1FF == 0;
    let pending_clear_honored = icsr_after_clear & (1 << 26) == 0 && !handler_hit_after_clear;
    let active_exception = icsr_in_handler & 0x1FF == 15;
    let pending_cleared = icsr_after & (1 << 26) == 0;
    let ok = prior_primask == 0
        && first_pending
        && pending_clear_honored
        && !handler_hit_while_masked
        && pending_while_masked
        && thread_while_masked
        && handler_hit
        && handler_ipsr == 15
        && active_exception
        && primask_in_handler == 0
        && pending_cleared
        && ipsr_after == 0
        && vtor_restored;

    puts("systick primask clear=");
    flag("honored", pending_clear_honored);
    puts("masked=");
    puts(
        if pending_while_masked && !handler_hit_while_masked && thread_while_masked {
            "1 "
        } else {
            "0 "
        },
    );
    puts("ipsr=");
    hex(handler_ipsr);
    puts(" active=");
    hex(icsr_in_handler & 0x1FF);
    puts(" primask=");
    hex(primask_in_handler);
    puts(" pending_after=");
    hex((icsr_after >> 26) & 1);
    puts(" thread_ipsr=");
    hex(ipsr_after);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// An NMI pended by a running IRQ must enter despite PRIMASK and return to
/// that IRQ before the deferred, lower-priority IRQ 6 can run.
fn check_nmi_repend_and_deferred_irq() -> bool {
    let source_vtor = rd(SCB_VTOR);
    let nmi_initially_clear = rd(SCB_ICSR) & ICSR_NMIPENDSET == 0;
    let source_priorities = rd(NVIC_IPR1);
    // IPR1 stores IRQ 4 in bits 7:0, IRQ 5 in 15:8, and IRQ 6 in 23:16.
    // Cortex-M priorities use the most-significant implemented bits, with a
    // lower numeric value taking precedence. IRQ 5 = 0x40, IRQ 6 = 0x80.
    let test_priority_bits = 0x0080_4000;
    let table = RAM_VECTOR_BASE as *mut u32;
    let irq_bit = 1 << EXTI0_1_IRQ;
    let deferred_irq_bit = 1 << 6;
    let irq_mask = irq_bit | deferred_irq_bit;
    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(NVIC_ICER0, irq_mask);
        unpend(EXTI0_1_IRQ);
        unpend(6);
        for index in 0..10 {
            write_volatile((NMI_RECORD as *mut u32).add(index), 0);
        }
        for index in 0..10 {
            write_volatile((NMI_DEFERRED_IRQ as *mut u32).add(index), 0);
        }
        wr(
            NVIC_IPR1,
            (source_priorities & !0x00FF_FF00) | test_priority_bits,
        );
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(NMI_VECTOR_INDEX),
            (smoke_nmi_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            table.add(EXTI0_1_VECTOR_INDEX),
            (smoke_irq_pend_nmi_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            table.add(IRQ6_VECTOR_INDEX),
            (smoke_irq6_deferred_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(NVIC_ISER0, irq_mask);
        wr(NVIC_ISPR0, irq_bit);
        asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    for _ in 0..100_000 {
        if unsafe { read_volatile((NMI_RECORD as *const u32).add(2)) } != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let irq_ipsr = unsafe { read_volatile(NMI_RECORD as *const u32) };
    let nmi_ipsr = unsafe { read_volatile((NMI_RECORD as *const u32).add(1)) };
    let resumed_ipsr = unsafe { read_volatile((NMI_RECORD as *const u32).add(2)) };
    let primask_in_nmi = unsafe { read_volatile((NMI_RECORD as *const u32).add(3)) };
    let icsr_in_nmi = unsafe { read_volatile((NMI_RECORD as *const u32).add(4)) };
    let irq_active_in_nmi = unsafe { read_volatile((NMI_RECORD as *const u32).add(5)) };
    let primask_after_nmi = unsafe { read_volatile((NMI_RECORD as *const u32).add(6)) };
    let nmi_count = rd(NMI_COUNT);
    let ipsr_after_repend = unsafe { read_volatile((NMI_RECORD as *const u32).add(7)) };
    let count_after_repend = unsafe { read_volatile((NMI_RECORD as *const u32).add(8)) };
    let deferred_pending_in_nmi = unsafe { read_volatile(NMI_DEFERRED_IRQ as *const u32) };
    let deferred_active_in_nmi = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(1)) };
    let ipsr_after_irq_unmask = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(2)) };
    let deferred_active_in_irq = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(3)) };
    let deferred_pending_in_irq = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(4)) };
    let deferred_irq_hit = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(5)) };
    let deferred_irq_ipsr = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(6)) };
    let deferred_irq_active = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(7)) };
    let deferred_irq_pending = unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(8)) };
    let nmi_count_in_deferred_irq =
        unsafe { read_volatile((NMI_DEFERRED_IRQ as *const u32).add(9)) };
    let priority_readback = rd(NVIC_IPR1) & 0x00FF_FF00;
    let irq5_priority = (priority_readback >> 8) & 0xFF;
    let irq6_priority = (priority_readback >> 16) & 0xFF;
    let active_after = rd(NVIC_IABR0) & irq_mask;
    let pending_after = rd(NVIC_ISPR0) & irq_mask;

    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(NVIC_ICER0, irq_mask);
        unpend(EXTI0_1_IRQ);
        unpend(6);
        wr(NVIC_IPR1, source_priorities);
        if nmi_ipsr == 2 {
            wr(SCB_VTOR, source_vtor);
        }
        asm!("cpsie i", options(nomem, nostack, preserves_flags));
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let cleanup_ok = active_after == 0 && pending_after == 0;
    let deferred_ok = priority_readback == test_priority_bits
        && deferred_pending_in_nmi == deferred_irq_bit
        && deferred_active_in_nmi == 0
        && ipsr_after_irq_unmask == 16 + EXTI0_1_IRQ
        && deferred_active_in_irq == 0
        && deferred_pending_in_irq == deferred_irq_bit
        && deferred_irq_hit == 1
        && deferred_irq_ipsr == 16 + 6
        && deferred_irq_active == deferred_irq_bit
        && deferred_irq_pending == 0
        && nmi_count_in_deferred_irq == 2;
    let ok = nmi_initially_clear
        && prior_primask == 0
        && irq_ipsr == 16 + EXTI0_1_IRQ
        && nmi_ipsr == 2
        && nmi_count == 2
        && ipsr_after_repend == 2
        && count_after_repend == 1
        && resumed_ipsr == 16 + EXTI0_1_IRQ
        && primask_in_nmi == 1
        && icsr_in_nmi & 0x1FF == 2
        && irq_active_in_nmi == irq_bit
        && primask_after_nmi == 1
        && deferred_ok
        && cleanup_ok
        && rd(NVIC_IPR1) == source_priorities
        && (nmi_ipsr != 2 || rd(SCB_VTOR) == source_vtor);

    puts("nmi repend ");
    flag("entered", nmi_ipsr == 2);
    puts("entries=");
    hex(nmi_count);
    puts(" after_pend_ipsr=");
    hex(ipsr_after_repend);
    puts(" after_pend_entries=");
    hex(count_after_repend);
    puts(" irq=");
    hex(irq_ipsr);
    puts(" nmi=");
    hex(nmi_ipsr);
    puts(" resumed=");
    hex(resumed_ipsr);
    puts(" icsr=");
    hex(icsr_in_nmi & 0x1FF);
    puts(" irq_active=");
    hex(irq_active_in_nmi);
    puts(" primask=");
    hex(primask_in_nmi);
    puts(" restored=");
    hex(primask_after_nmi);
    puts(" cleanup=");
    flag(
        "ok",
        cleanup_ok && (nmi_ipsr != 2 || rd(SCB_VTOR) == source_vtor),
    );
    verdict(ok);
    puts("nmi deferred irq pending_in_nmi=");
    hex(deferred_pending_in_nmi);
    puts(" active_in_nmi=");
    hex(deferred_active_in_nmi);
    puts(" after_unmask_ipsr=");
    hex(ipsr_after_irq_unmask);
    puts(" irq6_active_while_irq5=");
    hex(deferred_active_in_irq);
    puts(" irq6_pending_in_irq5=");
    hex(deferred_pending_in_irq);
    puts(" irq_ipsr=");
    hex(deferred_irq_ipsr);
    puts(" irq_active=");
    hex(deferred_irq_active);
    puts(" irq_pending=");
    hex(deferred_irq_pending);
    puts(" nmi_count=");
    hex(nmi_count_in_deferred_irq);
    puts(" irq5_priority=");
    hex(irq5_priority);
    puts(" irq6_priority=");
    hex(irq6_priority);
    puts(" ");
    verdict(deferred_ok && rd(NVIC_IPR1) == source_priorities);
    ok
}

/// Set known caller-saved register values immediately before pends IRQ 5.
/// The two saved callee-saved registers keep this helper ABI-correct while
/// leaving the exception frame eight-byte aligned.
#[unsafe(naked)]
unsafe extern "C" fn pend_exti0_1_irq_with_sentinels() {
    core::arch::naked_asm!(
        "push {{r4, r7}}",
        "mrs r7, msp",
        "ldr r4, =0x2000303c",
        "str r7, [r4]",
        "ldr r7, =0xe000e200",
        "movs r4, #0x20",
        "movs r0, #0x11",
        "movs r1, #0x22",
        "movs r2, #0x33",
        "movs r3, #0x44",
        "movs r4, #0x55",
        "mov r12, r4",
        "movs r4, #0x20",
        "str r4, [r7]",
        "pop {{r4, r7}}",
        "bx lr",
    );
}

/// Take a software-pended IRQ 5 through the imported Cortex-M0's exception
/// path, then check that the active and pending state clears on return.
fn check_exception_entry() -> bool {
    let source_vtor = rd(SCB_VTOR);
    let table = RAM_VECTOR_BASE as *mut u32;
    let irq_bit = 1 << EXTI0_1_IRQ;
    unsafe {
        wr(NVIC_ICER0, irq_bit);
        unpend(EXTI0_1_IRQ);
        write_volatile(IRQ_HIT as *mut u32, 0);
        write_volatile(IRQ_IPSR as *mut u32, 0);
        write_volatile(IRQ_ACTIVE as *mut u32, 0);
        write_volatile(IRQ_PENDING as *mut u32, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        let handler = (smoke_exti0_1_handler as *const () as usize as u32) | 1;
        write_volatile(table.add(EXTI0_1_VECTOR_INDEX), handler);
        wr(SCB_VTOR, RAM_VECTOR_BASE);
    }
    let vtor_installed = rd(SCB_VTOR) == RAM_VECTOR_BASE;
    let vector_installed = unsafe {
        read_volatile(table.add(EXTI0_1_VECTOR_INDEX))
            == ((smoke_exti0_1_handler as *const () as usize as u32) | 1)
    };

    let msp_before: u32;
    unsafe { asm!("mrs {}, msp", out(reg) msp_before, options(nomem, nostack, preserves_flags)) };
    wr(NVIC_ISER0, irq_bit);
    unsafe { pend_exti0_1_irq_with_sentinels() };
    let msp_after: u32;
    unsafe { asm!("mrs {}, msp", out(reg) msp_after, options(nomem, nostack, preserves_flags)) };
    for _ in 0..100_000 {
        if rd(IRQ_HIT) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let active_after = rd(NVIC_IABR0) & irq_bit;
    let pending_after = rd(NVIC_ISPR0) & irq_bit;
    wr(NVIC_ICER0, irq_bit);
    unpend(EXTI0_1_IRQ);
    wr(SCB_VTOR, source_vtor);
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let hit = rd(IRQ_HIT) == 1;
    let ipsr = rd(IRQ_IPSR);
    let active_during = rd(IRQ_ACTIVE);
    let pending_during = rd(IRQ_PENDING);
    let frame_r0 = unsafe { read_volatile(IRQ_FRAME as *const u32) };
    let frame_r1 = unsafe { read_volatile((IRQ_FRAME + 4) as *const u32) };
    let frame_r2 = unsafe { read_volatile((IRQ_FRAME + 8) as *const u32) };
    let frame_r3 = unsafe { read_volatile((IRQ_FRAME + 12) as *const u32) };
    let frame_r12 = unsafe { read_volatile((IRQ_FRAME + 16) as *const u32) };
    let frame_lr = unsafe { read_volatile((IRQ_FRAME + 20) as *const u32) };
    let frame_pc = unsafe { read_volatile((IRQ_FRAME + 24) as *const u32) };
    let frame_xpsr = unsafe { read_volatile((IRQ_FRAME + 28) as *const u32) };
    let exc_return = unsafe { read_volatile((IRQ_FRAME + 32) as *const u32) };
    let frame_sp = unsafe { read_volatile((IRQ_FRAME + 36) as *const u32) };
    let exception_msp = rd(IRQ_EXCEPTION_MSP);
    let frame_ok = frame_r0 == 0x11
        && frame_r1 == 0x22
        && frame_r2 == 0x33
        && frame_r3 == 0x44
        && frame_r12 == 0x55
        && frame_lr & 1 != 0
        && (0x0800_0000..0x0802_0000).contains(&frame_pc)
        && frame_xpsr & 0x0100_0000 != 0
        && frame_xpsr & 0x0000_03ff == 0
        && exc_return == 0xffff_fff9
        && frame_sp & 7 == 0
        && exception_msp == msp_before.wrapping_sub(8)
        && frame_sp == exception_msp.wrapping_sub(32)
        && msp_after == msp_before
        && (0x2000_0000..0x2000_4000).contains(&frame_sp);
    let returned = active_after == 0 && pending_after == 0 && vtor_restored;
    let ok = vtor_installed
        && vector_installed
        && hit
        && ipsr == 16 + EXTI0_1_IRQ
        && active_during == irq_bit
        && pending_during == 0
        && frame_ok
        && returned;

    puts("irq exception hit=");
    puts(if hit { "1" } else { "0" });
    puts(" ipsr=");
    hex(ipsr);
    puts(" active=");
    hex(active_during);
    puts(" pending=");
    hex(pending_during);
    puts(" frame=");
    puts(if frame_ok { "1" } else { "0" });
    puts(" regs=");
    hex(frame_r0);
    puts(",");
    hex(frame_r1);
    puts(",");
    hex(frame_r2);
    puts(",");
    hex(frame_r3);
    puts(",");
    hex(frame_r12);
    puts(" lr=");
    hex(frame_lr);
    puts(" pc=");
    hex(frame_pc);
    puts(" xpsr=");
    hex(frame_xpsr);
    puts(" exc_return=");
    hex(exc_return);
    puts(" sp=");
    hex(frame_sp);
    puts(" expected_sp=");
    hex(exception_msp.wrapping_sub(32));
    puts(" restored_msp=");
    hex(msp_after);
    puts(" returned=");
    puts(if returned { "1 " } else { "0 " });
    verdict(ok);
    ok
}

/// ARMv6-M reserves AIRCR.PRIGROUP. Even writes with VECTKEY must read back
/// zero, leaving all implemented priority bits available for preemption.
fn check_aircr_priority_group() -> bool {
    wr(SCB_AIRCR, AIRCR_VECTKEY | (5 << 8));
    let group_five = rd(SCB_AIRCR) & 0x700;
    wr(SCB_AIRCR, AIRCR_VECTKEY | (7 << 8));
    let group_seven = rd(SCB_AIRCR) & 0x700;
    let ok = group_five == 0 && group_seven == 0;
    puts("aircr prigroup=0 reserved=");
    flag("writes_ignored", ok);
    verdict(ok);
    ok
}

/// 8 MHz HSI / 115200 = 69 = 0x45.
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

/// PA9/PA10 as AF1 (USART1 on the ST-LINK VCP pins).
fn console_pins() {
    let moder = rd(GPIOA + G_MODER) & !(0b1111 << 18);
    wr(GPIOA + G_MODER, moder | (0b10 << 18) | (0b10 << 20));
    let afrh = rd(GPIOA + G_AFRH) & !(0xFF << 4);
    wr(GPIOA + G_AFRH, afrh | (0x11 << 4));
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

/// Start-injected USART2 bytes must stay on the wire until UE and RE are set.
#[cfg(feature = "onboarding-rx-gate")]
fn check_usart2_rx_enable_gates() -> bool {
    // uart_init leaves UE|TE set and RE clear.
    let re_off = rd(USART2 + U_ISR) & USART_ISR_RXNE == 0;

    // RE alone must not expose the queued bytes while UE is clear.
    wr(USART2 + U_CR1, (1 << 3) | (1 << 2)); // TE | RE
    let ue_off = rd(USART2 + U_CR1) & 1 == 0 && rd(USART2 + U_ISR) & USART_ISR_RXNE == 0;

    // Enabling both gates releases the injected bytes in order.
    wr(USART2 + U_CR1, 1 | (1 << 3) | (1 << 2)); // UE | TE | RE
    let mut received = [0u8; 2];
    let mut count = 0;
    for _ in 0..1_000_000 {
        if count < received.len() && rd(USART2 + U_ISR) & USART_ISR_RXNE != 0 {
            received[count] = rd(USART2 + U_RDR) as u8;
            count += 1;
        }
        if count == received.len() {
            break;
        }
        core::hint::spin_loop();
    }
    let released = rd(USART2 + U_ISR) & USART_ISR_RXNE == 0;
    let data = (u32::from(received[0]) << 8) | u32::from(received[1]);
    let ok = re_off && ue_off && count == 2 && received == *b"RS" && released;

    puts("usart2 rx_gate re_off=");
    puts(if re_off { "1" } else { "0" });
    puts(" ue_off=");
    puts(if ue_off { "1" } else { "0" });
    puts(" data=");
    hex(data);
    puts(" released=");
    puts(if released { "1" } else { "0" });
    puts(if ok { " ok\r\n" } else { " FAIL\r\n" });
    ok
}

/// LD3..LD6 lit one by one through BSRR, each step read back from ODR (and
/// the pad from IDR); then LD3 blinks twice for the gpio_edges assertion and
/// BRR turns them all off.
fn check_leds() -> bool {
    // PC6..PC9 as push-pull outputs.
    let moder = rd(GPIOC + G_MODER) & !(0xFF << 12);
    wr(GPIOC + G_MODER, moder | (0x55 << 12));
    let mut lit = 0u32;
    let mut walk = true;
    for &(name, mask) in &LEDS {
        wr(GPIOC + G_BSRR, mask);
        lit |= mask;
        let odr = rd(GPIOC + G_ODR);
        walk &= odr == lit && rd(GPIOC + G_IDR) & lit == lit;
        puts("led=");
        puts(name);
        puts(" odr=");
        hex(odr);
        puts("\r\n");
    }
    let mut blink = true;
    for _ in 0..2 {
        wr(GPIOC + G_BRR, LED_LD3);
        blink &= rd(GPIOC + G_ODR) & LED_LD3 == 0;
        wr(GPIOC + G_BSRR, LED_LD3);
        blink &= rd(GPIOC + G_ODR) & LED_LD3 != 0;
    }
    wr(GPIOC + G_BRR, lit);
    let off = rd(GPIOC + G_ODR) == 0;
    puts("leds ");
    flag("walk", walk);
    flag("blink", blink);
    flag("off", off);
    let ok = walk && blink && off;
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

/// B1 on PA0 (pressed = high, internal pull-down) through the SYSCFG port
/// mux into EXTI line 0 on the rising edge. With EXTICR1 selecting port C
/// the first press must not pend line 0; with port A the second press must,
/// on NVIC EXTI0_1 = 5 only.
fn check_button() -> bool {
    let pupdr = rd(GPIOA + G_PUPDR) & !0b11;
    wr(GPIOA + G_PUPDR, pupdr | 0b10);
    let moder = rd(GPIOA + G_MODER) & !0b11;
    wr(GPIOA + G_MODER, moder);
    wr(SYSCFG_EXTICR1, CODE_PC);
    wr(EXTI + E_RTSR, 1);
    wr(EXTI + E_IMR, 1);
    unpend(EXTI0_1_IRQ);

    let idle = wait_level(GPIOA, 0, false);
    let press1 = wait_level(GPIOA, 0, true) && wait_level(GPIOA, 0, false);
    let other_port_quiet = rd(EXTI + E_PR) & 1 == 0 && !pending(EXTI0_1_IRQ);

    wr(SYSCFG_EXTICR1, CODE_PA);
    let press2 = wait_level(GPIOA, 0, true);
    let routed = rd(EXTI + E_PR) & 1 != 0 && pending(EXTI0_1_IRQ) && !pending(6) && !pending(7);
    let released = wait_level(GPIOA, 0, false);
    wr(EXTI + E_PR, 1);
    unpend(EXTI0_1_IRQ);
    let cleared = rd(EXTI + E_PR) & 1 == 0 && !pending(EXTI0_1_IRQ);
    wr(EXTI + E_IMR, 0);

    puts("b1 ");
    flag("idle", idle);
    flag("presses", press1 && press2 && released);
    flag("pc_quiet", other_port_quiet);
    flag("pa0", routed);
    flag("cleared", cleared);
    let ok = idle && press1 && press2 && released && other_port_quiet && routed && cleared;
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

/// DMA1 channel 1 memory-to-memory (MEM2MEM, 32-bit, both pointers
/// incrementing): four words copied, TCIF1 set, and the transfer-complete
/// interrupt on DMA1_Channel1_IRQn = 9 only.
fn check_dma_m2m() -> bool {
    let words = [0x0072_0B01u32, 0xD15C_0000, 0x1234_5678, 0xA5A5_5A5A];
    for (i, &w) in words.iter().enumerate() {
        wr(DMA_SRC + 4 * i as u32, w);
        wr(DMA_DST + 4 * i as u32, 0);
    }
    for &line in &DMA_LINES {
        unpend(line);
    }
    wr(DMA1 + D_IFCR, 0x0FFF_FFFF);
    wr(DMA1_CH1 + C_CPAR, DMA_SRC);
    wr(DMA1_CH1 + C_CMAR, DMA_DST);
    wr(DMA1_CH1 + C_CNDTR, words.len() as u32);
    // MEM2MEM | MSIZE=32 | PSIZE=32 | MINC | PINC | TCIE | EN (DIR=0: CPAR is the source).
    wr(
        DMA1_CH1 + C_CCR,
        (1 << 14) | (0b10 << 10) | (0b10 << 8) | (1 << 7) | (1 << 6) | (1 << 1) | 1,
    );
    let mut tc = false;
    for _ in 0..100_000 {
        if rd(DMA1 + D_ISR) & (1 << 1) != 0 {
            tc = true;
            break;
        }
    }
    let data = words
        .iter()
        .enumerate()
        .all(|(i, &w)| rd(DMA_DST + 4 * i as u32) == w)
        && rd(DMA1_CH1 + C_CNDTR) == 0;
    let pend = pending(9);
    let alone = !pending(10) && !pending(11);
    wr(DMA1_CH1 + C_CCR, 0);
    wr(DMA1 + D_IFCR, 1); // CGIF1
    unpend(9);
    let released = rd(DMA1 + D_ISR) & 0xF == 0 && !pending(9);

    puts("dma1 m2m ");
    flag("tc", tc);
    flag("data", data);
    flag("pend", pend);
    flag("alone", alone);
    flag("released", released);
    let ok = tc && data && pend && alone && released;
    verdict(ok);
    ok
}

fn check_crc() -> bool {
    wr(CRC_CR, 1); // RESET
    let seed = rd(CRC_DR) == 0xFFFF_FFFF;
    wr(CRC_DR, 0x1234_5678);
    let word = rd(CRC_DR);
    let std = word == 0xDF8A_8A2B;
    puts("crc word=");
    hex(word);
    puts(" ");
    flag("seed", seed);
    flag("std", std);
    let ok = seed && std;
    verdict(ok);
    ok
}

/// The ADC on its internal channels: calibration, ADRDY, then TS and
/// V_REFINT as one 12-bit sequence with EOS on ADC1_COMP_IRQn = 12, and the
/// internal channels reading 0 again once CCR switches them off.
fn check_adc() -> bool {
    wr(ADC_CR, ADC_CR_ADCAL);
    let cal = wait_clear(ADC_CR, ADC_CR_ADCAL);
    wr(ADC_CCR, ADC_CCR_VREFEN | ADC_CCR_TSEN);
    wr(ADC_CR, ADC_CR_ADEN);
    let ready = wait_set(ADC_ISR, ADC_ISR_ADRDY);
    wr(ADC_ISR, ADC_ISR_ADRDY);
    wr(ADC_SMPR, 7);
    wr(ADC_CHSELR, (1 << 16) | (1 << 17));
    unpend(ADC_LINE);
    wr(ADC_IER, ADC_ISR_EOS);
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADSTART);
    let mut got = [0u32; 2];
    let mut eoc = true;
    for slot in got.iter_mut() {
        eoc &= wait_set(ADC_ISR, ADC_ISR_EOC);
        *slot = rd(ADC_DR);
    }
    let seq = eoc && got == ADC_WANT;
    let eos = wait_set(ADC_ISR, ADC_ISR_EOS);
    let pend = pending(ADC_LINE);
    wr(ADC_ISR, ADC_ISR_EOS);
    wr(ADC_IER, 0);
    unpend(ADC_LINE);
    let released = !pending(ADC_LINE);

    wr(ADC_CCR, 0);
    wr(ADC_CHSELR, 1 << 17);
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADSTART);
    let gated = wait_set(ADC_ISR, ADC_ISR_EOC) && rd(ADC_DR) == 0;
    wr(ADC_ISR, 0xFFF);
    wr(ADC_CR, ADC_CR_ADEN | ADC_CR_ADDIS);
    let off = wait_clear(ADC_CR, ADC_CR_ADEN);

    puts("adc ts=");
    hex(got[0]);
    puts(" vref=");
    hex(got[1]);
    puts(" ");
    flag("cal", cal);
    flag("ready", ready);
    flag("seq", seq);
    flag("eos", eos);
    flag("pend", pend);
    flag("released", released);
    flag("gated", gated);
    flag("off", off);
    let ok = cal && ready && seq && eos && pend && released && gated && off;
    verdict(ok);
    ok
}

fn wait_set(addr: u32, mask: u32) -> bool {
    (0..100_000).any(|_| rd(addr) & mask != 0)
}

fn wait_clear(addr: u32, mask: u32) -> bool {
    (0..100_000).any(|_| rd(addr) & mask == 0)
}

#[entry]
fn main() -> ! {
    let (rcc_reset, rcc_gate) = rcc_bringup();
    console_pins();
    uart_init(USART1);
    uart_init(USART2);
    uart_init(USART3);
    uart_init(USART4);
    puts("STM32F072B-DISCO SMOKE\r\n");
    puts("cpu=cortex-m0 uart=usart1\r\n");

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate));
    mark(17, check_aircr_priority_group());
    mark(1, check_usart_irqs());
    #[cfg(feature = "onboarding-rx-gate")]
    mark(16, check_usart2_rx_enable_gates());
    mark(2, check_leds());
    mark(3, check_button());
    for (i, &(name, base, irq, width)) in TIMERS.iter().enumerate() {
        mark(4 + i as u32, check_timer(name, base, irq, width));
    }
    mark(13, check_dma_m2m());
    mark(14, check_crc());
    mark(15, check_adc());
    mark(18, check_exception_entry());
    mark(19, check_nmi_repend_and_deferred_irq());
    mark(20, check_systick());
    mark(21, check_systick_primask());
    let summary = summary | 0x0DF0_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
