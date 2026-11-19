#![no_std]
#![no_main]

use core::panic::PanicInfo;

const RESULT: *mut u32 = 0x1000_0000 as *mut u32;

#[link_section = ".text.entry"]
#[no_mangle]
pub extern "C" fn _start() -> ! {
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
    }

    #[cfg(feature = "timer0")]
    unsafe {
        timer0_smoke();
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

// Fomu timer0 (Renode `cpus/fomu.repl` LiteX_Timer @ 0x60002800). Foboot's
// generated `csr.h` uses csr_data_width = 8: one CSR byte per 4-byte slot,
// most-significant byte first.
#[cfg(feature = "timer0")]
mod timer0 {
    const BASE: usize = 0x6000_2800;
    pub const LOAD: usize = BASE;
    pub const RELOAD: usize = BASE + 0x10;
    pub const EN: *mut u32 = (BASE + 0x20) as *mut u32;
    pub const UPDATE_VALUE: *mut u32 = (BASE + 0x24) as *mut u32;
    pub const VALUE: usize = BASE + 0x28;
    pub const EV_STATUS: *const u32 = (BASE + 0x38) as *const u32;
    pub const EV_PENDING: *mut u32 = (BASE + 0x3C) as *mut u32;
    pub const EV_ENABLE: *mut u32 = (BASE + 0x40) as *mut u32;
}

#[cfg(feature = "timer0")]
#[inline(always)]
unsafe fn csr8_write(addr: usize, value: u32) {
    for i in 0..4 {
        let byte = (value >> (24 - 8 * i)) & 0xFF;
        core::ptr::write_volatile((addr + 4 * i) as *mut u32, byte);
    }
}

#[cfg(feature = "timer0")]
#[inline(always)]
unsafe fn csr8_read(addr: usize) -> u32 {
    let mut value = 0;
    for i in 0..4 {
        value = (value << 8) | (core::ptr::read_volatile((addr + 4 * i) as *const u32) & 0xFF);
    }
    value
}

/// Results: +4 load readback, +8 latched countdown value, +0xC check bits,
/// +0x10 pass word (0xF0A0_003F when all six checks hold).
#[cfg(feature = "timer0")]
#[inline(always)]
unsafe fn timer0_smoke() {
    use core::ptr::{read_volatile, write_volatile};
    use timer0::*;

    let mut bits = 0u32;
    // Disabled: stage a 32-bit load across the four byte slots, read it back.
    write_volatile(EN, 0);
    csr8_write(LOAD, 0x0001_2345);
    let load = csr8_read(LOAD);
    write_volatile(RESULT.add(1), load);
    if load == 0x0001_2345 {
        bits |= 1 << 0;
    }
    // One-shot of 2000 cycles; clear any edge latched while staging.
    csr8_write(RELOAD, 0);
    csr8_write(LOAD, 2000);
    write_volatile(EV_PENDING, 1);
    write_volatile(EV_ENABLE, 0);
    if read_volatile(EV_STATUS) & 1 == 0 && read_volatile(EV_PENDING) & 1 == 0 {
        bits |= 1 << 1;
    }
    write_volatile(EN, 1);
    if read_volatile(EN) & 1 == 1 {
        bits |= 1 << 2;
    }
    // Latch the live countdown: it must be running, below the load.
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
    write_volatile(RESULT.add(3), bits);
    write_volatile(
        RESULT.add(4),
        if bits == 0x3F {
            0xF0A0_003F
        } else {
            0xBAD0_0000 | bits
        },
    );
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
