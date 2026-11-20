#![no_std]
#![no_main]

use core::panic::PanicInfo;

// The profile maps these addresses to the imported 512 KiB SRAM window.
const RESULT: *mut u32 = 0x1C000000 as *mut u32;
const STDOUT_READ: *mut u32 = 0x1C000004 as *mut u32;
const PASS: *mut u32 = 0x1C000010 as *mut u32;

// Renode `UART.PULP_STDOUT` (cpus/core-v-mcu.repl): one write-only STD_OUT
// register; a 32-bit write transmits the low byte.
const STD_OUT: *mut u32 = 0x1A10FF80 as *mut u32;

const BANNER: &[u8] = b"CORE-V stdout ok\n";
const PASS_WORD: u32 = 0xC0DE_5701;

// No stack is set up by the boot ROM: point sp at the top of SRAM first.
core::arch::global_asm!(
    ".section .text.entry, \"ax\"",
    ".globl _start",
    "_start:",
    "li sp, 0x1C080000",
    "j main",
);

#[no_mangle]
pub extern "C" fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            "c.nop",
            product = lateout(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        core::ptr::write_volatile(RESULT, product);

        // Sentinel first so a zeroed RAM word cannot fake the readback.
        core::ptr::write_volatile(STDOUT_READ, 0xDEAD_BEEF);
        for &byte in BANNER {
            // Upper bits are reserved; only the low byte is transmitted.
            core::ptr::write_volatile(STD_OUT, 0x5A00 | u32::from(byte));
        }
        // STD_OUT is write-only: reads return 0.
        core::ptr::write_volatile(STDOUT_READ, core::ptr::read_volatile(STD_OUT));

        if product == 0x143 && core::ptr::read_volatile(STDOUT_READ) == 0 {
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
