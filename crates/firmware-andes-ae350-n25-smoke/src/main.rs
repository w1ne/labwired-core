#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[cfg(not(feature = "crosslink-nx-evn"))]
const RESULT: *mut u32 = 0x1000 as *mut u32;

#[cfg(feature = "crosslink-nx-evn")]
const RESULT: *mut u32 = 0x4000_1000 as *mut u32;

// AE350 uart1 (Renode `andes_ae350_n25.repl`: NS16550 @ 0xF0300020 with
// `wideRegisters: true`), so 16550 registers on a 4-byte stride. ET171 uart0
// (Renode `egis_et171.repl`) is the same NS16550 at 0xF0200020.
#[cfg(feature = "ae350-uart")]
mod uart1 {
    #[cfg(not(feature = "et171-uart"))]
    const BASE: usize = 0xF030_0020;
    #[cfg(feature = "et171-uart")]
    const BASE: usize = 0xF020_0020;
    pub const THR: *mut u32 = BASE as *mut u32; // DLL while LCR.DLAB
    pub const DLM: *mut u32 = (BASE + 0x04) as *mut u32; // IER while !DLAB
    pub const LCR: *mut u32 = (BASE + 0x0C) as *mut u32;
    pub const LSR: *const u32 = (BASE + 0x14) as *const u32;
    pub const LSR_THRE: u32 = 1 << 5;
    pub const LSR_TEMT: u32 = 1 << 6;
}

#[cfg(feature = "ae350-uart")]
unsafe fn uart_smoke() {
    use core::ptr::{read_volatile, write_volatile};
    use uart1::*;

    // 8N1 with a 0x0141 divisor latched through DLAB, then read it back.
    write_volatile(LCR, 0x83);
    write_volatile(THR, 0x41);
    write_volatile(DLM, 0x01);
    let divisor = (read_volatile(THR) & 0xFF) | ((read_volatile(DLM) & 0xFF) << 8);
    write_volatile(LCR, 0x03);
    let lcr = read_volatile(LCR) & 0xFF;
    write_volatile(RESULT.add(2), divisor | (lcr << 16));

    let idle_lsr = read_volatile(LSR) & 0xFF;
    write_volatile(RESULT.add(1), idle_lsr);

    #[cfg(not(feature = "et171-uart"))]
    let banner = b"AE350 N25 uart1 ok\r\n";
    #[cfg(feature = "et171-uart")]
    let banner = b"ET171 uart0 ok\r\n";
    for &b in banner {
        while read_volatile(LSR) & LSR_THRE == 0 {}
        write_volatile(THR, b as u32);
    }
    while read_volatile(LSR) & LSR_TEMT == 0 {}

    let ok = divisor == 0x0141 && lcr == 0x03 && idle_lsr & (LSR_THRE | LSR_TEMT) != 0;
    write_volatile(RESULT.add(3), if ok { 0xAE35_0001 } else { 0xAE35_DEAD });
}

// The UART path calls into Rust that may spill, so give it a stack at the top
// of the 128 KiB DRAM first; the CPU-only builds keep the stackless entry.
#[cfg(feature = "ae350-uart")]
core::arch::global_asm!(
    ".section .text.entry, \"ax\"",
    ".global _start",
    "_start:",
    "    li sp, 0x20000",
    "    j smoke_main",
);

#[cfg_attr(not(feature = "ae350-uart"), link_section = ".text.entry")]
#[cfg_attr(not(feature = "ae350-uart"), export_name = "_start")]
#[cfg_attr(feature = "ae350-uart", export_name = "smoke_main")]
pub extern "C" fn smoke_main() -> ! {
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
        #[cfg(feature = "ae350-uart")]
        uart_smoke();
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
