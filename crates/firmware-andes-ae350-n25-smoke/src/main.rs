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

// CrossLink-NX EVN LiteX SoC (Renode `boards/crosslink-nx-evn.repl`):
// LiteX_UART @ 0xF0002000, LiteX_Timer @ 0xF0002800 (Renode's LiteX_Timer is
// the csr_data_width = 8 layout: one CSR byte per 4-byte slot, MSB first) and
// LiteX_ControlAndStatus LEDs @ 0xF0003000.
#[cfg(feature = "crosslink-litex")]
mod litex {
    pub const UART_RXTX: *mut u32 = 0xF000_2000 as *mut u32;
    pub const UART_TXFULL: *const u32 = 0xF000_2004 as *const u32;

    const TIMER: usize = 0xF000_2800;
    pub const LOAD: usize = TIMER;
    pub const RELOAD: usize = TIMER + 0x10;
    pub const EN: *mut u32 = (TIMER + 0x20) as *mut u32;
    pub const UPDATE_VALUE: *mut u32 = (TIMER + 0x24) as *mut u32;
    pub const VALUE: usize = TIMER + 0x28;
    pub const EV_STATUS: *const u32 = (TIMER + 0x38) as *const u32;
    pub const EV_PENDING: *mut u32 = (TIMER + 0x3C) as *mut u32;
    pub const EV_ENABLE: *mut u32 = (TIMER + 0x40) as *mut u32;

    pub const LEDS: *mut u32 = 0xF000_3000 as *mut u32;
}

#[cfg(feature = "crosslink-litex")]
unsafe fn csr8_write(addr: usize, value: u32) {
    for i in 0..4 {
        let byte = (value >> (24 - 8 * i)) & 0xFF;
        core::ptr::write_volatile((addr + 4 * i) as *mut u32, byte);
    }
}

#[cfg(feature = "crosslink-litex")]
unsafe fn csr8_read(addr: usize) -> u32 {
    let mut value = 0;
    for i in 0..4 {
        value = (value << 8) | (core::ptr::read_volatile((addr + 4 * i) as *const u32) & 0xFF);
    }
    value
}

/// Results: +4 timer load readback, +8 latched countdown, +0xC LED readback,
/// +0x10 check bits, +0x14 pass word, +0x18 UART txfull idle readback.
#[cfg(feature = "crosslink-litex")]
unsafe fn litex_smoke() {
    use core::ptr::{read_volatile, write_volatile};
    use litex::*;

    let mut bits = 0u32;
    // UART: txfull=0 means ready; RX status is outside this bounded profile.
    let txfull = read_volatile(UART_TXFULL) & 1;
    write_volatile(RESULT.add(6), txfull);
    if txfull == 0 {
        bits |= 1 << 0;
    }
    for &b in b"CrossLink-NX litex ok\r\n" {
        while read_volatile(UART_TXFULL) & 1 != 0 {}
        write_volatile(UART_RXTX, b as u32);
    }

    // Timer0 disabled: stage a 32-bit load across the byte slots, read back.
    write_volatile(EN, 0);
    csr8_write(LOAD, 0x0001_2345);
    let load = csr8_read(LOAD);
    write_volatile(RESULT.add(1), load);
    if load == 0x0001_2345 {
        bits |= 1 << 1;
    }
    // One-shot of 2000 cycles; clear any edge latched while staging.
    csr8_write(RELOAD, 0);
    csr8_write(LOAD, 2000);
    write_volatile(EV_PENDING, 1);
    write_volatile(EV_ENABLE, 0);
    if read_volatile(EV_STATUS) & 1 == 0 && read_volatile(EV_PENDING) & 1 == 0 {
        bits |= 1 << 2;
    }
    write_volatile(EN, 1);
    write_volatile(UPDATE_VALUE, 1);
    let value = csr8_read(VALUE);
    write_volatile(RESULT.add(2), value);
    if value > 0 && value < 2000 {
        bits |= 1 << 3;
    }
    // Poll the zero-event level; bounded so a dead timer cannot hang.
    let mut spins = 0u32;
    while read_volatile(EV_STATUS) & 1 == 0 && spins < 100_000 {
        spins += 1;
    }
    if read_volatile(EV_STATUS) & 1 == 1 && read_volatile(EV_PENDING) & 1 == 1 {
        bits |= 1 << 4;
    }
    // ev_pending is write-1-to-clear.
    write_volatile(EV_PENDING, 0);
    let held = read_volatile(EV_PENDING) & 1;
    write_volatile(EV_PENDING, 1);
    if held == 1 && read_volatile(EV_PENDING) & 1 == 0 {
        bits |= 1 << 5;
    }

    // CAS LEDs: the 14 board LEDs (bits 0..13) are readable back.
    write_volatile(LEDS, 0x2A55);
    let leds = read_volatile(LEDS);
    write_volatile(RESULT.add(3), leds);
    if leds == 0x2A55 {
        bits |= 1 << 6;
    }

    write_volatile(RESULT.add(4), bits);
    write_volatile(
        RESULT.add(5),
        if bits == 0x7F {
            0xC205_007F
        } else {
            0xBAD0_0000 | bits
        },
    );
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

// CrossLink-NX: stack at the top of the 8 MiB SRAM.
#[cfg(feature = "crosslink-litex")]
core::arch::global_asm!(
    ".section .text.entry, \"ax\"",
    ".global _start",
    "_start:",
    "    li sp, 0x40800000",
    "    j smoke_main",
);

#[cfg_attr(
    not(any(feature = "ae350-uart", feature = "crosslink-litex")),
    link_section = ".text.entry"
)]
#[cfg_attr(
    not(any(feature = "ae350-uart", feature = "crosslink-litex")),
    export_name = "_start"
)]
#[cfg_attr(
    any(feature = "ae350-uart", feature = "crosslink-litex"),
    export_name = "smoke_main"
)]
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
        #[cfg(feature = "crosslink-litex")]
        litex_smoke();
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
