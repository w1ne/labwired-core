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
//! is also checked as reserved on this ARMv6-M core. SysTick checks cover
//! exception 15, COUNTFLAG read-clear behavior, PRIMASK's pending-then-deliver
//! behavior, SCB software pend/set/clear, and re-pending while its handler is
//! active followed by a second exception entry. A separate periodic check
//! confirms hardware reloads generate three more exception entries. A CVR
//! clear during a live countdown must not fire immediately, but the next
//! countdown-to-zero must still pend SysTick. COUNTFLAG's byte-read and
//! read-only behavior is checked separately from its word-read clear. An RVR
//! update during a live countdown must preserve CVR and take effect on the
//! next reload. Writes to RVR bits [31:24] must be ignored.
//! Clearing CVR after a tick is already pending clears COUNTFLAG but must leave
//! the pending exception intact until PRIMASK is cleared.
//! Clearing a hardware-generated pending exception through ICSR must leave
//! COUNTFLAG latched. A word CSR read clears COUNTFLAG independently, and the
//! next timer wrap must latch it again and pend a fresh exception.

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
const SYSTICK_ICSR_RECORD: u32 = SUMMARY + 208;
const SYSTICK_REPEND_RECORD: u32 = SUMMARY + 224;
const SYSTICK_PERIODIC_RECORD: u32 = SUMMARY + 768;
const SYSTICK_FLAG_RECORD: u32 = SUMMARY + 832;
const SYSTICK_CVR_CLEAR_RECORD: u32 = SUMMARY + 864;
const SYSTICK_PENDING_CLEAR_RECORD: u32 = SUMMARY + 896;
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

fn rd8(addr: u32) -> u8 {
    unsafe { read_volatile(addr as *const u8) }
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

/// Capture software-pended SysTick state from the SCB ICSR path.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_icsr_handler() {
    core::arch::naked_asm!(
        "mrs r2, ipsr",
        "ldr r0, =0x200030D0",
        "str r2, [r0, #0]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "movs r2, #1",
        "str r2, [r0, #8]",
        "bx lr",
    );
}

/// Re-pend SysTick once while exception 15 is active, then record the
/// tail-chained entry.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_repend_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x200030E0",
        "ldr r2, [r0, #28]",
        "adds r2, r2, #1",
        "str r2, [r0, #28]",
        "cmp r2, #1",
        "bne 1f",
        "mrs r2, ipsr",
        "str r2, [r0, #0]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "ldr r2, [r1]",
        "str r2, [r0, #12]",
        "ldr r2, [r0, #28]",
        "str r2, [r0, #16]",
        "bx lr",
        "1:",
        "mrs r2, ipsr",
        "str r2, [r0, #20]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #24]",
        "bx lr",
    );
}

/// Record three timer-generated SysTick entries while the counter reloads.
/// Stop the timer in the third handler so the smoke has a bounded result.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_periodic_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20003300",
        "ldr r2, [r0, #0]",
        "adds r2, r2, #1",
        "str r2, [r0, #0]",
        "subs r3, r2, #1",
        "lsls r3, r3, #2",
        "adds r3, r3, #4",
        "mrs r1, ipsr",
        "str r1, [r0, r3]",
        "ldr r1, =0xE000ED04",
        "ldr r1, [r1]",
        "adds r3, r3, #12",
        "str r1, [r0, r3]",
        "ldr r1, =0xE000E010",
        "ldr r1, [r1]",
        "adds r3, r3, #12",
        "str r1, [r0, r3]",
        "cmp r2, #3",
        "bne 1f",
        "ldr r1, =0xE000E010",
        "movs r2, #0",
        "str r2, [r1]",
        "1:",
        "bx lr",
    );
}

/// Record an unexpected SysTick exception in the COUNTFLAG-only test.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_countflag_only_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20003340",
        "movs r1, #1",
        "str r1, [r0]",
        "ldr r0, =0xE000E010",
        "movs r1, #0",
        "str r1, [r0]",
        "bx lr",
    );
}

/// Record a timer-generated SysTick after a software CVR clear, then stop it.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_cvr_clear_handler() {
    core::arch::naked_asm!(
        "mrs r2, ipsr",
        "ldr r0, =0x20003360",
        "str r2, [r0, #0]",
        "ldr r1, =0xE000ED04",
        "ldr r2, [r1]",
        "str r2, [r0, #4]",
        "ldr r2, [r0, #8]",
        "adds r2, #1",
        "str r2, [r0, #8]",
        "ldr r1, =0xE000E010",
        "movs r2, #0",
        "str r2, [r1]",
        "bx lr",
    );
}

/// Capture the hardware-generated SysTick after software clears the first
/// pending request. Stop the timer once the following period enters.
#[unsafe(naked)]
unsafe extern "C" fn smoke_systick_pending_clear_handler() {
    core::arch::naked_asm!(
        "mrs r2, ipsr",
        "ldr r0, =0x20003380",
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

/// ICSR.PENDSTSET must expose a pending bit, PENDSTCLR must cancel it before
/// delivery, and a fresh software pend must enter after PRIMASK is cleared.
fn check_systick_icsr_software_pend() -> bool {
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
        write_volatile(SYSTICK_ICSR_RECORD as *mut u32, 0);
        write_volatile((SYSTICK_ICSR_RECORD + 4) as *mut u32, 0);
        write_volatile((SYSTICK_ICSR_RECORD + 8) as *mut u32, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_icsr_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SCB_ICSR, 1 << 26);
    }

    for _ in 0..1_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let set_readback = rd(SCB_ICSR) & (1 << 26) != 0;
    unsafe { wr(SCB_ICSR, 1 << 25) };
    let clear_readback = rd(SCB_ICSR) & (1 << 26) == 0;
    unsafe { asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags)) };
    for _ in 0..64 {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let canceled_before_delivery = rd(SYSTICK_ICSR_RECORD + 8) == 0;
    let thread_after_clear: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_after_clear,
            options(nomem, nostack, preserves_flags),
        );
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SCB_ICSR, 1 << 26);
    }

    for _ in 0..1_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let pending_while_masked = rd(SCB_ICSR) & (1 << 26) != 0;
    let handler_before_unmask = rd(SYSTICK_ICSR_RECORD + 8) != 0;
    unsafe { asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags)) };
    for _ in 0..100_000 {
        if rd(SYSTICK_ICSR_RECORD + 8) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_ipsr = rd(SYSTICK_ICSR_RECORD);
    let icsr_in_handler = rd(SYSTICK_ICSR_RECORD + 4);
    let handler_hit = rd(SYSTICK_ICSR_RECORD + 8) == 1;
    let pending_after_return = rd(SCB_ICSR) & (1 << 26) == 0;
    let thread_after_return: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_after_return,
            options(nomem, nostack, preserves_flags),
        );
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
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let active_exception = icsr_in_handler & 0x1FF == 15;
    let pending_in_handler = icsr_in_handler & (1 << 26) != 0;
    let ok = prior_primask == 0
        && set_readback
        && clear_readback
        && canceled_before_delivery
        && thread_after_clear == 0
        && pending_while_masked
        && !handler_before_unmask
        && handler_hit
        && handler_ipsr == 15
        && active_exception
        && !pending_in_handler
        && pending_after_return
        && thread_after_return == 0
        && vtor_restored;

    puts("systick icsr set_readback=");
    puts(if set_readback { "1 " } else { "0 " });
    flag("clear", clear_readback);
    flag("canceled", canceled_before_delivery);
    flag("masked", pending_while_masked && !handler_before_unmask);
    puts("ipsr=");
    hex(handler_ipsr);
    puts(" active=");
    hex(icsr_in_handler & 0x1FF);
    puts(" pending_handler=");
    puts(if pending_in_handler { "1" } else { "0" });
    puts(" pending_after=");
    puts(if pending_after_return { "0" } else { "1" });
    puts(" thread_ipsr=");
    hex(thread_after_return);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// A software pend written during SysTick must remain pending while exception
/// 15 is active and enter once more after the active handler returns.
fn check_systick_icsr_repend() -> bool {
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
        for index in 0..8 {
            write_volatile((SYSTICK_REPEND_RECORD as *mut u32).add(index), 0);
        }
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_repend_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SCB_ICSR, 1 << 26);
    }

    for _ in 0..1_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let pending_before_unmask = rd(SCB_ICSR) & (1 << 26) != 0;
    let no_handler_before_unmask = rd(SYSTICK_REPEND_RECORD + 28) == 0;
    unsafe { asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags)) };
    for _ in 0..100_000 {
        if rd(SYSTICK_REPEND_RECORD + 28) >= 2 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let first_ipsr = rd(SYSTICK_REPEND_RECORD);
    let first_icsr = rd(SYSTICK_REPEND_RECORD + 4);
    let icsr_after_repend = rd(SYSTICK_REPEND_RECORD + 12);
    let count_after_repend = rd(SYSTICK_REPEND_RECORD + 16);
    let second_ipsr = rd(SYSTICK_REPEND_RECORD + 20);
    let second_icsr = rd(SYSTICK_REPEND_RECORD + 24);
    let entries = rd(SYSTICK_REPEND_RECORD + 28);
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
    }

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

    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let first_active = first_icsr & 0x1FF == 15;
    let first_not_pending = first_icsr & (1 << 26) == 0;
    let pending_while_active = icsr_after_repend & (1 << 26) != 0;
    let active_after_repend = icsr_after_repend & 0x1FF == 15;
    let second_active = second_icsr & 0x1FF == 15;
    let second_not_pending = second_icsr & (1 << 26) == 0;
    let ok = prior_primask == 0
        && pending_before_unmask
        && no_handler_before_unmask
        && entries == 2
        && first_ipsr == 15
        && first_active
        && first_not_pending
        && pending_while_active
        && active_after_repend
        && count_after_repend == 1
        && second_ipsr == 15
        && second_active
        && second_not_pending
        && thread_ipsr == 0
        && vtor_restored;

    puts("systick icsr repend entries=");
    hex(entries);
    puts(" first_ipsr=");
    hex(first_ipsr);
    puts(" first_active=");
    hex(first_icsr & 0x1FF);
    puts(" pending_after_pend=");
    puts(if pending_while_active { "1" } else { "0" });
    puts(" count_after_pend=");
    hex(count_after_repend);
    puts(" second_ipsr=");
    hex(second_ipsr);
    puts(" second_active=");
    hex(second_icsr & 0x1FF);
    puts(" second_pending=");
    puts(if second_not_pending { "0" } else { "1" });
    puts(" thread_ipsr=");
    hex(thread_ipsr);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// Verify hardware SysTick reloads and generates repeated exception 15 entries.
fn check_systick_periodic_reload() -> bool {
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
        wr(SYSTICK_RVR, 127);
        wr(SYSTICK_CVR, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        for index in 0..10 {
            write_volatile((SYSTICK_PERIODIC_RECORD as *mut u32).add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_periodic_handler as *const () as usize as u32) | 1,
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
        if rd(SYSTICK_PERIODIC_RECORD) >= 3 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let entries = rd(SYSTICK_PERIODIC_RECORD);
    let ipsr = [
        rd(SYSTICK_PERIODIC_RECORD + 4),
        rd(SYSTICK_PERIODIC_RECORD + 8),
        rd(SYSTICK_PERIODIC_RECORD + 12),
    ];
    let icsr = [
        rd(SYSTICK_PERIODIC_RECORD + 16),
        rd(SYSTICK_PERIODIC_RECORD + 20),
        rd(SYSTICK_PERIODIC_RECORD + 24),
    ];
    let csr = [
        rd(SYSTICK_PERIODIC_RECORD + 28),
        rd(SYSTICK_PERIODIC_RECORD + 32),
        rd(SYSTICK_PERIODIC_RECORD + 36),
    ];
    let pending_cleared = rd(SCB_ICSR) & (1 << 26) == 0;
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
    }

    let active_each = icsr.iter().all(|value| value & 0x1FF == 15);
    let countflag_each = csr.iter().all(|value| value & (1 << 16) != 0);
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
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let ok = prior_primask == 0
        && entries == 3
        && ipsr == [15; 3]
        && active_each
        && countflag_each
        && rd(SYSTICK_RVR) == 127
        && pending_cleared
        && thread_ipsr == 0
        && vtor_restored;

    puts("systick periodic entries=");
    hex(entries);
    puts(" reload=");
    hex(rd(SYSTICK_RVR));
    puts(" ipsr=");
    hex(ipsr[0]);
    puts(",");
    hex(ipsr[1]);
    puts(",");
    hex(ipsr[2]);
    puts(" ");
    flag("active", active_each);
    flag("countflag", countflag_each);
    puts("pending=");
    puts(if pending_cleared { "0 " } else { "1 " });
    puts("thread_ipsr=");
    hex(thread_ipsr);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// COUNTFLAG records a zero-hit with TICKINT clear. Byte reads and writes to
/// its read-only CSR bit must preserve it; only a word read clears it.
fn check_systick_countflag_without_tickint() -> bool {
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
        wr(SYSTICK_RVR, 127);
        wr(SYSTICK_CVR, 0);
        write_volatile(SYSTICK_FLAG_RECORD as *mut u32, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_countflag_only_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SYSTICK_CSR, 0x5); // ENABLE | processor clock; TICKINT stays clear.
    }
    let control_before_wait = rd(SYSTICK_CSR) & 0x7;
    unsafe {
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let mut countflag_seen = false;
    for _ in 0..100_000 {
        if rd8(SYSTICK_CSR + 2) & 1 != 0 {
            countflag_seen = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let byte_read_preserved = rd8(SYSTICK_CSR + 2) & 1 != 0;
    // COUNTFLAG is read-only in CSR. Setting it in a word write must neither
    // clear the latched flag nor change the three control bits; disabling the
    // timer here makes the following word-read clear check stable.
    wr(SYSTICK_CSR, (1 << 16) | 0x4);
    let csr_after_write = rd(SYSTICK_CSR);
    let control_readback = csr_after_write & 0x7;
    let countflag_write_ignored = csr_after_write & (1 << 16) != 0;
    let countflag_read_clear = rd(SYSTICK_CSR) & (1 << 16) == 0;

    let icsr_before_cleanup = rd(SCB_ICSR);
    let handler_hit = rd(SYSTICK_FLAG_RECORD) != 0;
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
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
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let pending = icsr_before_cleanup & (1 << 26) != 0;
    let ok = prior_primask == 0
        && control_before_wait == 0x5
        && control_readback == 0x4
        && countflag_seen
        && byte_read_preserved
        && countflag_write_ignored
        && countflag_read_clear
        && !handler_hit
        && !pending
        && thread_ipsr == 0
        && vtor_restored;

    puts("systick countflag-only tickint=0 flag=");
    puts(if countflag_seen { "1 " } else { "0 " });
    flag("byte_preserved", byte_read_preserved);
    flag("write_ignored", countflag_write_ignored);
    puts("control=");
    hex(control_readback);
    puts(" ");
    flag("handler", handler_hit);
    puts("pending=");
    puts(if pending { "1 " } else { "0 " });
    flag("read_clear", countflag_read_clear);
    puts("ipsr=");
    hex(thread_ipsr);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// Clearing CVR during an active countdown must reload silently. Clearing it
/// again after a later wrap must clear COUNTFLAG without canceling the already
/// pending exception, which must enter after PRIMASK is cleared.
fn check_systick_cvr_clear_does_not_fire() -> bool {
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
        wr(SYSTICK_RVR, 127);
        wr(SYSTICK_CVR, 0);
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        for index in 0..3 {
            write_volatile((SYSTICK_CVR_CLEAR_RECORD as *mut u32).add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_cvr_clear_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock
    }

    let mut countdown_started = false;
    for _ in 0..1_000 {
        let count = rd(SYSTICK_CVR);
        if (1..110).contains(&count) {
            countdown_started = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let pending_before_clear = rd(SCB_ICSR) & (1 << 26) != 0;
    unsafe {
        wr(SYSTICK_CVR, 0);
        for _ in 0..16 {
            asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
    let pending_after_clear = rd(SCB_ICSR) & (1 << 26) != 0;
    let mut pending_after_period = false;
    for _ in 0..100_000 {
        if rd(SCB_ICSR) & (1 << 26) != 0 {
            pending_after_period = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    // A byte read observes COUNTFLAG without consuming it. CVR writes clear
    // both CVR and COUNTFLAG, but SCB ICSR's already pending SysTick remains
    // independent and must still dispatch once after PRIMASK is restored.
    let countflag_before_pending_clear = rd8(SYSTICK_CSR + 2) & 1 != 0;
    unsafe { wr(SYSTICK_CVR, 0) };
    let pending_survived_cvr_clear = rd(SCB_ICSR) & (1 << 26) != 0;
    let countflag_cleared_by_cvr = rd(SYSTICK_CSR) & (1 << 16) == 0;
    let handler_before_unmask = rd(SYSTICK_CVR_CLEAR_RECORD + 8);
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
        if rd(SYSTICK_CVR_CLEAR_RECORD + 8) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_ipsr = rd(SYSTICK_CVR_CLEAR_RECORD);
    let handler_icsr = rd(SYSTICK_CVR_CLEAR_RECORD + 4);
    let entries = rd(SYSTICK_CVR_CLEAR_RECORD + 8);
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
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

    let active = handler_icsr & 0x1FF == 15;
    let pending_cleared = handler_icsr & (1 << 26) == 0;
    let vtor_restored = rd(SCB_VTOR) == source_vtor;
    let ok = prior_primask == 0
        && countdown_started
        && !pending_before_clear
        && !pending_after_clear
        && pending_after_period
        && countflag_before_pending_clear
        && pending_survived_cvr_clear
        && countflag_cleared_by_cvr
        && handler_before_unmask == 0
        && handler_ipsr == 15
        && active
        && pending_cleared
        && entries == 1
        && thread_ipsr == 0
        && vtor_restored;

    puts("systick cvr-clear countdown=");
    flag("started", countdown_started);
    puts("pending_after_clear=");
    puts(if pending_after_clear { "1" } else { "0" });
    puts(" pending_after_period=");
    puts(if pending_after_period { "1" } else { "0" });
    puts(" ");
    flag("flag_before_clear", countflag_before_pending_clear);
    flag("pending_survived", pending_survived_cvr_clear);
    flag("flag_cleared", countflag_cleared_by_cvr);
    puts("handler_before_unmask=");
    hex(handler_before_unmask);
    puts(" handler_ipsr=");
    hex(handler_ipsr);
    puts(" active=");
    hex(handler_icsr & 0x1FF);
    puts(" entries=");
    hex(entries);
    puts(" thread_ipsr=");
    hex(thread_ipsr);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// Clearing a hardware-generated pending exception must not clear COUNTFLAG
/// or stop SysTick. The following timer period must pend and enter normally.
fn check_systick_hardware_pending_clear() -> bool {
    const RELOAD: u32 = 255;
    const PENDSTSET: u32 = 1 << 26;
    const PENDSTCLR: u32 = 1 << 25;

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
        wr(SCB_ICSR, PENDSTCLR);
        wr(SYSTICK_RVR, RELOAD);
        wr(SYSTICK_CVR, 0);
        for index in 0..3 {
            write_volatile((SYSTICK_PENDING_CLEAR_RECORD as *mut u32).add(index), 0);
        }
        for index in 0..VECTOR_TABLE_WORDS {
            write_volatile(table.add(index), 0);
        }
        write_volatile(
            table.add(SYSTICK_VECTOR_INDEX),
            (smoke_systick_pending_clear_handler as *const () as usize as u32) | 1,
        );
        wr(SCB_VTOR, RAM_VECTOR_BASE);
        wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock
    }

    let mut first_wrap_pending = false;
    for _ in 0..100_000 {
        if rd(SCB_ICSR) & PENDSTSET != 0 {
            first_wrap_pending = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let countflag_before_clear = rd8(SYSTICK_CSR + 2) & 1 != 0;
    let handler_before_clear = rd(SYSTICK_PENDING_CLEAR_RECORD + 8);
    unsafe { wr(SCB_ICSR, PENDSTCLR) };
    let pending_cleared = rd(SCB_ICSR) & PENDSTSET == 0;
    let countflag_survived_clear = rd8(SYSTICK_CSR + 2) & 1 != 0;
    let timer_enabled_after_clear = rd8(SYSTICK_CSR) & 0x3 == 0x3;
    let handler_after_clear = rd(SYSTICK_PENDING_CLEAR_RECORD + 8);
    let countflag_read_value = rd(SYSTICK_CSR) & (1 << 16) != 0;
    let countflag_read_cleared = rd8(SYSTICK_CSR + 2) & 1 == 0;
    let pending_stays_clear_after_countflag_read = rd(SCB_ICSR) & PENDSTSET == 0;

    let mut second_wrap_pending = false;
    for _ in 0..100_000 {
        if rd(SCB_ICSR) & PENDSTSET != 0 {
            second_wrap_pending = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let countflag_after_second_wrap = rd8(SYSTICK_CSR + 2) & 1 != 0;
    let handler_before_unmask = rd(SYSTICK_PENDING_CLEAR_RECORD + 8);
    unsafe {
        asm!("cpsie i", "dsb", "isb", options(nostack, preserves_flags));
    }
    for _ in 0..100_000 {
        if rd(SYSTICK_PENDING_CLEAR_RECORD + 8) != 0 {
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let handler_ipsr = rd(SYSTICK_PENDING_CLEAR_RECORD);
    let handler_icsr = rd(SYSTICK_PENDING_CLEAR_RECORD + 4);
    let entries = rd(SYSTICK_PENDING_CLEAR_RECORD + 8);
    let pending_after_return = rd(SCB_ICSR) & PENDSTSET != 0;
    let thread_ipsr: u32;
    unsafe {
        asm!(
            "mrs {value}, ipsr",
            value = out(reg) thread_ipsr,
            options(nomem, nostack, preserves_flags),
        );
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, PENDSTCLR);
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
    let active = handler_icsr & 0x1FF == 15;
    let not_pending_in_handler = handler_icsr & PENDSTSET == 0;
    let ok = prior_primask == 0
        && first_wrap_pending
        && countflag_before_clear
        && handler_before_clear == 0
        && pending_cleared
        && countflag_survived_clear
        && timer_enabled_after_clear
        && handler_after_clear == 0
        && countflag_read_value
        && countflag_read_cleared
        && pending_stays_clear_after_countflag_read
        && second_wrap_pending
        && countflag_after_second_wrap
        && handler_before_unmask == 0
        && handler_ipsr == 15
        && active
        && not_pending_in_handler
        && entries == 1
        && !pending_after_return
        && thread_ipsr == 0
        && vtor_restored;

    puts("systick pending-clear hardware ");
    flag("first_wrap", first_wrap_pending);
    flag(
        "flag_survived",
        countflag_before_clear && countflag_survived_clear,
    );
    flag("cleared", pending_cleared);
    flag("timer", timer_enabled_after_clear);
    flag("read_clear", countflag_read_value && countflag_read_cleared);
    flag(
        "pending_stays_clear",
        pending_stays_clear_after_countflag_read,
    );
    flag(
        "second_wrap",
        second_wrap_pending && countflag_after_second_wrap,
    );
    puts("handler_before_unmask=");
    hex(handler_before_unmask);
    puts(" ipsr=");
    hex(handler_ipsr);
    puts(" active=");
    hex(handler_icsr & 0x1FF);
    puts(" entries=");
    hex(entries);
    puts(" pending_after=");
    puts(if pending_after_return { "1" } else { "0" });
    puts(" thread_ipsr=");
    hex(thread_ipsr);
    puts(" ");
    flag("vtor_restored", vtor_restored);
    verdict(ok);
    ok
}

/// Updating RVR must leave the running CVR alone and take effect on the next
/// reload, rather than restarting the current countdown.
fn check_systick_reload_update() -> bool {
    const OLD_RELOAD: u32 = 255;
    const NEW_RELOAD: u32 = 31;
    const PENDSTSET: u32 = 1 << 26;
    const PENDSTCLR: u32 = 1 << 25;

    let prior_primask: u32;
    unsafe {
        asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, PENDSTCLR);
        wr(SYSTICK_RVR, OLD_RELOAD);
        wr(SYSTICK_CVR, 0);
        wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock
    }

    let mut current_before_update = 0;
    for _ in 0..10_000 {
        let current = rd(SYSTICK_CVR);
        if (128..=220).contains(&current) {
            current_before_update = current;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    unsafe { wr(SYSTICK_RVR, NEW_RELOAD) };
    let current_after_update = rd(SYSTICK_CVR);
    let reload_readback = rd(SYSTICK_RVR);
    // MMIO access cycles can decrement CVR between the reads, but changing
    // RVR must not reset the in-progress count to the new, smaller value.
    let current_preserved = current_before_update > NEW_RELOAD
        && current_after_update > NEW_RELOAD
        && current_after_update <= current_before_update;

    let mut first_wrap_pending = false;
    for _ in 0..100_000 {
        if rd(SCB_ICSR) & PENDSTSET != 0 {
            first_wrap_pending = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let first_countflag = rd(SYSTICK_CSR) & (1 << 16) != 0;
    unsafe { wr(SCB_ICSR, PENDSTCLR) };
    let pending_cleared = rd(SCB_ICSR) & PENDSTSET == 0;

    // With NEW_RELOAD=31, the next timer-generated pending edge arrives within
    // 32 clocks. Sixteen bounded polls allow for MMIO and branch cycles; the old
    // value is 255, so a stale reload does not wrap within this window.
    let mut next_reload_used = false;
    for _ in 0..16 {
        if rd(SCB_ICSR) & PENDSTSET != 0 {
            next_reload_used = true;
            break;
        }
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let second_countflag = rd(SYSTICK_CSR) & (1 << 16) != 0;
    let ok = prior_primask == 0
        && current_before_update != 0
        && reload_readback == NEW_RELOAD
        && current_preserved
        && first_wrap_pending
        && first_countflag
        && pending_cleared
        && next_reload_used
        && second_countflag;

    unsafe {
        asm!("cpsid i", options(nomem, nostack, preserves_flags));
        wr(SYSTICK_CSR, 0);
        wr(SCB_ICSR, PENDSTCLR);
        wr(SYSTICK_CVR, 0);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    puts("systick reload-update rvr=");
    hex(reload_readback);
    puts(" ");
    flag("preserved", current_preserved);
    flag("first_wrap", first_wrap_pending && first_countflag);
    flag("pending_clear", pending_cleared);
    flag("next_reload", next_reload_used && second_countflag);
    puts("old=");
    hex(current_before_update);
    puts(" current=");
    hex(current_after_update);
    puts(" ");
    verdict(ok);
    ok
}

/// SYST_RVR implements only RELOAD[23:0]; writes to bits [31:24] are ignored.
fn check_systick_reload_width() -> bool {
    const RELOAD: u32 = 0x00AB_CDEF;
    const RESERVED: u32 = 0xFF00_0000;

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
        wr(SYSTICK_RVR, RESERVED | RELOAD);
    }

    let reload = rd(SYSTICK_RVR);
    let low_24_preserved = reload == RELOAD;
    let reserved_zero = reload & RESERVED == 0;

    unsafe {
        wr(SYSTICK_RVR, 0);
        wr(SYSTICK_CVR, 0);
        wr(SCB_ICSR, 1 << 25);
        asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let ok = low_24_preserved && reserved_zero;
    puts("systick reload-width reload=");
    hex(reload);
    puts(" reserved_zero=");
    puts(if reserved_zero { "1" } else { "0" });
    puts(" ");
    verdict(ok);
    ok
}

/// The STM32F072 reference source is HCLK/8; CLKSOURCE=1 selects HCLK.
/// Sample a long countdown across a fixed instruction window in each mode and
/// check that the external source runs at HCLK/8 without allowing a wrap.
fn check_systick_clock_sources() -> bool {
    const RELOAD: u32 = 10_000;
    const SAMPLE_INSTRUCTIONS: usize = 128;

    fn sample(control: u32, reload: u32, sample_instructions: usize) -> (u32, u32, u32, bool) {
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
            wr(SYSTICK_RVR, reload);
            wr(SYSTICK_CVR, 0);
            wr(SYSTICK_CSR, control);
        }

        let mut before = 0;
        for _ in 0..1_000 {
            let current = rd(SYSTICK_CVR);
            if current != 0 {
                before = current;
                break;
            }
            unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
        }
        let csr = rd(SYSTICK_CSR) & 0x7;
        for _ in 0..sample_instructions {
            unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
        }
        let after = rd(SYSTICK_CVR);

        unsafe {
            wr(SYSTICK_CSR, 0);
            wr(SCB_ICSR, 1 << 25);
            wr(SYSTICK_CVR, 0);
            asm!(
                "msr PRIMASK, {primask}",
                "dsb",
                "isb",
                primask = in(reg) prior_primask,
                options(nostack, preserves_flags),
            );
        }
        let cleaned = rd(SYSTICK_CSR) & 1 == 0 && rd(SCB_ICSR) & (1 << 26) == 0;
        (before, after, csr, cleaned)
    }

    let (external_before, external_after, external_csr, external_cleaned) =
        sample(0x1, RELOAD, SAMPLE_INSTRUCTIONS);
    let (processor_before, processor_after, processor_csr, processor_cleaned) =
        sample(0x5, RELOAD, SAMPLE_INSTRUCTIONS);
    let external_delta = external_before.saturating_sub(external_after);
    let processor_delta = processor_before.saturating_sub(processor_after);
    let external_selected = external_csr & 0x4 == 0;
    let processor_selected = processor_csr & 0x4 != 0;
    // Short samples and MMIO boundaries add count rounding; tolerate that
    // while rejecting a materially different divider from the expected 8:1.
    let ratio_8_to_1 = external_delta > 0
        && processor_delta >= SAMPLE_INSTRUCTIONS as u32
        && processor_delta >= external_delta.saturating_mul(7)
        && processor_delta <= external_delta.saturating_mul(9);
    let ok = external_before > external_after
        && processor_before > processor_after
        && external_selected
        && processor_selected
        && external_cleaned
        && processor_cleaned
        && ratio_8_to_1;

    puts("systick clock-source ext_hclk8_delta=");
    hex(external_delta);
    puts(" cpu_hclk_delta=");
    hex(processor_delta);
    puts(" ext_clksource=");
    puts(if external_selected { "0" } else { "1" });
    puts(" cpu_clksource=");
    puts(if processor_selected { "1" } else { "0" });
    puts(" ratio_8_to_1=");
    puts(if ratio_8_to_1 { "yes" } else { "no" });
    puts(" cleanup=");
    puts(if external_cleaned && processor_cleaned {
        "yes"
    } else {
        "no"
    });
    puts(" ");
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
    let systick_primask_ok = check_systick_primask();
    let systick_icsr_software_pend_ok = check_systick_icsr_software_pend();
    let systick_icsr_repend_ok = check_systick_icsr_repend();
    let systick_periodic_ok = check_systick_periodic_reload();
    let systick_countflag_only_ok = check_systick_countflag_without_tickint();
    let systick_cvr_clear_ok = check_systick_cvr_clear_does_not_fire();
    let systick_hardware_pending_clear_ok = check_systick_hardware_pending_clear();
    let systick_reload_update_ok = check_systick_reload_update();
    let systick_reload_width_ok = check_systick_reload_width();
    let systick_clock_sources_ok = check_systick_clock_sources();
    mark(
        21,
        systick_primask_ok && systick_icsr_software_pend_ok && systick_icsr_repend_ok,
    );
    mark(22, systick_periodic_ok);
    mark(
        23,
        systick_countflag_only_ok
            && systick_cvr_clear_ok
            && systick_hardware_pending_clear_ok
            && systick_reload_update_ok
            && systick_reload_width_ok,
    );
    mark(24, systick_clock_sources_ok);
    let summary = summary | 0x0CF0_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
