#![no_std]
#![no_main]

use core::arch::asm;
use core::panic::PanicInfo;
use core::ptr::{read_volatile, write_volatile};

const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x38) as *mut u32;

const IO_MUX_GPIO5: *mut u32 = 0x6009_0014 as *mut u32;
const GPIO5_MASK: u32 = 1 << 5;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        // Espressif C5 io_mux_reg.h: GPIO5 is IO_MUX base + 0x14 and
        // FUNC_MTDO_GPIO5 is MCU_SEL=1 at bits [14:12].
        let pad = read_volatile(IO_MUX_GPIO5);
        write_volatile(IO_MUX_GPIO5, (pad & !(0x7 << 12)) | (1 << 12));

        write_volatile(GPIO_OUT_W1TC, GPIO5_MASK);
        write_volatile(GPIO_ENABLE_W1TS, GPIO5_MASK);

        loop {
            write_volatile(GPIO_OUT_W1TS, GPIO5_MASK);
            delay();
            write_volatile(GPIO_OUT_W1TC, GPIO5_MASK);
            delay();
        }
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..1_000 {
        // A real base-ISA instruction keeps the output transitions separated
        // for both the simulator trace and a physical scope/LED check.
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
