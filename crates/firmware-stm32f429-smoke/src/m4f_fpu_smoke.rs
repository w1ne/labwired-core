use core::arch::asm;
use core::ptr::{read_volatile, write_volatile};

const SCB_CPACR: *mut u32 = 0xE000_ED88 as *mut u32;
const CPACR_CP10_CP11_FULL_ACCESS: u32 = 0x00F0_0000;
const MVFR0: *const u32 = 0xE000_EF40 as *const u32;
const MVFR1: *const u32 = 0xE000_EF44 as *const u32;
const MVFR2: *const u32 = 0xE000_EF48 as *const u32;

/// Unlock and execute the Cortex-M4F single-precision datapath.
pub fn run() {
    let cpacr_before = unsafe { read_volatile(SCB_CPACR) };
    unsafe {
        write_volatile(SCB_CPACR, cpacr_before | CPACR_CP10_CP11_FULL_ACCESS);
        asm!("dsb", "isb", options(nostack, preserves_flags));
    }

    let cpacr = unsafe { read_volatile(SCB_CPACR) };
    let mvfr0 = unsafe { read_volatile(MVFR0) };
    let mvfr1 = unsafe { read_volatile(MVFR1) };
    let mvfr2 = unsafe { read_volatile(MVFR2) };

    let sum: u32;
    unsafe {
        asm!(
            "vmov s0, {lhs}",
            "vmov s1, {rhs}",
            "vadd.f32 s2, s0, s1",
            "vmov {sum}, s2",
            lhs = in(reg) 0x3F80_0000u32,
            rhs = in(reg) 0x4000_0000u32,
            sum = out(reg) sum,
            options(nostack, preserves_flags),
        );
    }

    let passed = cpacr & CPACR_CP10_CP11_FULL_ACCESS == CPACR_CP10_CP11_FULL_ACCESS
        && mvfr0 == 0x1011_0021
        && mvfr1 == 0x1100_0011
        && mvfr2 == 0
        && sum == 0x4040_0000;
    send(b"fpu m4f cpacr=");
    send(
        if cpacr & CPACR_CP10_CP11_FULL_ACCESS == CPACR_CP10_CP11_FULL_ACCESS {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" mvfr=");
    send(
        if mvfr0 == 0x1011_0021 && mvfr1 == 0x1100_0011 && mvfr2 == 0 {
            b"1"
        } else {
            b"0"
        },
    );
    send(b" add=");
    send(if sum == 0x4040_0000 { b"1" } else { b"0" });
    send(if passed { b" ok\r\n" } else { b" FAIL\r\n" });
}

fn send(bytes: &[u8]) {
    const USART_SR: *const u32 = 0x4000_4400 as *const u32;
    const USART_DR: *mut u32 = 0x4000_4404 as *mut u32;
    const USART_TXE: u32 = 1 << 7;

    for byte in bytes {
        unsafe {
            while read_volatile(USART_SR) & USART_TXE == 0 {}
            write_volatile(USART_DR, u32::from(*byte));
        }
    }
}
