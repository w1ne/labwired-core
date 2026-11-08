#![no_std]
#![no_main]

use core::panic::PanicInfo;

const RESULT: *mut u32 = 0x2060_0000 as *mut u32;
const PASS: *mut u32 = 0x2060_0010 as *mut u32;

// Renode `htif_mem` @0xD0580000 (cpus/tock_veer_el2_sim.repl): the
// veer_el2-tock.resc watchpoint prints every byte written to the first word,
// the console Tock's VeeR EL2 board uses. 0x01 and 0xFF mean "finish", so the
// banner must avoid them.
const HTIF: *mut u8 = 0xD058_0000 as *mut u8;

const BANNER: &[u8] = b"VeeR EL2 htif ok\n";
const PASS_WORD: u32 = 0x7E3E_E120;

core::arch::global_asm!(
    ".section .text.entry,\"ax\",@progbits",
    ".globl _start",
    ".type _start,@function",
    "_start:",
    "la sp, __stack_top",
    "call main",
    "1: j 1b",
);

#[no_mangle]
pub extern "C" fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = lateout(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        core::ptr::write_volatile(RESULT, product);

        // Byte stores, as Tock's VeeR EL2 console writes them.
        for &byte in BANNER {
            core::ptr::write_volatile(HTIF, byte);
        }

        if product == 0x143 {
            core::ptr::write_volatile(PASS, PASS_WORD);
        }
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
