#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! Renode-onboarding smoke for the STM32F401CDU6 Black Pill. Bare registers,
//! no .data/.bss (minimal.ld has neither). Each block reads its own
//! peripheral back and reports over USART2:
//!   RCC    AHB1ENR/APB1ENR enable bits read back as written
//!   GPIOC  PC13 (active-low user LED) as push-pull output, toggled via BSRR,
//!          ODR read back after each write
//!   TIM2   PSC/ARR readback, up-counter advances, UIF latches and clears (W1C 0)
//!   I2C1   TMP102 temperature-register pointer write + repeated-start read
//!          (25 °C = 0x1900), plus TXE/RXNE buffer IRQs, master-transmit and
//!          master-receive BTF event IRQs, absent-address NACK and error IRQ
//!          checks
//!   I2C2   a separate TMP102 repeated-start read, ITEVTEN-gated SB/ADDR/BTF event IRQs,
//!          TXE/RXNE isolation under ITEVTEN and TXE/RXNE assertion/release
//!          under ITBUFEN,
//!          and a POS-sequenced two-byte receive with
//!          STOP-before-DR, BTF/RXNE source isolation, and both final DR reads
//!          on the imported second controller instance; absent-address AF
//!          status, event/error-line isolation, ITERREN-gated ER IRQ assertion
//!          and release, and recovery
//!   SPI1   MAX31855 reset frame plus TXEIE/RXNEIE level IRQ assertions
//!   USART1 TXEIE, TCIE and RXNEIE assert and release their NVIC line;
//!   USART2 keeps injected bytes hidden until UE and RE are set, then drains
//!   two bytes in order and gates RXNE IRQ 38
//!   SysTick wraps with PRIMASK set, latches exception 15, exposes COUNTFLAG
//!   once on CSR read, clears its pending request through ICSR, dispatches
//!   repeated timer exceptions through vector 15, and defers a lower-priority
//!   PendSV requested inside the SysTick handler until that handler returns;
//!   the tail-chain reuses MSP/EXC_RETURN and preserves the handler's live R0
//!   a second tail-chain starts in Thread mode on PSP and checks EXC_RETURN,
//!   PSP frame reuse, Handler-mode MSP stability, live R0, and caller-state restore
//!   Cortex-M4F lazy FP context is checked with Thread mode using both MSP and PSP;
//!   an additional nested IRQ uses the FPU while the outer handler has live FP state
//! A summary word goes to RAM at 0x20001000 (one bit per smoke group passed).

use core::ptr::{read_volatile, write_volatile};

const RCC_AHB1ENR: u32 = 0x4002_3830;
const RCC_APB1ENR: u32 = 0x4002_3840;
const RCC_APB2ENR: u32 = 0x4002_3844;

const GPIOA: u32 = 0x4002_0000;
const GPIOC: u32 = 0x4002_0800;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

const USART2: u32 = 0x4000_4400;
const USART_SR: u32 = USART2;
const USART_DR: u32 = USART2 + 0x04;
const USART_BRR: u32 = USART2 + 0x08;
const USART_CR1: u32 = USART2 + 0x0C;
const USART1: u32 = 0x4001_1000;
const USART1_SR: u32 = USART1;
const USART1_DR: u32 = USART1 + 0x04;
const USART1_CR1: u32 = USART1 + 0x0C;

const TIM2: u32 = 0x4000_0000;
const TIM_CR1: u32 = TIM2;
const TIM_SR: u32 = TIM2 + 0x10;
const TIM_EGR: u32 = TIM2 + 0x14;
const TIM_CNT: u32 = TIM2 + 0x24;
const TIM_PSC: u32 = TIM2 + 0x28;
const TIM_ARR: u32 = TIM2 + 0x2C;

const I2C1: u32 = 0x4000_5400;
const I2C_CR1: u32 = I2C1;
const I2C_CR2: u32 = I2C1 + 0x04;
const I2C_OAR1: u32 = I2C1 + 0x08;
const I2C_DR: u32 = I2C1 + 0x10;
const I2C_SR1: u32 = I2C1 + 0x14;
const I2C_SR2: u32 = I2C1 + 0x18;
const I2C_CCR: u32 = I2C1 + 0x1C;
const I2C_TRISE: u32 = I2C1 + 0x20;
const I2C2: u32 = 0x4000_5800;
const I2C2_CR1: u32 = I2C2;
const I2C2_CR2: u32 = I2C2 + 0x04;
const I2C2_DR: u32 = I2C2 + 0x10;
const I2C2_SR1: u32 = I2C2 + 0x14;
const I2C2_SR2: u32 = I2C2 + 0x18;
const I2C2_CCR: u32 = I2C2 + 0x1C;
const I2C2_TRISE: u32 = I2C2 + 0x20;

const SPI1: u32 = 0x4001_3000;
const SPI_CR1: u32 = SPI1;
const SPI_CR2: u32 = SPI1 + 0x04;
const SPI_SR: u32 = SPI1 + 0x08;
const SPI_DR: u32 = SPI1 + 0x0C;
const NVIC_ISPR1: u32 = 0xE000_E204;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ISER1: u32 = 0xE000_E104;
const NVIC_ICER1: u32 = 0xE000_E184;
const NVIC_ICPR0: u32 = 0xE000_E280;
const NVIC_ICPR1: u32 = 0xE000_E284;
const NVIC_IPR: u32 = 0xE000_E400;
const NVIC_IABR0: u32 = 0xE000_E300;
const SPI1_IRQ_BIT: u32 = 1 << (35 - 32);
const USART2_IRQ_BIT: u32 = 1 << (38 - 32);
const USART1_IRQ_BIT: u32 = 1 << (37 - 32);
const I2C1_EV_IRQ_BIT: u32 = 1 << 31;
const I2C1_ER_IRQ_BIT: u32 = 1;
const I2C2_EV_IRQ_BIT: u32 = 1 << 1;
const I2C2_ER_IRQ_BIT: u32 = 1 << 2;
const FPU_IRQ: u32 = 54;
const FPU_IRQ_BIT: u32 = 1 << (FPU_IRQ - 32);
const FPU_VECTOR: u32 = 16 + FPU_IRQ;
const FPU_NESTED_IRQ: u32 = 55;
const FPU_NESTED_IRQ_BIT: u32 = 1 << (FPU_NESTED_IRQ - 32);
const FPU_NESTED_VECTOR: u32 = 16 + FPU_NESTED_IRQ;
const SCB_CPACR: u32 = 0xE000_ED88;
const FPU_FPCCR: u32 = 0xE000_EF34;
const FPU_FPCAR: u32 = 0xE000_EF38;
const FPU_RECORD: u32 = 0x2000_1300;
const FPU_NESTED_RECORD: u32 = 0x2000_1340;
const FPU_ACTIVE_NESTED_RECORD: u32 = 0x2000_1380;
const FPU_ACTIVE_NESTED_IRQ_RECORD: u32 = 0x2000_13C0;
const FPU_PSP_STACK_TOP: u32 = 0x2000_1500;
const FPU_PSP_THREAD_S0: u32 = 0x4000_0000;
const FPU_ACTIVE_THREAD_S0: u32 = 0x4080_0000;
const FPU_ACTIVE_OUTER_S0: u32 = 0x4120_0000;
const FPU_ACTIVE_NESTED_S0: u32 = 0x4140_0000;

const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;
const SYSTICK_COUNTFLAG: u32 = 1 << 16;
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;

const RESULT: u32 = 0x2000_1000;
const SYSTICK_HANDLER_COUNT: u32 = 0x2000_1004;
const SYSTICK_HANDLER_IPSR: u32 = 0x2000_1008;
const PENDSV_HANDLER_COUNT: u32 = 0x2000_100C;
const PENDSV_HANDLER_IPSR: u32 = 0x2000_1010;
const SYSTICK_PENDSV_ARMED: u32 = 0x2000_1014;
const SYSTICK_PENDSV_FLAGS: u32 = 0x2000_1018;
const PENDSV_SYSTICK_COUNT_AT_ENTRY: u32 = 0x2000_101C;
const TAILCHAIN_RECORD: u32 = 0x2000_1020;
const TAILCHAIN_THREAD_R0: u32 = 0x5448_5244;
const TAILCHAIN_HANDLER_R0: u32 = 0x5045_4E44;
const PSP_TAILCHAIN_RECORD: u32 = 0x2000_1100;
const PSP_TAILCHAIN_STACK_TOP: u32 = 0x2000_1200;
const TEST_VECTOR_TABLE: u32 = 0x2000_0080;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(b: u8) {
    while rd(USART_SR) & (1 << 7) == 0 {}
    wr(USART_DR, b as u32);
}

fn send(s: &[u8]) {
    for b in s {
        putc(*b);
    }
}

fn put_hex32(v: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for i in (0..8).rev() {
        putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

fn line(label: &[u8], value: u32, ok: bool) {
    send(label);
    put_hex32(value);
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    // GPIOA + GPIOC on AHB1; TIM2, USART2, I2C1/2 on APB1; USART1 + SPI1 on APB2.
    wr(RCC_AHB1ENR, rd(RCC_AHB1ENR) | (1 << 0) | (1 << 2));
    wr(
        RCC_APB1ENR,
        rd(RCC_APB1ENR) | (1 << 0) | (1 << 17) | (1 << 21) | (1 << 22),
    );
    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 12));
    // HSI = 16 MHz, 115200 baud: USARTDIV 8.6875 -> 0x8B.
    wr(USART_BRR, 0x8B);
    wr(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    send(b"BLACKPILL F401 SMOKE\r\n");

    let mut passed = 0u32;

    let systick_pending_ok = smoke_systick_pending_and_countflag();
    send(if systick_pending_ok {
        b"systick wrap=1 pending=1 countflag=read-clear clear=1 ok\r\n"
    } else {
        b"systick wrap/pending/countflag CHECK FAILED\r\n"
    });
    let systick_dispatch_ok = smoke_systick_periodic_exception();
    send(if systick_dispatch_ok {
        b"systick irq entries>=2 vector=15 ipsr=15 return=1 ok\r\n"
    } else {
        b"systick irq dispatch CHECK FAILED\r\n"
    });
    let systick_priority_flags = smoke_systick_pendsv_priority();
    let systick_priority_ok = systick_priority_flags == 0x7FF;
    line(
        b"systick priority flags=",
        systick_priority_flags,
        systick_priority_ok,
    );
    send(if systick_priority_flags & 0x600 == 0x600 {
        b"pendsv clear pending=1 fresh repend=1 ok\r\n"
    } else {
        b"pendsv clear/repend CHECK FAILED\r\n"
    });
    let systick_tailchain_ok = smoke_systick_pendsv_tailchain();
    let systick_psp_tailchain_ok = smoke_systick_pendsv_psp_tailchain();
    let fpu_lazy_exception_ok = unsafe { smoke_fpu_lazy_exception() };
    let fpu_lazy_nested_exception_ok = unsafe { smoke_fpu_lazy_nested_exception() };
    let fpu_nested_active_exception_ok = unsafe { smoke_fpu_nested_active_context() };
    let fpu_lazy_psp_exception_ok = unsafe { smoke_fpu_lazy_psp_exception() };
    passed |= ((systick_pending_ok && systick_dispatch_ok) as u32) << 12;
    passed |=
        ((systick_priority_ok && systick_tailchain_ok && systick_psp_tailchain_ok) as u32) << 13;
    passed |= ((fpu_lazy_exception_ok
        && fpu_lazy_nested_exception_ok
        && fpu_nested_active_exception_ok) as u32)
        << 14;
    passed |= (fpu_lazy_psp_exception_ok as u32) << 15;

    let ahb = rd(RCC_AHB1ENR) & 0x5;
    let apb = rd(RCC_APB1ENR) & ((1 << 0) | (1 << 17) | (1 << 21) | (1 << 22));
    let rcc_ok = ahb == 0x5 && apb == 0x0062_0001;
    line(b"rcc apb1=", apb, rcc_ok);
    passed |= rcc_ok as u32;

    let gpio_ok = smoke_led();
    passed |= (gpio_ok as u32) << 1;

    let tim_ok = smoke_tim2();
    passed |= (tim_ok as u32) << 2;

    let (
        temperature,
        nack_sr1,
        i2c_buf_irq,
        i2c_buf_released,
        i2c_ev_irq,
        i2c_ev_released,
        i2c_er_irq,
        i2c_irq_released,
        i2c_ok,
    ) = smoke_i2c1();
    send(b"i2c1 tmp102=");
    put_hex32(u32::from(temperature));
    send(b" nack=");
    put_hex32(nack_sr1);
    send(b" buf_irq=");
    send(if i2c_buf_irq { b"1" } else { b"0" });
    send(b" buf_release=");
    send(if i2c_buf_released { b"1" } else { b"0" });
    send(b" ev_irq=");
    send(if i2c_ev_irq { b"1" } else { b"0" });
    send(b" ev_release=");
    send(if i2c_ev_released { b"1" } else { b"0" });
    send(b" er_irq=");
    send(if i2c_er_irq { b"1" } else { b"0" });
    send(b" irq_released=");
    send(if i2c_irq_released { b"1" } else { b"0" });
    send(if i2c_ok { b" ok\r\n" } else { b" FAIL\r\n" });
    passed |= (i2c_ok as u32) << 3;

    let (
        i2c2_temperature,
        i2c2_sb_gate,
        i2c2_addr_event_gate,
        i2c2_txe_isolated,
        i2c2_txe_buf_irq,
        i2c2_txe_buf_released,
        i2c2_btf_event_gate,
        i2c2_rxne_buf_irq,
        i2c2_rxne_buf_released,
    ) = smoke_i2c2();
    let i2c2_ok = i2c2_temperature == 0x1900
        && i2c2_sb_gate
        && i2c2_addr_event_gate
        && i2c2_txe_isolated
        && i2c2_txe_buf_irq
        && i2c2_txe_buf_released
        && i2c2_btf_event_gate
        && i2c2_rxne_buf_irq
        && i2c2_rxne_buf_released;
    send(b"i2c2 tmp102=");
    put_hex32(u32::from(i2c2_temperature));
    send(b" sb_irq_gate=");
    send(if i2c2_sb_gate { b"1" } else { b"0" });
    send(b" addr_irq_gate=");
    send(if i2c2_addr_event_gate { b"1" } else { b"0" });
    send(b" txe_isolated=");
    send(if i2c2_txe_isolated { b"1" } else { b"0" });
    send(b" txe_buf_irq=");
    send(if i2c2_txe_buf_irq { b"1" } else { b"0" });
    send(b" txe_buf_release=");
    send(if i2c2_txe_buf_released { b"1" } else { b"0" });
    send(b" btf_irq_gate=");
    send(if i2c2_btf_event_gate { b"1" } else { b"0" });
    send(b" rxne_buf_irq=");
    send(if i2c2_rxne_buf_irq { b"1" } else { b"0" });
    send(b" rxne_buf_release=");
    send(if i2c2_rxne_buf_released { b"1" } else { b"0" });
    send(if i2c2_ok { b" ok\r\n" } else { b" FAIL\r\n" });
    passed |= (i2c2_ok as u32) << 7;

    let (
        i2c2_rx_btf_data,
        i2c2_pos_armed,
        i2c2_rx_btf_irq,
        i2c2_rxne_isolated,
        i2c2_stop_before_dr,
        i2c2_first_read_keeps_rxne,
        i2c2_rx_btf_released,
        i2c2_final_rxne_cleared,
    ) = smoke_i2c2_receive_btf();
    let i2c2_rx_btf_ok = i2c2_rx_btf_data == 0x1980
        && i2c2_pos_armed
        && i2c2_rx_btf_irq
        && i2c2_rxne_isolated
        && i2c2_stop_before_dr
        && i2c2_first_read_keeps_rxne
        && i2c2_rx_btf_released
        && i2c2_final_rxne_cleared;
    send(b"pos=");
    send(if i2c2_pos_armed { b"1" } else { b"0" });
    send(b" ");
    send(b"i2c2 btf_rx_irq=");
    send(if i2c2_rx_btf_irq { b"1" } else { b"0" });
    send(b" rxne_isolated=");
    send(if i2c2_rxne_isolated { b"1" } else { b"0" });
    send(b" stop_before_dr=");
    send(if i2c2_stop_before_dr { b"1" } else { b"0" });
    send(b" rxne_after_first=");
    send(if i2c2_first_read_keeps_rxne {
        b"1"
    } else {
        b"0"
    });
    send(b" release=");
    send(if i2c2_rx_btf_released { b"1" } else { b"0" });
    send(b" rxne_cleared=");
    send(if i2c2_final_rxne_cleared { b"1" } else { b"0" });
    send(b" data=");
    put_hex32(u32::from(i2c2_rx_btf_data));
    send(if i2c2_rx_btf_ok {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    passed |= (i2c2_rx_btf_ok as u32) << 8;

    let (
        i2c2_nack_sr1,
        i2c2_nack_flags,
        i2c2_nack_event_isolated,
        i2c2_nack_er_isolated,
        i2c2_nack_er_irq,
        i2c2_nack_er_masked_release,
        i2c2_nack_af_cleared,
        i2c2_nack_idle,
        i2c2_nack_er_released,
    ) = smoke_i2c2_nack();
    let i2c2_nack_ok = i2c2_nack_flags
        && i2c2_nack_event_isolated
        && i2c2_nack_er_isolated
        && i2c2_nack_er_irq
        && i2c2_nack_er_masked_release
        && i2c2_nack_af_cleared
        && i2c2_nack_idle
        && i2c2_nack_er_released;
    send(b"i2c2 nack=");
    put_hex32(i2c2_nack_sr1);
    send(b" no_addr=");
    send(if i2c2_nack_flags { b"1" } else { b"0" });
    send(b" ev_isolated=");
    send(if i2c2_nack_event_isolated { b"1" } else { b"0" });
    send(b" er_isolated=");
    send(if i2c2_nack_er_isolated { b"1" } else { b"0" });
    send(b" er_irq=");
    send(if i2c2_nack_er_irq { b"1" } else { b"0" });
    send(b" er_masked_release=");
    send(if i2c2_nack_er_masked_release {
        b"1"
    } else {
        b"0"
    });
    send(b" af_cleared=");
    send(if i2c2_nack_af_cleared { b"1" } else { b"0" });
    send(b" idle=");
    send(if i2c2_nack_idle { b"1" } else { b"0" });
    send(b" er_released=");
    send(if i2c2_nack_er_released { b"1" } else { b"0" });
    send(if i2c2_nack_ok {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    passed |= (i2c2_nack_ok as u32) << 9;

    let spi_ok = smoke_spi1();
    passed |= (spi_ok as u32) << 4;

    let usart1_irq_ok = smoke_usart1_txe_irq();
    passed |= (usart1_irq_ok as u32) << 5;
    let usart1_tc_irq_ok = smoke_usart1_tcie_irq();
    passed |= (usart1_tc_irq_ok as u32) << 10;
    let usart1_rx_irq_ok = smoke_usart1_rxne_irq();
    passed |= (usart1_rx_irq_ok as u32) << 6;
    let usart2_rx_gate_ok = smoke_usart2_rx_gate();
    passed |= (usart2_rx_gate_ok as u32) << 11;

    wr(RESULT, 0xB1AC_0000 | passed);
    send(if passed == 0xFFFF {
        b"BLACKPILL PASS\r\n"
    } else {
        b"BLACKPILL FAIL\r\n"
    });

    loop {}
}

/// Exercise the F401's Cortex-M4F floating-point context integration. Thread
/// mode sets S0, then a software-pended IRQ takes an extended lazy frame. The
/// handler's first FP instruction forces the save and deliberately clobbers
/// S0; exception return must restore the interrupted value.
unsafe fn smoke_fpu_lazy_exception() -> bool {
    const CPACR_FULL_ACCESS: u32 = 0x00F0_0000;
    const FPCCR_ASPEN_LSPEN: u32 = 0xC000_0000;
    const FPCCR_LSPACT: u32 = 1;
    const FPCA: u32 = 1 << 2;
    const THREAD_S0: u32 = 0x3F80_0000;

    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_cpacr = rd(SCB_CPACR);
    let prior_fpccr = rd(FPU_FPCCR);
    let prior_fpcar = rd(FPU_FPCAR);
    let prior_vtor = rd(SCB_VTOR);
    let vector_addr = TEST_VECTOR_TABLE + FPU_VECTOR * 4;
    let prior_vector = rd(vector_addr);
    let prior_irq_priority = unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_IRQ) as *const u8) };
    let prior_irq_enabled = rd(NVIC_ISER1) & FPU_IRQ_BIT != 0;
    let prior_irq_pending = rd(NVIC_ISPR1) & FPU_IRQ_BIT != 0;
    let clean_irq_state = rd(NVIC_IABR0 + 4) & FPU_IRQ_BIT == 0 && !prior_irq_pending;

    wr(NVIC_ICER1, FPU_IRQ_BIT);
    wr(NVIC_ICPR1, FPU_IRQ_BIT);
    wr(SCB_CPACR, prior_cpacr | CPACR_FULL_ACCESS);
    wr(FPU_FPCCR, FPCCR_ASPEN_LSPEN);
    wr(FPU_FPCAR, 0);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, 0x80);
    }
    wr(
        vector_addr,
        (fpu_irq_handler as *const () as usize as u32) | 1,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(NVIC_ISER1, FPU_IRQ_BIT);
    for word in 0..6 {
        wr(FPU_RECORD + word * 4, 0);
    }
    unsafe {
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags),);
        core::arch::asm!(
            "vmov s0, {thread_s0}",
            thread_s0 = in(reg) THREAD_S0,
            out("s0") _,
            options(nostack),
        );
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        core::ptr::write_volatile(NVIC_ISPR1 as *mut u32, FPU_IRQ_BIT);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let frame_sp = rd(FPU_RECORD);
    let exc_return = rd(FPU_RECORD + 4);
    let fpccr_entry = rd(FPU_RECORD + 8);
    let fpcar_entry = rd(FPU_RECORD + 12);
    let fpccr_after_touch = rd(FPU_RECORD + 16);
    let saved_s0 = rd(FPU_RECORD + 20);
    let fpcar_after = rd(FPU_FPCAR);
    let pending_after = rd(NVIC_ISPR1) & FPU_IRQ_BIT;
    let active_after = rd(NVIC_IABR0 + 4) & FPU_IRQ_BIT;
    let restored_s0: u32;
    let restored_control: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) restored_s0,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "mrs {control}, CONTROL",
            control = out(reg) restored_control,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }

    let lazy_frame = fpccr_entry & FPCCR_LSPACT != 0
        && fpcar_entry == frame_sp.wrapping_add(0x20)
        && fpccr_after_touch & FPCCR_LSPACT == 0
        && fpcar_after == fpcar_entry
        && saved_s0 == THREAD_S0;
    let context_restored = restored_s0 == THREAD_S0 && restored_control & FPCA != 0;
    let passed = prior_primask == 0
        && clean_irq_state
        && exc_return == 0xFFFF_FFE9
        && lazy_frame
        && context_restored
        && pending_after == 0
        && active_after == 0;

    wr(NVIC_ICER1, FPU_IRQ_BIT);
    wr(NVIC_ICPR1, FPU_IRQ_BIT);
    wr(vector_addr, prior_vector);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, prior_irq_priority);
    }
    wr(SCB_VTOR, prior_vtor);
    if prior_irq_enabled {
        wr(NVIC_ISER1, FPU_IRQ_BIT);
    }
    if prior_irq_pending {
        wr(NVIC_ISPR1, FPU_IRQ_BIT);
    }
    wr(FPU_FPCAR, prior_fpcar);
    wr(FPU_FPCCR, prior_fpccr);
    wr(SCB_CPACR, prior_cpacr);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    send(b"fpu m4f lazy frame=");
    put_hex32(exc_return);
    send(b" lspact=");
    send(if fpccr_entry & FPCCR_LSPACT != 0 {
        b"1->"
    } else {
        b"0->"
    });
    send(if fpccr_after_touch & FPCCR_LSPACT == 0 {
        b"0"
    } else {
        b"1"
    });
    send(b" fpcar=");
    send(
        if fpcar_entry == frame_sp.wrapping_add(0x20) && fpcar_after == fpcar_entry {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" saved=");
    send(if saved_s0 == THREAD_S0 { b"1" } else { b"0" });
    send(b" restored=");
    send(if context_restored { b"1" } else { b"0" });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Check that a nested higher-priority exception leaves the interrupted
/// Thread's lazy FP save pending. The outer handler touches S0 only after the
/// nested basic frame has returned.
unsafe fn smoke_fpu_lazy_nested_exception() -> bool {
    const CPACR_FULL_ACCESS: u32 = 0x00F0_0000;
    const FPCCR_ASPEN_LSPEN: u32 = 0xC000_0000;
    const FPCCR_LSPACT: u32 = 1;
    const FPCA: u32 = 1 << 2;
    const THREAD_S0: u32 = 0x4040_0000;
    const BASIC_FRAME_BYTES: u32 = 0x20;
    const BOTH_IRQS: u32 = FPU_IRQ_BIT | FPU_NESTED_IRQ_BIT;

    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_cpacr = rd(SCB_CPACR);
    let prior_fpccr = rd(FPU_FPCCR);
    let prior_fpcar = rd(FPU_FPCAR);
    let prior_vtor = rd(SCB_VTOR);
    let vector_addr = TEST_VECTOR_TABLE + FPU_VECTOR * 4;
    let nested_vector_addr = TEST_VECTOR_TABLE + FPU_NESTED_VECTOR * 4;
    let prior_vector = rd(vector_addr);
    let prior_nested_vector = rd(nested_vector_addr);
    let prior_irq_priority = unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_IRQ) as *const u8) };
    let prior_nested_priority =
        unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_NESTED_IRQ) as *const u8) };
    let prior_irq_enabled = rd(NVIC_ISER1) & FPU_IRQ_BIT != 0;
    let prior_nested_enabled = rd(NVIC_ISER1) & FPU_NESTED_IRQ_BIT != 0;
    let prior_irq_pending = rd(NVIC_ISPR1) & FPU_IRQ_BIT != 0;
    let prior_nested_pending = rd(NVIC_ISPR1) & FPU_NESTED_IRQ_BIT != 0;
    let clean_irq_state =
        rd(NVIC_IABR0 + 4) & BOTH_IRQS == 0 && !prior_irq_pending && !prior_nested_pending;

    wr(NVIC_ICER1, BOTH_IRQS);
    wr(NVIC_ICPR1, BOTH_IRQS);
    wr(SCB_CPACR, prior_cpacr | CPACR_FULL_ACCESS);
    wr(FPU_FPCCR, FPCCR_ASPEN_LSPEN);
    wr(FPU_FPCAR, 0);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, 0x80);
        core::ptr::write_volatile((NVIC_IPR + FPU_NESTED_IRQ) as *mut u8, 0x40);
    }
    wr(
        vector_addr,
        (fpu_nested_outer_handler as *const () as usize as u32) | 1,
    );
    wr(
        nested_vector_addr,
        (fpu_nested_handler as *const () as usize as u32) | 1,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(NVIC_ISER1, BOTH_IRQS);
    for word in 0..9 {
        wr(FPU_RECORD + word * 4, 0);
    }
    for word in 0..5 {
        wr(FPU_NESTED_RECORD + word * 4, 0);
    }
    unsafe {
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "vmov s0, {thread_s0}",
            thread_s0 = in(reg) THREAD_S0,
            out("s0") _,
            options(nostack),
        );
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        core::ptr::write_volatile(NVIC_ISPR1 as *mut u32, FPU_IRQ_BIT);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let frame_sp = rd(FPU_RECORD);
    let exc_return = rd(FPU_RECORD + 4);
    let fpccr_entry = rd(FPU_RECORD + 8);
    let fpcar_entry = rd(FPU_RECORD + 12);
    let fpccr_after_nested = rd(FPU_RECORD + 16);
    let fpcar_after_nested = rd(FPU_RECORD + 20);
    let fpccr_after_touch = rd(FPU_RECORD + 24);
    let saved_s0 = rd(FPU_RECORD + 28);
    let handler_done = rd(FPU_RECORD + 32) == 1;
    let nested_frame_sp = rd(FPU_NESTED_RECORD);
    let nested_exc_return = rd(FPU_NESTED_RECORD + 4);
    let nested_ipsr = rd(FPU_NESTED_RECORD + 8);
    let nested_fpccr = rd(FPU_NESTED_RECORD + 12);
    let nested_fpcar = rd(FPU_NESTED_RECORD + 16);
    let fpcar_after = rd(FPU_FPCAR);
    let pending_after = rd(NVIC_ISPR1) & BOTH_IRQS;
    let active_after = rd(NVIC_IABR0 + 4) & BOTH_IRQS;
    let restored_s0: u32;
    let restored_control: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) restored_s0,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "mrs {control}, CONTROL",
            control = out(reg) restored_control,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }

    let outer_frame_ok = exc_return == 0xFFFF_FFE9
        && fpccr_entry & FPCCR_LSPACT != 0
        && fpcar_entry == frame_sp.wrapping_add(0x20);
    let nested_frame_ok = nested_exc_return == 0xFFFF_FFF1
        && nested_ipsr == FPU_NESTED_VECTOR
        && nested_frame_sp.wrapping_add(BASIC_FRAME_BYTES) == frame_sp;
    let lazy_state_survived = nested_fpccr & FPCCR_LSPACT != 0
        && nested_fpcar == fpcar_entry
        && fpccr_after_nested & FPCCR_LSPACT != 0
        && fpcar_after_nested == fpcar_entry;
    let lazy_save_completed = fpccr_after_touch & FPCCR_LSPACT == 0
        && fpcar_after == fpcar_entry
        && saved_s0 == THREAD_S0;
    let context_restored = restored_s0 == THREAD_S0 && restored_control & FPCA != 0;
    let passed = prior_primask == 0
        && clean_irq_state
        && handler_done
        && outer_frame_ok
        && frame_sp & 7 == 0
        && nested_frame_ok
        && lazy_state_survived
        && lazy_save_completed
        && context_restored
        && pending_after == 0
        && active_after == 0;

    wr(NVIC_ICER1, BOTH_IRQS);
    wr(NVIC_ICPR1, BOTH_IRQS);
    wr(vector_addr, prior_vector);
    wr(nested_vector_addr, prior_nested_vector);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, prior_irq_priority);
        core::ptr::write_volatile(
            (NVIC_IPR + FPU_NESTED_IRQ) as *mut u8,
            prior_nested_priority,
        );
    }
    wr(SCB_VTOR, prior_vtor);
    if prior_irq_enabled {
        wr(NVIC_ISER1, FPU_IRQ_BIT);
    }
    if prior_nested_enabled {
        wr(NVIC_ISER1, FPU_NESTED_IRQ_BIT);
    }
    if prior_irq_pending || prior_nested_pending {
        wr(
            NVIC_ISPR1,
            (if prior_irq_pending { FPU_IRQ_BIT } else { 0 })
                | (if prior_nested_pending {
                    FPU_NESTED_IRQ_BIT
                } else {
                    0
                }),
        );
    }
    wr(FPU_FPCAR, prior_fpcar);
    wr(FPU_FPCCR, prior_fpccr);
    wr(SCB_CPACR, prior_cpacr);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    send(b"fpu m4f nested lazy lspact=");
    send(if lazy_state_survived { b"1" } else { b"0" });
    send(b" exc_return=");
    put_hex32(nested_exc_return);
    send(b" frame=");
    send(if nested_frame_ok { b"1" } else { b"0" });
    send(b" saved=");
    send(if lazy_save_completed { b"1" } else { b"0" });
    send(b" restored=");
    send(if context_restored { b"1" } else { b"0" });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Verify nested lazy FP preservation when both exception handlers use the FPU.
/// The outer handler saves the interrupted Thread state, then the nested
/// exception lazily saves and restores the outer handler's distinct S0 value.
unsafe fn smoke_fpu_nested_active_context() -> bool {
    const CPACR_FULL_ACCESS: u32 = 0x00F0_0000;
    const FPCCR_ASPEN_LSPEN: u32 = 0xC000_0000;
    const FPCCR_LSPACT: u32 = 1;
    const FPCA: u32 = 1 << 2;
    const EXTENDED_FRAME_BYTES: u32 = 0x68;
    const BOTH_IRQS: u32 = FPU_IRQ_BIT | FPU_NESTED_IRQ_BIT;

    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_cpacr = rd(SCB_CPACR);
    let prior_fpccr = rd(FPU_FPCCR);
    let prior_fpcar = rd(FPU_FPCAR);
    let prior_vtor = rd(SCB_VTOR);
    let vector_addr = TEST_VECTOR_TABLE + FPU_VECTOR * 4;
    let nested_vector_addr = TEST_VECTOR_TABLE + FPU_NESTED_VECTOR * 4;
    let prior_vector = rd(vector_addr);
    let prior_nested_vector = rd(nested_vector_addr);
    let prior_irq_priority = unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_IRQ) as *const u8) };
    let prior_nested_priority =
        unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_NESTED_IRQ) as *const u8) };
    let prior_irq_enabled = rd(NVIC_ISER1) & FPU_IRQ_BIT != 0;
    let prior_nested_enabled = rd(NVIC_ISER1) & FPU_NESTED_IRQ_BIT != 0;
    let prior_irq_pending = rd(NVIC_ISPR1) & FPU_IRQ_BIT != 0;
    let prior_nested_pending = rd(NVIC_ISPR1) & FPU_NESTED_IRQ_BIT != 0;
    let clean_irq_state =
        rd(NVIC_IABR0 + 4) & BOTH_IRQS == 0 && !prior_irq_pending && !prior_nested_pending;

    wr(NVIC_ICER1, BOTH_IRQS);
    wr(NVIC_ICPR1, BOTH_IRQS);
    wr(SCB_CPACR, prior_cpacr | CPACR_FULL_ACCESS);
    wr(FPU_FPCCR, FPCCR_ASPEN_LSPEN);
    wr(FPU_FPCAR, 0);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, 0x80);
        core::ptr::write_volatile((NVIC_IPR + FPU_NESTED_IRQ) as *mut u8, 0x40);
    }
    wr(
        vector_addr,
        (fpu_nested_active_outer_handler as *const () as usize as u32) | 1,
    );
    wr(
        nested_vector_addr,
        (fpu_nested_active_handler as *const () as usize as u32) | 1,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(NVIC_ISER1, BOTH_IRQS);
    for word in 0..11 {
        wr(FPU_ACTIVE_NESTED_RECORD + word * 4, 0);
    }
    for word in 0..10 {
        wr(FPU_ACTIVE_NESTED_IRQ_RECORD + word * 4, 0);
    }
    unsafe {
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "vmov s0, {thread_s0}",
            thread_s0 = in(reg) FPU_ACTIVE_THREAD_S0,
            out("s0") _,
            options(nostack),
        );
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
        core::ptr::write_volatile(NVIC_ISPR1 as *mut u32, FPU_IRQ_BIT);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let outer_frame_sp = rd(FPU_ACTIVE_NESTED_RECORD);
    let outer_exc_return = rd(FPU_ACTIVE_NESTED_RECORD + 4);
    let outer_fpccr_entry = rd(FPU_ACTIVE_NESTED_RECORD + 8);
    let outer_fpcar_entry = rd(FPU_ACTIVE_NESTED_RECORD + 12);
    let outer_fpccr_after_touch = rd(FPU_ACTIVE_NESTED_RECORD + 16);
    let saved_thread_s0 = rd(FPU_ACTIVE_NESTED_RECORD + 20);
    let outer_fpcar_after_touch = rd(FPU_ACTIVE_NESTED_RECORD + 24);
    let outer_control_before_nested = rd(FPU_ACTIVE_NESTED_RECORD + 28);
    let outer_s0_after_nested = rd(FPU_ACTIVE_NESTED_RECORD + 32);
    let outer_control_after_nested = rd(FPU_ACTIVE_NESTED_RECORD + 36);
    let outer_done = rd(FPU_ACTIVE_NESTED_RECORD + 40) == 1;

    let nested_frame_sp = rd(FPU_ACTIVE_NESTED_IRQ_RECORD);
    let nested_exc_return = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 4);
    let nested_ipsr = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 8);
    let nested_fpccr_entry = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 12);
    let nested_fpcar_entry = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 16);
    let nested_fpccr_after_touch = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 20);
    let nested_fpcar_after_touch = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 24);
    let saved_outer_s0 = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 28);
    let nested_live_s0 = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 32);
    let nested_done = rd(FPU_ACTIVE_NESTED_IRQ_RECORD + 36) == 1;
    let fpcar_after = rd(FPU_FPCAR);
    let pending_after = rd(NVIC_ISPR1) & BOTH_IRQS;
    let active_after = rd(NVIC_IABR0 + 4) & BOTH_IRQS;
    let restored_thread_s0: u32;
    let restored_control: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) restored_thread_s0,
            options(nomem, nostack),
        );
        core::arch::asm!(
            "mrs {control}, CONTROL",
            control = out(reg) restored_control,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }

    let outer_lazy_frame = outer_exc_return == 0xFFFF_FFE9
        && outer_fpccr_entry & FPCCR_LSPACT != 0
        && outer_fpcar_entry == outer_frame_sp.wrapping_add(0x20)
        && outer_fpccr_after_touch & FPCCR_LSPACT == 0
        && outer_fpcar_after_touch == outer_fpcar_entry
        && outer_frame_sp & 7 == 0;
    let thread_saved = saved_thread_s0 == FPU_ACTIVE_THREAD_S0;
    let outer_handler_active = outer_control_before_nested & FPCA != 0;
    let nested_lazy_frame = nested_exc_return == 0xFFFF_FFE1
        && nested_ipsr == FPU_NESTED_VECTOR
        && nested_frame_sp.wrapping_add(EXTENDED_FRAME_BYTES) == outer_frame_sp
        && nested_frame_sp & 7 == 0
        && nested_fpccr_entry & FPCCR_LSPACT != 0
        && nested_fpcar_entry == nested_frame_sp.wrapping_add(0x20);
    let outer_saved = saved_outer_s0 == FPU_ACTIVE_OUTER_S0;
    let nested_fp_active = nested_fpccr_after_touch & FPCCR_LSPACT == 0
        && nested_fpcar_after_touch == nested_fpcar_entry
        && nested_live_s0 == FPU_ACTIVE_NESTED_S0;
    let handler_restored =
        outer_s0_after_nested == FPU_ACTIVE_OUTER_S0 && outer_control_after_nested & FPCA != 0;
    let thread_restored =
        restored_thread_s0 == FPU_ACTIVE_THREAD_S0 && restored_control & FPCA != 0;
    let passed = prior_primask == 0
        && clean_irq_state
        && outer_done
        && nested_done
        && outer_lazy_frame
        && thread_saved
        && outer_handler_active
        && nested_lazy_frame
        && outer_saved
        && nested_fp_active
        && handler_restored
        && thread_restored
        && fpcar_after == nested_fpcar_entry
        && pending_after == 0
        && active_after == 0;

    wr(NVIC_ICER1, BOTH_IRQS);
    wr(NVIC_ICPR1, BOTH_IRQS);
    wr(vector_addr, prior_vector);
    wr(nested_vector_addr, prior_nested_vector);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, prior_irq_priority);
        core::ptr::write_volatile(
            (NVIC_IPR + FPU_NESTED_IRQ) as *mut u8,
            prior_nested_priority,
        );
    }
    wr(SCB_VTOR, prior_vtor);
    if prior_irq_enabled {
        wr(NVIC_ISER1, FPU_IRQ_BIT);
    }
    if prior_nested_enabled {
        wr(NVIC_ISER1, FPU_NESTED_IRQ_BIT);
    }
    if prior_irq_pending || prior_nested_pending {
        wr(
            NVIC_ISPR1,
            (if prior_irq_pending { FPU_IRQ_BIT } else { 0 })
                | (if prior_nested_pending {
                    FPU_NESTED_IRQ_BIT
                } else {
                    0
                }),
        );
    }
    wr(FPU_FPCAR, prior_fpcar);
    wr(FPU_FPCCR, prior_fpccr);
    wr(SCB_CPACR, prior_cpacr);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    send(b"fpu m4f nested active exc_return=");
    put_hex32(nested_exc_return);
    send(b" frames=");
    send(if outer_lazy_frame && nested_lazy_frame {
        b"1"
    } else {
        b"0"
    });
    send(b" saved=");
    send(if thread_saved && outer_saved && nested_fp_active {
        b"1"
    } else {
        b"0"
    });
    send(b" restored=");
    send(if handler_restored && thread_restored {
        b"1"
    } else {
        b"0"
    });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Repeat the Cortex-M4F lazy-save check with Thread mode using the PSP. This
/// distinguishes the interrupted process stack from the Handler-mode MSP and
/// verifies that exception return restores the PSP thread's S0 value.
unsafe fn smoke_fpu_lazy_psp_exception() -> bool {
    const CPACR_FULL_ACCESS: u32 = 0x00F0_0000;
    const FPCCR_ASPEN_LSPEN: u32 = 0xC000_0000;
    const FPCCR_LSPACT: u32 = 1;
    const FPCA: u32 = 1 << 2;
    const SPSEL: u32 = 1 << 1;
    const EXTENDED_FRAME_BYTES: u32 = 0x68;

    let prior_primask: u32;
    let prior_control: u32;
    let prior_psp: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "mrs {control}, CONTROL",
            "mrs {psp}, PSP",
            "cpsid i",
            primask = out(reg) prior_primask,
            control = out(reg) prior_control,
            psp = out(reg) prior_psp,
            options(nomem, nostack, preserves_flags),
        );
    }
    let prior_cpacr = rd(SCB_CPACR);
    let prior_fpccr = rd(FPU_FPCCR);
    let prior_fpcar = rd(FPU_FPCAR);
    let prior_vtor = rd(SCB_VTOR);
    let vector_addr = TEST_VECTOR_TABLE + FPU_VECTOR * 4;
    let prior_vector = rd(vector_addr);
    let prior_irq_priority = unsafe { core::ptr::read_volatile((NVIC_IPR + FPU_IRQ) as *const u8) };
    let prior_irq_enabled = rd(NVIC_ISER1) & FPU_IRQ_BIT != 0;
    let prior_irq_pending = rd(NVIC_ISPR1) & FPU_IRQ_BIT != 0;
    let clean_irq_state = rd(NVIC_IABR0 + 4) & FPU_IRQ_BIT == 0 && !prior_irq_pending;

    wr(NVIC_ICER1, FPU_IRQ_BIT);
    wr(NVIC_ICPR1, FPU_IRQ_BIT);
    wr(SCB_CPACR, prior_cpacr | CPACR_FULL_ACCESS);
    wr(FPU_FPCCR, FPCCR_ASPEN_LSPEN);
    wr(FPU_FPCAR, 0);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, 0x80);
    }
    wr(
        vector_addr,
        (fpu_irq_handler as *const () as usize as u32) | 1,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(NVIC_ISER1, FPU_IRQ_BIT);
    for word in 0..8 {
        wr(FPU_RECORD + word * 4, 0);
    }
    let prior_s0: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) prior_s0,
            options(nomem, nostack),
        );
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    let restored_s0 = unsafe { fpu_irq_from_psp() };
    let frame_sp = rd(FPU_RECORD);
    let exc_return = rd(FPU_RECORD + 4);
    let fpccr_entry = rd(FPU_RECORD + 8);
    let fpcar_entry = rd(FPU_RECORD + 12);
    let fpccr_after_touch = rd(FPU_RECORD + 16);
    let saved_s0 = rd(FPU_RECORD + 20);
    let handler_hit = rd(FPU_RECORD + 24) == 1;
    let thread_psp = rd(FPU_RECORD + 28);
    let fpcar_after = rd(FPU_FPCAR);
    let pending_after = rd(NVIC_ISPR1) & FPU_IRQ_BIT;
    let active_after = rd(NVIC_IABR0 + 4) & FPU_IRQ_BIT;
    let control_after: u32;
    let psp_after: u32;
    unsafe {
        core::arch::asm!(
            "mrs {control}, CONTROL",
            "mrs {psp}, PSP",
            control = out(reg) control_after,
            psp = out(reg) psp_after,
            options(nomem, nostack, preserves_flags),
        );
        core::arch::asm!(
            "vmov s0, {value}",
            value = in(reg) prior_s0,
            out("s0") _,
            options(nostack),
        );
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }

    let lazy_frame = frame_sp == thread_psp.wrapping_sub(EXTENDED_FRAME_BYTES)
        && fpccr_entry & FPCCR_LSPACT != 0
        && fpcar_entry == frame_sp.wrapping_add(0x20)
        && fpccr_after_touch & FPCCR_LSPACT == 0
        && fpcar_after == fpcar_entry
        && saved_s0 == FPU_PSP_THREAD_S0;
    let context_restored = restored_s0 == FPU_PSP_THREAD_S0 && control_after & FPCA != 0;
    let caller_state_restored = psp_after == prior_psp && control_after == prior_control;
    let passed = prior_primask == 0
        && prior_control & SPSEL == 0
        && clean_irq_state
        && handler_hit
        && exc_return == 0xFFFF_FFED
        && lazy_frame
        && context_restored
        && caller_state_restored
        && pending_after == 0
        && active_after == 0;

    wr(NVIC_ICER1, FPU_IRQ_BIT);
    wr(NVIC_ICPR1, FPU_IRQ_BIT);
    wr(vector_addr, prior_vector);
    unsafe {
        core::ptr::write_volatile((NVIC_IPR + FPU_IRQ) as *mut u8, prior_irq_priority);
    }
    wr(SCB_VTOR, prior_vtor);
    if prior_irq_enabled {
        wr(NVIC_ISER1, FPU_IRQ_BIT);
    }
    if prior_irq_pending {
        wr(NVIC_ISPR1, FPU_IRQ_BIT);
    }
    wr(FPU_FPCAR, prior_fpcar);
    wr(FPU_FPCCR, prior_fpccr);
    wr(SCB_CPACR, prior_cpacr);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    send(b"fpu m4f psp lazy frame=");
    put_hex32(exc_return);
    send(b" lspact=");
    send(if fpccr_entry & FPCCR_LSPACT != 0 {
        b"1->"
    } else {
        b"0->"
    });
    send(if fpccr_after_touch & FPCCR_LSPACT == 0 {
        b"0"
    } else {
        b"1"
    });
    send(b" psp_frame=");
    send(
        if frame_sp == thread_psp.wrapping_sub(EXTENDED_FRAME_BYTES) {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" fpcar=");
    send(
        if fpcar_entry == frame_sp.wrapping_add(0x20) && fpcar_after == fpcar_entry {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" saved=");
    send(if saved_s0 == FPU_PSP_THREAD_S0 {
        b"1"
    } else {
        b"0"
    });
    send(b" restored=");
    send(if context_restored && caller_state_restored {
        b"1"
    } else {
        b"0"
    });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Enter Thread mode on PSP, pend the FPU test IRQ, then restore its caller's
/// stack selection before returning the restored S0 value in R0.
#[unsafe(naked)]
unsafe extern "C" fn fpu_irq_from_psp() -> u32 {
    core::arch::naked_asm!(
        "push {{r4-r7, lr}}",
        "sub sp, sp, #4",
        "mrs r4, control",
        "mrs r5, psp",
        "ldr r1, ={stack_top}",
        "msr psp, r1",
        "mov r6, r4",
        "bic r6, r6, #4",
        "orr r6, r6, #2",
        "msr control, r6",
        "isb",
        "bl {body}",
        "msr psp, r5",
        "msr control, r4",
        "isb",
        "add sp, sp, #4",
        "pop {{r4-r7, pc}}",
        stack_top = const FPU_PSP_STACK_TOP,
        body = sym fpu_psp_thread_body,
    );
}

/// Run with SPSEL set. Capture the settled process SP and arm the interrupt in
/// one stack-neutral asm block, so the stacked frame can be checked exactly.
unsafe fn fpu_psp_thread_body() -> u32 {
    unsafe {
        core::arch::asm!(
            "mrs {thread_psp}, psp",
            "ldr {record}, ={record_addr}",
            "str {thread_psp}, [{record}, #28]",
            "vmov s0, {thread_s0}",
            "ldr {ispr}, ={ispr_addr}",
            "ldr {irq_bit}, ={irq_mask}",
            "str {irq_bit}, [{ispr}]",
            "dsb",
            "isb",
            thread_psp = out(reg) _,
            record = out(reg) _,
            thread_s0 = in(reg) FPU_PSP_THREAD_S0,
            ispr = out(reg) _,
            irq_bit = out(reg) _,
            record_addr = const FPU_RECORD,
            ispr_addr = const NVIC_ISPR1,
            irq_mask = const FPU_IRQ_BIT,
            out("s0") _,
            options(nostack),
        );
    }
    while rd(FPU_RECORD + 24) == 0 {
        core::hint::spin_loop();
    }
    let restored_s0: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) restored_s0,
            options(nomem, nostack),
        );
    }
    restored_s0
}

#[unsafe(naked)]
unsafe extern "C" fn fpu_irq_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "tst r2, #4",
        "beq 1f",
        "mrs r0, psp",
        "b 2f",
        "1:",
        "mrs r0, msp",
        "2:",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        "push {{r4-r7}}",
        "mov r4, r0",
        "mov r5, r1",
        "bl {touch}",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r5, #16]",
        "ldr r3, [r4, #32]",
        "str r3, [r5, #20]",
        "pop {{r4-r7}}",
        "ldr r2, ={record}",
        "movs r3, #1",
        "str r3, [r2, #24]",
        "ldr r2, [r2, #4]",
        "mov lr, r2",
        "bx lr",
        record = const FPU_RECORD,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
        touch = sym fpu_irq_touch,
    );
}

/// Outer handler leaves the extended Thread frame lazy while a higher-priority
/// basic IRQ preempts it. Only afterward does the outer handler touch the FPU.
#[unsafe(naked)]
unsafe extern "C" fn fpu_nested_outer_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "mrs r0, msp",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        "ldr r3, ={ispr}",
        "ldr r0, ={nested_irq_bit}",
        "str r0, [r3]",
        "dsb",
        "isb",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #16]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #20]",
        "push {{r4-r7}}",
        "ldr r4, ={record}",
        "ldr r5, [r4, #0]",
        "bl {touch}",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r4, #24]",
        "ldr r3, [r5, #32]",
        "str r3, [r4, #28]",
        "movs r3, #1",
        "str r3, [r4, #32]",
        "pop {{r4-r7}}",
        "ldr r2, ={record}",
        "ldr r2, [r2, #4]",
        "mov lr, r2",
        "bx lr",
        record = const FPU_RECORD,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
        ispr = const NVIC_ISPR1,
        nested_irq_bit = const FPU_NESTED_IRQ_BIT,
        touch = sym fpu_irq_touch,
    );
}

/// Nested handler uses only the integer register set, preserving the outer
/// handler's pending lazy FP save and FPCAR pointer.
#[unsafe(naked)]
unsafe extern "C" fn fpu_nested_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "mrs r0, msp",
        "mrs r3, ipsr",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #16]",
        "bx r2",
        record = const FPU_NESTED_RECORD,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
    );
}

/// Outer handler activates its FP context before it pends the nested IRQ.
#[unsafe(naked)]
unsafe extern "C" fn fpu_nested_active_outer_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "mrs r0, msp",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        // This first FP access completes the Thread's pending lazy save.
        "ldr r0, ={outer_s0}",
        "bl {touch}",
        "ldr r1, ={record}",
        "ldr r0, [r1, #0]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #16]",
        "ldr r3, [r0, #32]",
        "str r3, [r1, #20]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #24]",
        "mrs r3, control",
        "str r3, [r1, #28]",
        "ldr r3, ={ispr}",
        "ldr r0, ={nested_irq_bit}",
        "str r0, [r3]",
        "dsb",
        "isb",
        // Exception return from the nested handler must recover the outer S0.
        "bl {read_s0}",
        "ldr r1, ={record}",
        "str r0, [r1, #32]",
        "mrs r3, control",
        "str r3, [r1, #36]",
        "movs r3, #1",
        "str r3, [r1, #40]",
        "ldr r2, [r1, #4]",
        "mov lr, r2",
        "bx lr",
        record = const FPU_ACTIVE_NESTED_RECORD,
        outer_s0 = const FPU_ACTIVE_OUTER_S0,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
        ispr = const NVIC_ISPR1,
        nested_irq_bit = const FPU_NESTED_IRQ_BIT,
        touch = sym fpu_active_fp_set,
        read_s0 = sym fpu_active_fp_get,
    );
}

/// Nested handler's first FP access lazily saves the outer handler's S0.
#[unsafe(naked)]
unsafe extern "C" fn fpu_nested_active_handler() {
    core::arch::naked_asm!(
        "mov r2, lr",
        "mrs r0, msp",
        "mrs r3, ipsr",
        "ldr r1, ={record}",
        "str r0, [r1, #0]",
        "str r2, [r1, #4]",
        "str r3, [r1, #8]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #12]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #16]",
        // Complete the nested exception's lazy save, then overwrite S0.
        "ldr r0, ={nested_s0}",
        "bl {touch}",
        "ldr r1, ={record}",
        "ldr r0, [r1, #0]",
        "ldr r3, ={fpccr}",
        "ldr r3, [r3]",
        "str r3, [r1, #20]",
        "ldr r3, ={fpcar}",
        "ldr r3, [r3]",
        "str r3, [r1, #24]",
        "ldr r0, [r0, #32]",
        "str r0, [r1, #28]",
        "bl {read_s0}",
        "ldr r1, ={record}",
        "str r0, [r1, #32]",
        "movs r3, #1",
        "str r3, [r1, #36]",
        "ldr r2, [r1, #4]",
        "mov lr, r2",
        "bx lr",
        record = const FPU_ACTIVE_NESTED_IRQ_RECORD,
        nested_s0 = const FPU_ACTIVE_NESTED_S0,
        fpccr = const FPU_FPCCR,
        fpcar = const FPU_FPCAR,
        touch = sym fpu_active_fp_set,
        read_s0 = sym fpu_active_fp_get,
    );
}

unsafe fn fpu_active_fp_set(value: u32) {
    unsafe {
        core::arch::asm!(
            "vmov s0, {value}",
            value = in(reg) value,
            out("s0") _,
            options(nostack),
        );
    }
}

unsafe fn fpu_active_fp_get() -> u32 {
    let value: u32;
    unsafe {
        core::arch::asm!(
            "vmov {value}, s0",
            value = out(reg) value,
            options(nomem, nostack),
        );
    }
    value
}

unsafe fn fpu_irq_touch() {
    unsafe {
        core::arch::asm!(
            "movs r0, #0xA5",
            "vmov s0, r0",
            out("r0") _,
            out("s0") _,
            options(nostack),
        );
    }
}

/// Exercise the imported F401 profile's SysTick and SCB pending state. Keep
/// the exception masked so the wrap remains visible until ICSR.PENDSTCLR.
fn smoke_systick_pending_and_countflag() -> bool {
    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    wr(SYSTICK_RVR, 63);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | CLKSOURCE

    let mut wrapped = false;
    for _ in 0..4_096 {
        if rd(SCB_ICSR) & ICSR_PENDSTSET != 0 {
            wrapped = true;
            break;
        }
    }

    wr(SYSTICK_CSR, 0);
    let countflag_reported = rd(SYSTICK_CSR) & SYSTICK_COUNTFLAG != 0;
    let countflag_read_clear = rd(SYSTICK_CSR) & SYSTICK_COUNTFLAG == 0;
    let pending_survives_countflag_read = rd(SCB_ICSR) & ICSR_PENDSTSET != 0;
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    let pending_cleared = rd(SCB_ICSR) & ICSR_PENDSTSET == 0;

    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    prior_primask == 0
        && wrapped
        && countflag_reported
        && countflag_read_clear
        && pending_survives_countflag_read
        && pending_cleared
}

/// Require timer-generated SysTick exceptions to enter vector 15 repeatedly
/// and return to the polling code on the imported Cortex-M4 profile.
fn smoke_systick_periodic_exception() -> bool {
    const RELOAD: u32 = 255;

    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) prior_primask,
            options(nomem, nostack, preserves_flags),
        );
    }

    let prior_vtor = rd(SCB_VTOR);
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    wr(SYSTICK_PENDSV_ARMED, 0);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        SysTick_Handler as *const () as usize as u32,
    );
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    wr(SYSTICK_HANDLER_COUNT, 0);
    wr(SYSTICK_HANDLER_IPSR, 0);
    wr(SYSTICK_RVR, RELOAD);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock.

    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    for _ in 0..4_096 {
        if rd(SYSTICK_HANDLER_COUNT) >= 2 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let masked_primask: u32;
    unsafe {
        core::arch::asm!(
            "mrs {primask}, PRIMASK",
            "cpsid i",
            primask = out(reg) masked_primask,
            options(nomem, nostack, preserves_flags),
        );
    }
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    let entries = rd(SYSTICK_HANDLER_COUNT);
    let ipsr = rd(SYSTICK_HANDLER_IPSR);
    let vector_installed = rd(SCB_VTOR) == TEST_VECTOR_TABLE;

    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..128 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let stable_after_disable = rd(SYSTICK_HANDLER_COUNT) == entries;

    unsafe {
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR);
    wr(SCB_VTOR, prior_vtor);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    prior_primask == 0 && entries >= 2 && ipsr == 15 && vector_installed && stable_after_disable
}

/// PENDSVCLR must cancel a masked request, a fresh request must still dispatch,
/// and a lower-priority PendSV requested by SysTick must wait until exception 15
/// returns before entering its own vector.
fn smoke_systick_pendsv_priority() -> u32 {
    const RELOAD: u32 = 63;
    const PENDSV_EXCEPTION: u32 = 14;

    let prior_primask: u32;
    unsafe {
        core::arch::asm!(
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
    // PendSV is lowest priority; SysTick is highest configurable priority.
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000);
    wr(SYSTICK_HANDLER_COUNT, 0);
    wr(SYSTICK_HANDLER_IPSR, 0);
    wr(PENDSV_HANDLER_COUNT, 0);
    wr(PENDSV_HANDLER_IPSR, 0);
    wr(SYSTICK_PENDSV_FLAGS, 0);
    wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, 0);

    // A software-pended PendSV can be cancelled before exception entry, even
    // after the SCB has exposed it as pending. PRIMASK keeps the probe in thread
    // mode until PENDSVCLR has taken effect; unmasking must not enter the stale
    // request.
    wr(SCB_ICSR, ICSR_PENDSVSET);
    let pending_before_clear = rd(SCB_ICSR) & ICSR_PENDSVSET != 0;
    wr(SCB_ICSR, ICSR_PENDSVCLR);
    let pending_after_clear = rd(SCB_ICSR) & ICSR_PENDSVSET == 0;
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..32 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let clear_cancels_pending = prior_primask == 0
        && pending_before_clear
        && pending_after_clear
        && rd(PENDSV_HANDLER_COUNT) == 0;

    // The clear must not poison a later set. Re-pend while masked, then require
    // one vector-14 entry after interrupts are restored.
    unsafe {
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }
    wr(SCB_ICSR, ICSR_PENDSVCLR);
    wr(SCB_ICSR, ICSR_PENDSVSET);
    let fresh_pending_visible = rd(SCB_ICSR) & ICSR_PENDSVSET != 0;
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..4_096 {
        if rd(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    for _ in 0..32 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let fresh_repend_dispatches_once = fresh_pending_visible
        && rd(PENDSV_HANDLER_COUNT) == 1
        && rd(PENDSV_HANDLER_IPSR) == PENDSV_EXCEPTION;

    unsafe {
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }
    wr(SCB_ICSR, ICSR_PENDSVCLR | ICSR_PENDSTCLR);
    wr(PENDSV_HANDLER_COUNT, 0);
    wr(PENDSV_HANDLER_IPSR, 0);
    wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, 0);

    wr(SYSTICK_PENDSV_ARMED, 1);
    wr(SYSTICK_RVR, RELOAD);
    wr(SYSTICK_CVR, 0);
    wr(SYSTICK_CSR, 0x7); // ENABLE | TICKINT | processor clock.

    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    for _ in 0..4_096 {
        if rd(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }

    let masked_primask: u32;
    unsafe {
        core::arch::asm!(
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
    let pendsv_entries = rd(PENDSV_HANDLER_COUNT);
    let pendsv_ipsr = rd(PENDSV_HANDLER_IPSR);
    let active_probe = rd(SYSTICK_PENDSV_FLAGS);
    let systick_entries_at_pendsv = rd(PENDSV_SYSTICK_COUNT_AT_ENTRY);
    let priority_configured = rd(SCB_SHPR3) & 0xFFFF_0000 == 0x00FF_0000;
    let vector_installed = rd(SCB_VTOR) == TEST_VECTOR_TABLE;

    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) masked_primask,
            options(nostack, preserves_flags),
        );
    }
    for _ in 0..32 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
    let stable_after_disable = rd(PENDSV_HANDLER_COUNT) == pendsv_entries;

    unsafe {
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    }
    wr(SYSTICK_CSR, 0);
    wr(SYSTICK_PENDSV_ARMED, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SCB_VTOR, prior_vtor);
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            "dsb",
            "isb",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    (u32::from(prior_primask == 0))
        | (u32::from(systick_entries == 1) << 1)
        | (u32::from(pendsv_entries == 1) << 2)
        | (u32::from(pendsv_ipsr == PENDSV_EXCEPTION) << 3)
        | (u32::from(active_probe == 0b11) << 4)
        | (u32::from(systick_entries_at_pendsv == 1) << 5)
        | (u32::from(priority_configured) << 6)
        | (u32::from(vector_installed) << 7)
        | (u32::from(stable_after_disable) << 8)
        | (u32::from(clear_cancels_pending) << 9)
        | (u32::from(fresh_repend_dispatches_once) << 10)
}

/// Verify that SysTick tail-chains to lower-priority PendSV without unstacking
/// the Thread frame or losing the live R0 value from the SysTick handler.
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

    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    let prior_rvr = rd(SYSTICK_RVR);
    let prior_csr = rd(SYSTICK_CSR) & 0x7;
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    // SysTick has the highest configurable priority; PendSV is lowest.
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        (tailchain_pendsv_handler as *const () as usize as u32) | 1,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        (tailchain_systick_handler as *const () as usize as u32) | 1,
    );
    for word in 0..12 {
        wr(TAILCHAIN_RECORD + word * 4, 0);
    }
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    unsafe {
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { pend_systick_with_thread_r0() };
    }
    for _ in 0..4_096 {
        if rd(TAILCHAIN_RECORD + 44) != 0 {
            break;
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
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
    }
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(SCB_VTOR, prior_vtor);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SYSTICK_RVR, prior_rvr);
    wr(SYSTICK_CSR, prior_csr);
    unsafe {
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
    let cleanup = rd(SCB_VTOR) == prior_vtor
        && rd(SCB_SHPR3) == prior_shpr3
        && rd(SCB_ICSR) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0;
    let passed = prior_primask == 0
        && deferred
        && frame_reused
        && child_is_pendsv
        && r0_live
        && thread_ipsr == 0
        && cleanup;

    send(b"systick pendsv tailchain frame=");
    putc(if frame_reused { b'1' } else { b'0' });
    send(b" r0=");
    putc(if r0_live { b'1' } else { b'0' });
    send(b" pending=");
    putc(if deferred && child_is_pendsv && cleanup {
        b'1'
    } else {
        b'0'
    });
    send(b" ipsr=");
    put_hex32(child_ipsr);
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Verify SysTick-to-PendSV tail chaining while Thread mode uses the PSP task stack.
fn smoke_systick_pendsv_psp_tailchain() -> bool {
    let prior_primask: u32;
    let source_control: u32;
    let source_psp: u32;
    unsafe {
        core::arch::asm!(
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
    unsafe {
        core::arch::asm!(
            "mrs {}, MSP",
            out(reg) source_msp,
            options(nomem, nostack, preserves_flags),
        );
    }
    let prior_vtor = rd(SCB_VTOR);
    let prior_shpr3 = rd(SCB_SHPR3);
    let prior_rvr = rd(SYSTICK_RVR);
    let prior_csr = rd(SYSTICK_CSR) & 0x7;

    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    // SysTick has higher priority than PendSV; preserve reserved priority bytes.
    wr(SCB_SHPR3, (prior_shpr3 & 0x0000_FFFF) | 0x80C0_0000);
    for slot in 0..16 {
        wr(TEST_VECTOR_TABLE + slot * 4, 0);
    }
    wr(
        TEST_VECTOR_TABLE + 14 * 4,
        (psp_tailchain_pendsv_handler as *const () as usize as u32) | 1,
    );
    wr(
        TEST_VECTOR_TABLE + 15 * 4,
        (psp_tailchain_systick_handler as *const () as usize as u32) | 1,
    );
    for word in 0..17 {
        wr(PSP_TAILCHAIN_RECORD + word * 4, 0);
    }
    wr(SCB_VTOR, TEST_VECTOR_TABLE);
    unsafe { core::arch::asm!("dsb", "isb", options(nostack, preserves_flags)) };
    unsafe {
        core::arch::asm!(
            "msr PRIMASK, {primask}",
            primask = in(reg) prior_primask,
            options(nostack, preserves_flags),
        );
    }

    if prior_primask == 0 {
        unsafe { pend_systick_from_psp() };
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

    unsafe { core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags)) };
    wr(SYSTICK_CSR, 0);
    wr(SCB_ICSR, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    wr(SCB_VTOR, prior_vtor);
    wr(SCB_SHPR3, prior_shpr3);
    wr(SYSTICK_RVR, prior_rvr);
    wr(SYSTICK_CSR, prior_csr);
    unsafe {
        core::arch::asm!(
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
        && parent_icsr_before & ICSR_PENDSVSET == 0
        && parent_icsr_after & 0x1FF == 15
        && parent_icsr_after & ICSR_PENDSVSET != 0;
    let frame_reused = parent_msp == child_msp
        && parent_exc_return == 0xFFFF_FFFD
        && child_exc_return == parent_exc_return
        && parent_psp == child_psp
        && parent_psp == PSP_TAILCHAIN_STACK_TOP.wrapping_sub(32)
        && wrapper_msp == source_msp
        && parent_msp == source_msp.wrapping_sub(16);
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let r0_live = thread_r0 == TAILCHAIN_THREAD_R0
        && child_r0 == TAILCHAIN_HANDLER_R0
        && child_r0 != thread_r0;
    let cleanup = rd(SCB_VTOR) == prior_vtor
        && rd(SCB_SHPR3) == prior_shpr3
        && rd(SCB_ICSR) & (ICSR_PENDSTSET | ICSR_PENDSVSET) == 0
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
    send(b"systick pendsv psp tailchain ");
    send(b"frame=");
    putc(if frame_reused { b'1' } else { b'0' });
    send(b" r0=");
    putc(if r0_live { b'1' } else { b'0' });
    send(b" pending=");
    putc(if deferred && child_is_pendsv && cleanup {
        b'1'
    } else {
        b'0'
    });
    send(b" ipsr=");
    put_hex32(child_ipsr);
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Enter Thread mode on PSP, then software-pend SysTick with a marker in R0.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_from_psp() {
    core::arch::naked_asm!(
        "mrs r3, msp",
        "push {{r4, r5, r6, lr}}",
        "mrs r4, control",
        "mrs r5, psp",
        "ldr r0, =0x20001100",
        "str r3, [r0, #56]",
        "str r5, [r0, #60]",
        "str r4, [r0, #64]",
        "ldr r1, =0x20001200",
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
        "ldr r1, =0x20001100",
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

/// Capture the SysTick PSP frame, pend lower-priority PendSV, and change R0.
#[unsafe(naked)]
unsafe extern "C" fn psp_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20001100",
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

/// Capture PendSV's reused PSP frame before it returns to Thread mode.
#[unsafe(naked)]
unsafe extern "C" fn psp_tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20001100",
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

/// Set a distinct Thread-mode R0 marker and software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_with_thread_r0() {
    core::arch::naked_asm!(
        "ldr r0, =0x54485244",
        "ldr r1, =0xE000ED04",
        "movs r2, #1",
        "lsls r2, r2, #26",
        "str r2, [r1]",
        "bx lr",
    );
}

/// Record the SysTick exception frame, pend PendSV, and leave a new R0 marker.
#[unsafe(naked)]
unsafe extern "C" fn tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20001020",
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
unsafe extern "C" fn tailchain_pendsv_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20001020",
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

/// Cortex-M SysTick exception handler installed at vector-table slot 15.
#[no_mangle]
pub extern "C" fn SysTick_Handler() {
    unsafe {
        let ipsr: u32;
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) ipsr,
            options(nomem, nostack, preserves_flags),
        );
        wr(SYSTICK_HANDLER_IPSR, ipsr);
        wr(
            SYSTICK_HANDLER_COUNT,
            rd(SYSTICK_HANDLER_COUNT).wrapping_add(1),
        );
        if rd(SYSTICK_PENDSV_ARMED) != 0 && rd(SYSTICK_HANDLER_COUNT) == 1 {
            // Stop further timer wraps, request the lower-priority PendSV, and
            // record that it stayed pending without preempting this handler.
            wr(SYSTICK_CSR, 0);
            wr(SCB_ICSR, ICSR_PENDSVSET);
            let pending = (rd(SCB_ICSR) & ICSR_PENDSVSET != 0) as u32;
            let not_preempted = (rd(PENDSV_HANDLER_COUNT) == 0) as u32;
            wr(SYSTICK_PENDSV_FLAGS, pending | (not_preempted << 1));
        }
    }
}

/// PendSV exception handler installed at vector-table slot 14.
#[no_mangle]
pub extern "C" fn PendSV_Handler() {
    unsafe {
        let ipsr: u32;
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) ipsr,
            options(nomem, nostack, preserves_flags),
        );
        wr(PENDSV_HANDLER_IPSR, ipsr);
        wr(PENDSV_SYSTICK_COUNT_AT_ENTRY, rd(SYSTICK_HANDLER_COUNT));
        wr(
            PENDSV_HANDLER_COUNT,
            rd(PENDSV_HANDLER_COUNT).wrapping_add(1),
        );
    }
}

/// PC13 drives the active-low LED: BSRR reset half lights it.
fn smoke_led() -> bool {
    wr(
        GPIOC + MODER,
        (rd(GPIOC + MODER) & !(0b11 << 26)) | (0b01 << 26),
    );
    let moder_ok = (rd(GPIOC + MODER) >> 26) & 0b11 == 0b01;
    let mut ok = moder_ok;
    for _ in 0..3 {
        wr(GPIOC + BSRR, 1 << (13 + 16)); // LED on (pin low)
        let on = rd(GPIOC + ODR) & (1 << 13);
        wr(GPIOC + BSRR, 1 << 13); // LED off (pin high)
        let off = rd(GPIOC + ODR) & (1 << 13);
        ok &= on == 0 && off == 1 << 13;
    }
    line(b"led pc13 odr=", rd(GPIOC + ODR), ok);
    ok
}

fn smoke_tim2() -> bool {
    wr(TIM_PSC, 3);
    wr(TIM_ARR, 0x40);
    wr(TIM_EGR, 1); // UG: load PSC
    wr(TIM_SR, 0);
    let regs_ok = rd(TIM_PSC) == 3 && rd(TIM_ARR) == 0x40;
    wr(TIM_CR1, 1); // CEN
    let c0 = rd(TIM_CNT);
    let mut c1 = c0;
    for _ in 0..64 {
        c1 = rd(TIM_CNT);
        if c1 != c0 {
            break;
        }
    }
    let mut uif = false;
    for _ in 0..4096 {
        if rd(TIM_SR) & 1 != 0 {
            uif = true;
            break;
        }
    }
    wr(TIM_CR1, 0); // stop first so UIF cannot re-latch before the readback
    wr(TIM_SR, 0);
    let cleared = rd(TIM_SR) & 1 == 0;
    let ok = regs_ok && c1 != c0 && c1 <= 0x40 && uif && cleared;
    line(b"tim2 arr=", rd(TIM_ARR), ok);
    ok
}

/// Read the fixture TMP102 through the legacy STM32 I2C controller, then probe
/// an absent address. The exact imported system wires TMP102 at 0x48.
fn smoke_i2c1() -> (u16, u32, bool, bool, bool, bool, bool, bool, bool) {
    wr(I2C_OAR1, 0x4000 | 0xAA); // bit 14 must be kept at 1 by software
    if rd(I2C_OAR1) & 0xFFFF != 0x40AA {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }
    // FREQ = 16 MHz APB1 (HSI). Start with ITEVTEN alone so SB drives the
    // shared EV line; buffer and error enables are checked separately.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    wr(I2C_CCR, 80); // 100 kHz standard mode
    wr(I2C_TRISE, 17);
    wr(I2C_CR1, 1); // PE

    // Write TMP102's temperature-register pointer (0), then use a repeated
    // START to read its two-byte big-endian result. ACK both address phases;
    // after the first byte, clear ACK so the final byte is NACKed.
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 8)); // START
    let sb_irq =
        wait(NVIC_ISPR0, I2C1_EV_IRQ_BIT, I2C1_EV_IRQ_BIT) && rd(I2C_SR1) & 0x0001 == 0x0001;
    send(b"i2c1 sb_irq=");
    putc(if sb_irq { b'1' } else { b'0' });
    send(if sb_irq { b" ok\r\n" } else { b" FAIL\r\n" });
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }
    // SB is cleared by the address write. Switch to ITBUFEN alone first so
    // the next EV assertion can only come from TXE.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 10));
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(I2C_DR, 0x90); // TMP102 7-bit address 0x48, write
    if !wait(I2C_SR1, 0x0002, 0x0002) {
        return (0xFFFF, 0, false, false, false, false, false, false, false);
    }

    // ADDR and TXE are both set. With ITEVTEN clear, TXE alone must assert
    // I2C1_EV through ITBUFEN. Disabling ITBUFEN must release the level even
    // while TXE remains set.
    let buffer_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    wr(I2C_CR2, 16 | (1 << 8)); // ITERREN only; ITBUFEN off, ITEVTEN off
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut buffer_irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        buffer_irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    // Now enable ITEVTEN alone. ADDR must assert the same EV line; the
    // SR1-to-SR2 sequence clears ADDR and releases it while TXE stays set.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    let mut event_irq = false;
    for _ in 0..16 {
        event_irq |= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2); // clear ADDR
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut event_irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        event_irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    // ITEVTEN remains enabled while ITBUFEN is off. TXE alone must not
    // pend EV; BTF is an event source and must assert the same line.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(I2C_DR, 0x00); // temperature register pointer
    if !wait(I2C_SR1, 0x0004, 0x0004) {
        // BTF: pointer byte sent
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let btf_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    send(b"i2c1 btf_irq=");
    putc(if btf_irq { b'1' } else { b'0' });
    send(if btf_irq { b" ok\r\n" } else { b" FAIL\r\n" });

    // For the receiver phase, leave only ITBUFEN set. The subsequent RXNE
    // flag must assert EV by itself, then DR must clear the level source.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 10)); // ITERREN | ITBUFEN; ITEVTEN off
    wr(I2C_CR1, 1 | (1 << 8) | (1 << 10)); // PE | START | ACK
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    wr(I2C_DR, 0x91); // TMP102 address, read
    if !wait(I2C_SR1, 0x0002, 0x0002) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2); // clear ADDR; receiver starts clocking
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    if !wait(I2C_SR1, 0x0040, 0x0040) {
        // RXNE
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let rxne_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;
    let msb = rd(I2C_DR) as u8;
    // RXNE clears on the DR read. Clear the sticky NVIC pending bit and allow
    // a short interval before the next 1440-cycle byte arrives; EV must stay
    // low even though ITBUFEN remains enabled.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut rxne_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        rxne_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }
    wr(I2C_CR1, 1); // PE; NACK the final byte
    wr(I2C_CR1, 1 | (1 << 9)); // request STOP after the final byte
    if !wait(I2C_SR1, 0x0040, 0x0040) {
        return (
            0xFFFF,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            false,
            false,
        );
    }
    let lsb = rd(I2C_DR) as u8;
    let temperature = u16::from_be_bytes([msb, lsb]);
    let idle = wait(I2C_SR2, 0x0002, 0);

    // The system has no device at 0x50. Silicon sets AF, no ADDR, and keeps
    // MSL|BUSY until STOP; AF is rc_w0.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(NVIC_ICPR1, I2C1_ER_IRQ_BIT);
    let mut irq_released = true;
    for _ in 0..16 {
        let _ = rd(I2C_SR1);
        irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
        irq_released &= rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT == 0;
    }
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 8)); // START
    if !wait(I2C_SR1, 0x0001, 0x0001) {
        return (
            temperature,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            irq_released,
            false,
        );
    }
    wr(I2C_DR, 0xA0); // absent EEPROM-style address 0x50, write
    if !wait(I2C_SR1, 0x0400, 0x0400) {
        return (
            temperature,
            0,
            buffer_irq,
            buffer_irq_released,
            event_irq,
            event_irq_released,
            false,
            irq_released,
            false,
        );
    }
    let sr1 = rd(I2C_SR1);
    let sr2 = rd(I2C_SR2);
    let nack_ok = sr1 & 0x0002 == 0 && sr2 & 0x3 == 0x3; // no ADDR; MSL|BUSY
    let er_irq = rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT != 0;
    wr(I2C_CR1, rd(I2C_CR1) | (1 << 9)); // STOP
    wr(I2C_SR1, !0x0400 & 0xFFFF); // clear AF (rc_w0)
    let af_cleared = rd(I2C_SR1) & 0x0400 == 0;
    let nack_idle = wait(I2C_SR2, 0x0002, 0);
    // Clear the sticky NVIC bits only after STOP and AF clear. With the
    // source flags gone, neither I2C line should pend again while polled.
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    wr(NVIC_ICPR1, I2C1_ER_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(I2C_SR1);
        irq_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
        irq_released &= rd(NVIC_ISPR1) & I2C1_ER_IRQ_BIT == 0;
    }
    let rxne_ok = rxne_irq && rxne_released;
    send(b"i2c1 rxne_buf_irq=");
    putc(if rxne_irq { b'1' } else { b'0' });
    send(b" release=");
    putc(if rxne_released { b'1' } else { b'0' });
    send(if rxne_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // Repeat the TMP102 read without consuming the first byte. The second
    // byte then waits behind it in the receive shift register, setting BTF.
    // ITEVTEN alone must report BTF while RXNE alone remains disabled.
    let (rx_btf_data, rx_btf_irq, rxne_isolated, rx_btf_released) = smoke_i2c1_receive_btf();
    // The TMP102 fixture advances by 0.5 °C after each complete temperature
    // read, so the second transaction returns 25.5 °C (0x1980).
    let rx_btf_ok = rx_btf_data == 0x1980 && rx_btf_irq && rxne_isolated && rx_btf_released;
    send(b"i2c1 btf_rx_irq=");
    putc(if rx_btf_irq { b'1' } else { b'0' });
    send(b" rxne_isolated=");
    putc(if rxne_isolated { b'1' } else { b'0' });
    send(b" release=");
    putc(if rx_btf_released { b'1' } else { b'0' });
    send(b" data=");
    put_hex32(u32::from(rx_btf_data));
    send(if rx_btf_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    (
        temperature,
        sr1,
        buffer_irq,
        buffer_irq_released,
        event_irq,
        event_irq_released,
        er_irq,
        irq_released,
        temperature == 0x1900
            && idle
            && nack_ok
            && af_cleared
            && nack_idle
            && buffer_irq
            && buffer_irq_released
            && rxne_ok
            && rx_btf_ok
            && event_irq
            && event_irq_released
            && btf_irq
            && er_irq
            && irq_released,
    )
}

/// Read two TMP102 bytes while leaving the first in DR until the second has
/// arrived. With ITEVTEN enabled and ITBUFEN disabled, RXNE must not assert EV;
/// the full receive pipeline must assert BTF and release it when DR is read.
fn smoke_i2c1_receive_btf() -> (u16, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;

    // ITERREN | ITEVTEN; ITBUFEN stays clear for the receive phase.
    wr(I2C_CR2, 16 | (1 << 8) | (1 << 9));
    wr(I2C_CR1, PE | ACK | START);
    if !wait(I2C_SR1, SB, SB) {
        return (0xFFFF, false, false, false);
    }
    wr(I2C_DR, 0x90);
    if !wait(I2C_SR1, ADDR, ADDR) {
        return (0xFFFF, false, false, false);
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2);
    wr(I2C_DR, 0); // TMP102 temperature register pointer
    if !wait(I2C_SR1, BTF, BTF) {
        return (0xFFFF, false, false, false);
    }

    wr(I2C_CR1, PE | ACK | START); // repeated START
    if !wait(I2C_SR1, SB, SB) {
        return (0xFFFF, false, false, false);
    }
    wr(I2C_DR, 0x91);
    if !wait(I2C_SR1, ADDR, ADDR) {
        return (0xFFFF, false, false, false);
    }
    let _ = rd(I2C_SR1);
    let _ = rd(I2C_SR2);
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);

    // RXNE is a buffer source and must stay out of EV with ITBUFEN disabled.
    if !wait(I2C_SR1, RXNE, RXNE) {
        return (0xFFFF, false, false, false);
    }
    let rxne_isolated = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    wr(I2C_CR1, PE); // NACK the second (final) byte while the first stays in DR
    if !wait(I2C_SR1, BTF, BTF) {
        return (0xFFFF, false, rxne_isolated, false);
    }
    let btf_irq = rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT != 0;

    let msb = rd(I2C_DR) as u8; // second byte moves from shift register into DR
    wr(NVIC_ICPR0, I2C1_EV_IRQ_BIT);
    let mut btf_released = true;
    for _ in 0..16 {
        btf_released &= rd(I2C_SR1) & BTF == 0;
        btf_released &= rd(NVIC_ISPR0) & I2C1_EV_IRQ_BIT == 0;
    }

    wr(I2C_CR1, PE | STOP);
    let idle = wait(I2C_SR2, 0x0002, 0);
    let lsb = rd(I2C_DR) as u8;
    let data = u16::from_be_bytes([msb, lsb]);
    (data, btf_irq, rxne_isolated, btf_released && idle)
}

/// Exercise the imported F401's second legacy I2C controller with its own
/// TMP102 and event vector. This catches instance wiring/configuration mistakes
/// that an I2C1-only transaction cannot see. It checks that SB and ADDR stay
/// off the EV line until ITEVTEN is enabled, isolates TXE from ITEVTEN, then
/// checks TXE and BTF event gating plus TXE/RXNE buffer IRQ behavior under
/// ITBUFEN.
fn smoke_i2c2() -> (u16, bool, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;

    wr(I2C2_CR2, 16); // all I2C interrupt sources disabled
    wr(I2C2_CCR, 80);
    wr(I2C2_TRISE, 17);
    wr(I2C2_CR1, PE);

    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR1, PE | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (
            0xFFFF, false, false, false, false, false, false, false, false,
        );
    }
    let sb_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    // Enabling ITEVTEN while SB is still set must assert the EV line.
    wr(I2C2_CR2, 16 | (1 << 9));
    let sb_event_gate = sb_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR2, 16); // disable ITEVTEN before ADDR is raised
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT); // discard the earlier SB event
    wr(I2C2_DR, 0x92); // independent TMP102 at 0x49, write
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return (
            0xFFFF,
            sb_event_gate,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
    }
    // ADDR must remain isolated with all interrupt enables clear; enabling
    // ITEVTEN while the flag is up must assert the EV line.
    let addr_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 9));
    let addr_event_gate = addr_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2); // clear ADDR; TXE is not an ITEVTEN source
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut txe_isolated = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_isolated &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }

    // The same TXE source must assert the same EV vector once ITBUFEN is
    // enabled alongside ITEVTEN, then release when ITBUFEN is removed.
    wr(I2C2_CR2, 16 | (1 << 9) | (1 << 10));
    let mut txe_buf_irq = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_buf_irq &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT != 0;
    }
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut txe_buf_released = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        txe_buf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }

    // BTF is an event source: keep every interrupt enable clear while the
    // pointer byte completes, then enable ITEVTEN with BTF still set.
    wr(I2C2_CR2, 16);
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_DR, 0); // temperature register pointer
    if !wait(I2C2_SR1, BTF, BTF) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            false,
            false,
            false,
        );
    }
    let btf_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 9)); // ITEVTEN only
    let btf_event_gate = btf_isolated && wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);

    // Repeated START, then read the independent sensor's two-byte result.
    wr(I2C2_CR1, PE | ACK | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    wr(I2C2_DR, 0x93);
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    wr(I2C2_CR2, 16 | (1 << 9)); // ITEVTEN only; RXNE must stay isolated
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            false,
            false,
        );
    }
    let rxne_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    // RXNE remains set; enabling ITBUFEN must assert EV without another byte.
    wr(I2C2_CR2, 16 | (1 << 9) | (1 << 10)); // ITEVTEN | ITBUFEN
    let rxne_irq_asserted = wait(NVIC_ISPR1, I2C2_EV_IRQ_BIT, I2C2_EV_IRQ_BIT);
    let rxne_buf_irq = rxne_isolated && rxne_irq_asserted;
    let msb = rd(I2C2_DR) as u8;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut rxne_buf_released = true;
    for _ in 0..16 {
        rxne_buf_released &= rd(I2C2_SR1) & RXNE == 0;
        rxne_buf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }
    send(b"i2c2 rxne_isolated=");
    send(if rxne_isolated { b"1" } else { b"0" });
    send(b" gate_asserted=");
    send(if rxne_irq_asserted { b"1" } else { b"0" });
    send(b" release=");
    send(if rxne_buf_released { b"1" } else { b"0" });
    send(if rxne_buf_irq && rxne_buf_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    wr(I2C2_CR1, PE); // NACK the final byte
    wr(I2C2_CR1, PE | STOP);
    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            rxne_buf_irq,
            rxne_buf_released,
        );
    }
    let lsb = rd(I2C2_DR) as u8;
    if !wait(I2C2_SR2, 0x0002, 0) {
        return (
            0xFFFF,
            sb_event_gate,
            addr_event_gate,
            txe_isolated,
            txe_buf_irq,
            txe_buf_released,
            btf_event_gate,
            rxne_buf_irq,
            rxne_buf_released,
        );
    }
    (
        u16::from_be_bytes([msb, lsb]),
        sb_event_gate,
        addr_event_gate,
        txe_isolated,
        txe_buf_irq,
        txe_buf_released,
        btf_event_gate,
        rxne_buf_irq,
        rxne_buf_released,
    )
}

/// Read two bytes from the I2C2 TMP102 with POS delaying the ACK decision by
/// one byte. With ITEVTEN enabled and ITBUFEN disabled, RXNE must not assert
/// EV; BTF must assert the configured EV line (IRQ 33). STOP is issued while
/// both bytes remain pending, then both DR reads must preserve the bytes and
/// clear BTF/RXNE in order.
fn smoke_i2c2_receive_btf() -> (u16, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const ACK: u32 = 1 << 10;
    const POS: u32 = 1 << 11;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const BTF: u32 = 1 << 2;
    const RXNE: u32 = 1 << 6;
    const FAILED: (u16, bool, bool, bool, bool, bool, bool, bool) =
        (0xFFFF, false, false, false, false, false, false, false);

    // ITEVTEN only; ITBUFEN remains clear for receive source isolation.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(I2C2_CR1, PE | ACK | START);
    if !wait(I2C2_SR1, SB, SB) {
        return FAILED;
    }
    wr(I2C2_DR, 0x92); // TMP102 at 0x49, write
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return FAILED;
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    wr(I2C2_DR, 0); // temperature register pointer
    if !wait(I2C2_SR1, BTF, BTF) {
        return FAILED;
    }

    // Arm POS while ACK is set so the address phase captures an ACK for the
    // first byte; the write after ADDR clears ACK for the second byte.
    wr(I2C2_CR1, PE | ACK | POS | START);
    if !wait(I2C2_SR1, SB, SB) {
        return FAILED;
    }
    wr(I2C2_DR, 0x93); // TMP102 at 0x49, read
    if !wait(I2C2_SR1, ADDR, ADDR) {
        return FAILED;
    }
    let _ = rd(I2C2_SR1);
    let _ = rd(I2C2_SR2);
    let pos_armed = rd(I2C2_CR1) & POS != 0;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    wr(I2C2_CR1, PE | POS); // POS delays the NACK so the first byte is ACKed.

    if !wait(I2C2_SR1, RXNE, RXNE) {
        return (0xFFFF, pos_armed, false, false, false, false, false, false);
    }
    let rxne_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    if !wait(I2C2_SR1, BTF, BTF) {
        return (
            0xFFFF,
            pos_armed,
            false,
            rxne_isolated,
            false,
            false,
            false,
            false,
        );
    }
    let btf_irq = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT != 0;

    // Complete the receive before draining DR. STOP must release BUSY while
    // retaining both completed bytes for the following two DR reads.
    wr(I2C2_CR1, PE | POS | STOP);
    let stop_before_dr = wait(I2C2_SR2, 0x0002, 0);
    let msb = rd(I2C2_DR) as u8;
    let first_read_keeps_rxne = rd(I2C2_SR1) & RXNE != 0;
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let mut btf_released = true;
    for _ in 0..16 {
        btf_released &= rd(I2C2_SR1) & BTF == 0;
        btf_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
    }
    let lsb = rd(I2C2_DR) as u8;
    let final_rxne_cleared = rd(I2C2_SR1) & RXNE == 0;
    (
        u16::from_be_bytes([msb, lsb]),
        pos_armed,
        btf_irq,
        rxne_isolated,
        stop_before_dr,
        first_read_keeps_rxne,
        btf_released,
        final_rxne_cleared,
    )
}

/// Probe an absent address on the imported I2C2 instance. Check that the
/// controller records AF while ADDR is clear, that ITERREN gates the ER vector,
/// then request STOP, clear AF, and verify the bus and error source recover.
fn smoke_i2c2_nack() -> (u32, bool, bool, bool, bool, bool, bool, bool, bool) {
    const PE: u32 = 1;
    const START: u32 = 1 << 8;
    const STOP: u32 = 1 << 9;
    const SB: u32 = 1;
    const ADDR: u32 = 1 << 1;
    const AF: u32 = 1 << 10;
    const MSL_BUSY: u32 = 0x3;

    // Keep ITERREN and ITBUFEN disabled initially. The NACK should set AF with
    // neither I2C2 interrupt vector active; enabling ITERREN should then pend
    // only the error vector.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT | I2C2_ER_IRQ_BIT);
    wr(I2C2_CR1, PE | START);
    if !wait(I2C2_SR1, SB, SB) {
        return (0, false, false, false, false, false, false, false, false);
    }
    wr(I2C2_DR, 0xA0); // absent 7-bit address 0x50, write
    if !wait(I2C2_SR1, AF, AF) {
        return (
            rd(I2C2_SR1),
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        );
    }

    // The START event left a sticky NVIC pending bit. Clear it after the DR
    // write cleared SB so the following observations isolate the NACK phase.
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT);
    let sr1 = rd(I2C2_SR1);
    let sr2 = rd(I2C2_SR2);
    let nack_flags = sr1 & AF != 0 && sr1 & ADDR == 0 && sr2 & MSL_BUSY == MSL_BUSY;
    let event_isolated = rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;

    let er_isolated = rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    wr(I2C2_CR2, 16 | (1 << 8) | (1 << 9)); // enable ITERREN
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
    }
    let er_irq = rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT != 0;

    // ITERREN gates the level source even while AF remains set. Clear the
    // sticky NVIC bit after disabling it, then ensure it does not re-pend.
    wr(I2C2_CR2, 16 | (1 << 9));
    wr(NVIC_ICPR1, I2C2_ER_IRQ_BIT);
    let mut er_masked_release = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        er_masked_release &= rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    }

    wr(I2C2_CR1, PE | STOP);
    wr(I2C2_SR1, !(AF) & 0xFFFF); // AF is cleared by writing zero to the flag
    let af_cleared = rd(I2C2_SR1) & AF == 0;
    let idle = wait(I2C2_SR2, 1 << 1, 0);
    wr(NVIC_ICPR1, I2C2_EV_IRQ_BIT | I2C2_ER_IRQ_BIT);
    let mut event_released = true;
    let mut er_released = true;
    for _ in 0..16 {
        let _ = rd(I2C2_SR1);
        event_released &= rd(NVIC_ISPR1) & I2C2_EV_IRQ_BIT == 0;
        er_released &= rd(NVIC_ISPR1) & I2C2_ER_IRQ_BIT == 0;
    }
    (
        sr1,
        nack_flags,
        event_isolated && event_released,
        er_isolated,
        er_irq,
        er_masked_release,
        af_cleared,
        idle,
        er_released,
    )
}

/// The fixture MAX31855 is on SPI1 with PA4 as GPIO chip select. Check the
/// idle TXE interrupt level, then the receive level before reading each DR
/// byte and its release after the last byte is consumed.
fn smoke_spi1() -> bool {
    const TXE: u32 = 1 << 1;
    const RXNE: u32 = 1;
    const BSY: u32 = 1 << 7;

    // PA4 as a push-pull output, initially deasserted (high).
    wr(
        GPIOA + MODER,
        (rd(GPIOA + MODER) & !(0b11 << 8)) | (0b01 << 8),
    );
    wr(GPIOA + BSRR, 1 << 4);

    // SPI1 master, software NSS, mode 0, 8-bit frames, PCLK2/8.
    wr(SPI_CR1, (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2));
    wr(SPI_CR1, rd(SPI_CR1) | (1 << 6)); // SPE
    wr(NVIC_ICPR1, SPI1_IRQ_BIT);
    wr(SPI_CR2, 1 << 7); // TXEIE
    let txe_pended = rd(SPI_SR) & TXE != 0 && rd(NVIC_ISPR1) & SPI1_IRQ_BIT != 0;
    wr(SPI_CR2, 0);
    let txe_released = rd(NVIC_ISPR1) & SPI1_IRQ_BIT == 0;
    wr(NVIC_ICPR1, SPI1_IRQ_BIT);

    wr(SPI_CR2, 1 << 6); // RXNEIE
    wr(GPIOA + BSRR, 1 << (4 + 16)); // select MAX31855
    let mut frame = 0u32;
    let mut rxne_pended = true;
    let mut transfer_ok = true;
    for _ in 0..4 {
        if !wait(SPI_SR, TXE, TXE) {
            transfer_ok = false;
            break;
        }
        wr(SPI_DR, 0);
        if !wait(SPI_SR, RXNE, RXNE) {
            transfer_ok = false;
            break;
        }
        rxne_pended &= rd(NVIC_ISPR1) & SPI1_IRQ_BIT != 0;
        frame = (frame << 8) | (rd(SPI_DR) & 0xFF);
    }
    let idle = wait(SPI_SR, BSY, 0);
    wr(GPIOA + BSRR, 1 << 4); // deselect
    let rxne_released = rd(NVIC_ISPR1) & SPI1_IRQ_BIT == 0 && rd(SPI_SR) & RXNE == 0;
    wr(SPI_CR2, 0);
    wr(SPI_CR1, 0);

    let ok = transfer_ok
        && idle
        && frame == 0x0190_1600
        && txe_pended
        && txe_released
        && rxne_pended
        && rxne_released;
    send(b"spi1 max31855=");
    put_hex32(frame);
    send(b" txe_irq=");
    putc(if txe_pended { b'1' } else { b'0' });
    send(b" rxne_irq=");
    putc(if rxne_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if txe_released && rxne_released {
        b'1'
    } else {
        b'0'
    });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
}

/// USART1 is imported with NVIC IRQ 37. TXE must stay off the line while
/// TXEIE is clear, pend when TXEIE is enabled, and release when disabled.
fn smoke_usart1_txe_irq() -> bool {
    const TXE: u32 = 1 << 7;
    const TXEIE: u32 = 1 << 7;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE; TXEIE remains clear

    let txe_isolated = rd(USART1_SR) & TXE != 0 && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 3) | TXEIE); // enable TXEIE with TXE already set
    let txe_pended = txe_isolated && wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);

    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE, TXEIE off
                                          // Clear the latched NVIC pending bit, then allow the level source to
                                          // settle. A still-asserted USART line will immediately re-pend IRQ 37.
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let txe_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 txe_isolated=");
    putc(if txe_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if txe_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if txe_released { b'1' } else { b'0' });
    send(if txe_isolated && txe_pended && txe_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    txe_isolated && txe_pended && txe_released
}

/// USART1's idle TC flag must pend imported NVIC IRQ 37 only while TCIE is set.
/// TXEIE stays clear so the completion source is tested independently.
fn smoke_usart1_tcie_irq() -> bool {
    const TC: u32 = 1 << 6;
    const TCIE: u32 = 1 << 6;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE; TCIE and TXEIE clear

    let tc_isolated = rd(USART1_SR) & TC != 0 && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 3) | TCIE); // enable TCIE with TC already set
    let tc_pended = tc_isolated && wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);

    wr(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE, TCIE off
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let tc_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 tc_isolated=");
    putc(if tc_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if tc_pended { b'1' } else { b'0' });
    send(b" released=");
    putc(if tc_released { b'1' } else { b'0' });
    send(if tc_isolated && tc_pended && tc_released {
        b" ok\r\n"
    } else {
        b" FAIL\r\n"
    });
    tc_isolated && tc_pended && tc_released
}

/// The onboarding stimulus queues `R` on USART1 before reset. RXNE must stay
/// off IRQ 37 while RXNEIE is clear, then pend it when RXNEIE is enabled with
/// the byte still waiting. Reading DR must release the source.
fn smoke_usart1_rxne_irq() -> bool {
    const RXNE: u32 = 1 << 5;
    const RXNEIE: u32 = 1 << 5;
    const EXPECTED: u32 = b'R' as u32;

    wr(RCC_APB2ENR, rd(RCC_APB2ENR) | (1 << 4)); // USART1EN
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    wr(USART1_CR1, (1 << 13) | (1 << 2)); // UE | RE; RXNEIE remains clear

    let mut rxne_seen = false;
    for _ in 0..4096 {
        if rd(USART1_SR) & RXNE != 0 {
            rxne_seen = true;
            break;
        }
    }
    let rxne_isolated = rxne_seen && rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;
    wr(USART1_CR1, (1 << 13) | (1 << 2) | RXNEIE); // enable RXNEIE with RXNE pending
    let rxne_pended = wait(NVIC_ISPR1, USART1_IRQ_BIT, USART1_IRQ_BIT);
    let data = rd(USART1_DR) & 0xFF;
    let rxne_cleared = rd(USART1_SR) & RXNE == 0;
    // NVIC pending is sticky. Clear it after consuming DR and make sure the
    // still-enabled RXNEIE source does not pend again with an empty queue.
    wr(NVIC_ICPR1, USART1_IRQ_BIT);
    for _ in 0..64 {
        let _ = rd(USART1_SR);
    }
    let irq_released = rd(NVIC_ISPR1) & USART1_IRQ_BIT == 0;

    send(b"usart1 rxne_isolated=");
    putc(if rxne_isolated { b'1' } else { b'0' });
    send(b" gate_asserted=");
    putc(if rxne_pended { b'1' } else { b'0' });
    send(b" data=");
    put_hex32(data);
    send(b" released=");
    putc(if rxne_cleared && irq_released {
        b'1'
    } else {
        b'0'
    });
    send(
        if rxne_isolated && rxne_pended && data == EXPECTED && rxne_cleared && irq_released {
            b" ok\r\n"
        } else {
            b" FAIL\r\n"
        },
    );
    rxne_isolated && rxne_pended && data == EXPECTED && rxne_cleared && irq_released
}

/// USART2 is both the smoke console and the imported Black Pill's IRQ 38
/// receiver. The script queues two bytes before reset. Neither may appear in
/// SR.RXNE until both legacy CR1.UE and CR1.RE are set; with RXNEIE clear the
/// line stays low, then asserts and releases as DR is drained.
fn smoke_usart2_rx_gate() -> bool {
    const RXNE: u32 = 1 << 5;
    const RXNEIE: u32 = 1 << 5;
    const UE: u32 = 1 << 13;
    const RE: u32 = 1 << 2;
    const TE: u32 = 1 << 3;
    const EXPECTED: u32 = 0x5253; // "RS"

    // main enabled UE|TE for the console, leaving RE disabled. Start-injected
    // bytes must remain on the wire through both one-bit-only configurations.
    let mut re_off = true;
    for _ in 0..16 {
        re_off &= rd(USART_SR) & RXNE == 0;
    }
    wr(USART_CR1, RE | TE); // RE set, UE clear
    let mut ue_off = true;
    for _ in 0..16 {
        ue_off &= rd(USART_SR) & RXNE == 0;
    }

    wr(NVIC_ICPR1, USART2_IRQ_BIT);
    wr(USART_CR1, UE | RE | TE); // both receiver enables; RXNEIE stays clear
    let mut rxne_seen = false;
    for _ in 0..4096 {
        if rd(USART_SR) & RXNE != 0 {
            rxne_seen = true;
            break;
        }
    }
    let irq_isolated = rxne_seen && rd(NVIC_ISPR1) & USART2_IRQ_BIT == 0;
    wr(USART_CR1, UE | RE | TE | RXNEIE);
    let irq_asserted = rxne_seen && wait(NVIC_ISPR1, USART2_IRQ_BIT, USART2_IRQ_BIT);

    let first = rd(USART_DR) & 0xFF;
    let rxne_after_first = rd(USART_SR) & RXNE != 0;
    let second = if rxne_after_first {
        rd(USART_DR) & 0xFF
    } else {
        0
    };
    let rxne_cleared = rd(USART_SR) & RXNE == 0;
    wr(USART_CR1, UE | TE); // disable RXNEIE and RE before final console output
    wr(NVIC_ICPR1, USART2_IRQ_BIT);
    let mut irq_released = true;
    for _ in 0..16 {
        irq_released &= rd(NVIC_ISPR1) & USART2_IRQ_BIT == 0;
    }

    let data = (first << 8) | second;
    let ok = re_off
        && ue_off
        && irq_isolated
        && irq_asserted
        && data == EXPECTED
        && rxne_after_first
        && rxne_cleared
        && irq_released;
    send(b"usart2 rx_gate re_off=");
    send(if re_off { b"1" } else { b"0" });
    send(b" ue_off=");
    send(if ue_off { b"1" } else { b"0" });
    send(b" data=");
    put_hex32(data);
    send(b" rxne_after_first=");
    send(if rxne_after_first { b"1" } else { b"0" });
    send(b" rxne_after=");
    send(if rxne_cleared { b"0" } else { b"1" });
    send(b" irq=");
    send(if irq_isolated {
        b"isolated"
    } else {
        b"not_isolated"
    });
    send(if irq_asserted {
        b",asserted"
    } else {
        b",not_asserted"
    });
    send(if irq_released {
        b",released"
    } else {
        b",not_released"
    });
    send(if ok { b" ok\r\n" } else { b" FAIL\r\n" });
    ok
}

fn wait(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..20_000 {
        if rd(addr) & mask == want {
            return true;
        }
    }
    false
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
