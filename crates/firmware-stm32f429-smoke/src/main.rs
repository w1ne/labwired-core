#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC: u32 = 0x4002_3800;
const RCC_AHB1ENR: *mut u32 = (RCC + 0x30) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC + 0x40) as *mut u32;

const GPIOA: u32 = 0x4002_0000;
const GPIOD: u32 = 0x4002_0C00;
const USART2: u32 = 0x4000_4400;

const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const BSRR: u32 = 0x18;
const USART_SR: *const u32 = USART2 as *const u32;
const USART_DR: *mut u32 = (USART2 + 0x04) as *mut u32;
const USART_BRR: *mut u32 = (USART2 + 0x08) as *mut u32;
const USART_CR1: *mut u32 = (USART2 + 0x0C) as *mut u32;

const TXE: u32 = 1 << 7;
const LED: u32 = 1 << 12;

const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_SHPR3: u32 = 0xE000_ED20;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;

// These scratch locations are in the low RAM region, away from the runtime's
// descending MSP stack at the top of SRAM.
const TEST_VECTOR_TABLE: u32 = 0x2000_0080;
const PSP_TAILCHAIN_RECORD: u32 = 0x2000_0100;
const PSP_TAILCHAIN_STACK_TOP: u32 = 0x2000_0200;
const THREAD_R0_MARKER: u32 = 0x5448_5244;
const HANDLER_R0_MARKER: u32 = 0x5045_4E44;

fn register_rmw(address: u32, clear: u32, set: u32) {
    unsafe {
        let register = address as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn configure_usart2_pins() {
    for pin in [2u32, 3] {
        register_rmw(GPIOA + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
        register_rmw(GPIOA + AFRL, 0xF << (pin * 4), 7 << (pin * 4));
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_SR) & TXE == 0 {}
        write_volatile(USART_DR, byte as u32);
    }
}

fn send(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn set_led(on: bool) {
    let bit = if on { LED } else { LED << 16 };
    unsafe { write_volatile((GPIOD + BSRR) as *mut u32, bit) };
}

fn delay() {
    for _ in 0..40_000 {
        unsafe {
            let _ = read_volatile((GPIOD + 0x14) as *const u32);
        }
    }
}

/// Verify that a SysTick exception entered from Thread mode on PSP tail-chains
/// to the lower-priority PendSV handler without replacing its exception frame.
fn smoke_systick_pendsv_psp_tailchain() -> bool {
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
    let prior_msp: u32;
    unsafe {
        core::arch::asm!(
            "mrs {}, MSP",
            out(reg) prior_msp,
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
        // SysTick outranks PendSV; leave the reserved SHPR3 bytes untouched.
        write_volatile(
            SCB_SHPR3 as *mut u32,
            (prior_shpr3 & 0x0000_FFFF) | 0x80C0_0000,
        );
        for slot in 0..16 {
            write_volatile((TEST_VECTOR_TABLE + slot * 4) as *mut u32, 0);
        }
        write_volatile(
            (TEST_VECTOR_TABLE + 14 * 4) as *mut u32,
            (psp_tailchain_pendsv_handler as *const () as usize as u32) | 1,
        );
        write_volatile(
            (TEST_VECTOR_TABLE + 15 * 4) as *mut u32,
            (psp_tailchain_systick_handler as *const () as usize as u32) | 1,
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
        unsafe { pend_systick_from_psp() };
    }

    let record = PSP_TAILCHAIN_RECORD;
    let parent_ipsr = unsafe { read_volatile(record as *const u32) };
    let parent_icsr_before = unsafe { read_volatile((record + 4) as *const u32) };
    let parent_icsr_after = unsafe { read_volatile((record + 8) as *const u32) };
    let parent_msp = unsafe { read_volatile((record + 12) as *const u32) };
    let parent_exc_return = unsafe { read_volatile((record + 16) as *const u32) };
    let thread_r0 = unsafe { read_volatile((record + 20) as *const u32) };
    let child_ipsr = unsafe { read_volatile((record + 24) as *const u32) };
    let child_msp = unsafe { read_volatile((record + 28) as *const u32) };
    let child_exc_return = unsafe { read_volatile((record + 32) as *const u32) };
    let child_r0 = unsafe { read_volatile((record + 36) as *const u32) };
    let child_icsr = unsafe { read_volatile((record + 40) as *const u32) };
    let child_hit = unsafe { read_volatile((record + 44) as *const u32) == 1 };
    let parent_psp = unsafe { read_volatile((record + 48) as *const u32) };
    let child_psp = unsafe { read_volatile((record + 52) as *const u32) };
    let wrapper_msp = unsafe { read_volatile((record + 56) as *const u32) };
    let wrapper_psp = unsafe { read_volatile((record + 60) as *const u32) };
    let wrapper_control = unsafe { read_volatile((record + 64) as *const u32) };

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
        core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
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
        && parent_psp == PSP_TAILCHAIN_STACK_TOP - 32
        && wrapper_msp == prior_msp
        && parent_msp == prior_msp - 16;
    let child_is_pendsv = child_hit
        && child_ipsr == 14
        && child_icsr & 0x1FF == 14
        && child_icsr & ICSR_PENDSVSET == 0;
    let r0_live =
        thread_r0 == THREAD_R0_MARKER && child_r0 == HANDLER_R0_MARKER && child_r0 != thread_r0;
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

    send(b"systick pendsv psp tailchain frame=");
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
    for shift in (0..8).rev() {
        let digit = ((child_ipsr >> (shift * 4)) & 0xF) as u8;
        putc(if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        });
    }
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
    passed
}

/// Switch Thread mode to a dedicated PSP stack, then software-pend SysTick.
#[unsafe(naked)]
unsafe extern "C" fn pend_systick_from_psp() {
    core::arch::naked_asm!(
        "mrs r3, msp",
        "push {{r4, r5, r6, lr}}",
        "mrs r4, control",
        "mrs r5, psp",
        "ldr r0, =0x20000100",
        "str r3, [r0, #56]",
        "str r5, [r0, #60]",
        "str r4, [r0, #64]",
        "ldr r1, =0x20000200",
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
        "ldr r1, =0x20000100",
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
unsafe extern "C" fn psp_tailchain_systick_handler() {
    core::arch::naked_asm!(
        "mov r2, r0",
        "ldr r0, =0x20000100",
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
        "ldr r0, =0x20000100",
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

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOA/GPIOD on AHB1 and USART2 on APB1.
        write_volatile(
            RCC_AHB1ENR,
            read_volatile(RCC_AHB1ENR) | (1 << 0) | (1 << 3),
        );
        write_volatile(RCC_APB1ENR, read_volatile(RCC_APB1ENR) | (1 << 17));
    }

    configure_usart2_pins();
    register_rmw(GPIOD + MODER, 0b11 << 24, 0b01 << 24);
    unsafe {
        // The smoke checks the register path, not baud accuracy or a clock tree.
        write_volatile(USART_BRR, 0x8B);
        write_volatile(USART_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    set_led(false);
    send(b"STM32F4 shared profile smoke\n");

    let psp_tailchain_ok = smoke_systick_pendsv_psp_tailchain();
    send(if psp_tailchain_ok {
        b"STM32F429 PSP tailchain PASS\r\n"
    } else {
        b"STM32F429 PSP tailchain FAIL\r\n"
    });

    loop {
        set_led(true);
        delay();
        set_led(false);
        delay();
    }
}
