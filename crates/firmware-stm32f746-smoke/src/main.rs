#![no_std]
#![no_main]

//! STM32F746xG smoke for the imported Renode platform: RCC clock gates and
//! ENR masks, USART1/2/3/6 + UART4/5/7/8, GPIOA..K, SYSCFG EXTICR + EXTI,
//! TIM1..TIM14, SPI1/2/3 (FIFO SPI), I2C1..4 ("v2" I2C), DMA1/DMA2 (stream
//! DMA), CRC and RNG. Register offsets, bases and IRQ numbers are RM0385 and
//! ST's CMSIS `stm32f746xx.h`.
//!
//! The fixture system puts a MAX31855 on each SPI (thermocouple 25, 100 and
//! 75 °C, so each bus answers with its own frame), a second MAX31855 on SPI1
//! behind its own chip select (PB0, 50 °C) and a TMP102 at 0x48 on I2C1.
//!
//! Interrupts stay disabled in the NVIC: each check reads the line's
//! ISPR bit, which proves the block reaches the right NVIC input without a
//! device vector table. Every check prints one line and sets one bit of the
//! summary word at `SUMMARY`.

use core::arch::asm;
use cortex_m_rt::entry;
use panic_halt as _;

mod f746_checks;
use f746_checks::*;

const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const TEST_VECTOR_TABLE: u32 = 0x2000_4000;
const PENDSV_HANDLER_COUNT: u32 = 0x2000_4100;
const PENDSV_HANDLER_IPSR: u32 = 0x2000_4104;
const SYSTICK_HANDLER_COUNT: u32 = 0x2000_4108;
const SYSTICK_HANDLER_IPSR: u32 = 0x2000_410C;
const SYSTICK_PENDSV_ARMED: u32 = 0x2000_4110;
const SYSTICK_PENDSV_FLAGS: u32 = 0x2000_4114;
const PENDSV_SYSTICK_COUNT_AT_ENTRY: u32 = 0x2000_4118;

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
    let (rcc_reset, rcc_gate, rcc_mask) = rcc_bringup();
    for &(_, base, _) in &UARTS {
        uart_init(base);
    }
    puts("STM32F746 Cortex-M7 smoke\r\n");
    for &(name, base, _) in &UARTS[1..] {
        send(base, name.as_bytes());
        send(base, b" ok\r\n");
    }

    let mut summary: u32 = 0;
    let mut mark = |bit: u32, ok: bool| {
        if ok {
            summary |= 1 << bit;
        }
    };
    mark(0, report_rcc(rcc_reset, rcc_gate, rcc_mask));
    mark(1, check_usart_irqs());
    mark(2, check_gpio());
    mark(3, check_syscfg());
    mark(4, check_exti_swier());
    let mut timers = true;
    for &(name, base, irq, width) in &TIMERS {
        timers &= check_timer(name, base, irq, width);
    }
    mark(5, timers);
    mark(6, check_exti_pads());
    // Park the second SPI1 slave's CS high before SPI1 is used at all (an
    // undriven CS pad leaves its device on the bus).
    cs_output(GPIOB, 0);
    for (i, &(name, base, irq, port, pin, want)) in SPIS.iter().enumerate() {
        mark(7 + i as u32, check_spi(name, base, irq, port, pin, want));
    }
    mark(10, check_spi1_shared_cs());
    mark(
        11,
        check_spi_dma(
            "dma2 spi1",
            SPI1,
            GPIOA,
            4,
            SPIS[0].5,
            DMA2,
            &DMA2_LINES,
            0,
            3,
            3,
            2,
        ),
    );
    mark(
        12,
        check_spi_dma(
            "dma1 spi2",
            SPI2,
            GPIOB,
            12,
            SPIS[1].5,
            DMA1,
            &DMA1_LINES,
            3,
            4,
            0,
            1,
        ),
    );
    mark(13, check_dma_m2m());
    mark(14, check_i2c1_tmp102());
    for (i, &(name, base, ev, er)) in I2CS.iter().enumerate() {
        mark(15 + i as u32, check_i2c_nack(name, base, ev, er));
    }
    mark(19, check_crc());
    mark(20, check_rng());
    check_pendsv_m7();
    let summary = summary | 0x7460_0000;
    wr(SUMMARY, summary);
    puts("summary ");
    hex(summary);
    puts("\r\nDONE\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
