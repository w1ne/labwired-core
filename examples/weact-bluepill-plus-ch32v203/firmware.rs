#![no_std]
#![no_main]

use core::arch::global_asm;
use core::ptr::{read_volatile, write_volatile};

// WeAct BluePill-Plus-CH32 (CH32V203C8T6): reset-HSI USART1 transmit on PA9
// and the blue user LED on PB2. Mirrors WeAct's CH32V203 01-template example
// (board_led_init / USART_Printf_Init) with direct register writes.

const RCC_APB2PCENR: *mut u32 = 0x4002_1018 as *mut u32;
const GPIOA_CFGHR: *mut u32 = 0x4001_0804 as *mut u32;
const GPIOB_CFGLR: *mut u32 = 0x4001_0C00 as *mut u32;
const GPIOB_BSHR: *mut u32 = 0x4001_0C10 as *mut u32;
const USART1_STATR: *const u32 = 0x4001_3800 as *const u32;
const USART1_DATAR: *mut u32 = 0x4001_3804 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_3808 as *mut u32;
const USART1_CTLR1: *mut u32 = 0x4001_380C as *mut u32;

global_asm!(
    r#"
    .section .text.entry
    .global _start
_start:
    la sp, __stack_top
    j  main
    "#
);

fn delay(n: u32) {
    for i in 0..n {
        unsafe { core::arch::asm!("", in("a0") i) };
    }
}

#[no_mangle]
extern "C" fn main() -> ! {
    unsafe {
        // IOPAEN (2), IOPBEN (3), USART1EN (14).
        write_volatile(RCC_APB2PCENR, (1 << 2) | (1 << 3) | (1 << 14));
        // PA9: alternate-function push-pull, 50 MHz; rest stay floating input.
        write_volatile(GPIOA_CFGHR, 0x4444_44B4);
        // PB2: general-purpose push-pull output, 50 MHz.
        write_volatile(GPIOB_CFGLR, 0x4444_4344);
        // 8 MHz HSI / 115200 baud = 4.3125 -> mantissa 4, fraction 5.
        write_volatile(USART1_BRR, 0x45);
        // UE | TE.
        write_volatile(USART1_CTLR1, (1 << 13) | (1 << 3));

        for &b in b"CH32V203_BLUEPILL_PLUS_OK\r\n" {
            while read_volatile(USART1_STATR) & (1 << 7) == 0 {}
            write_volatile(USART1_DATAR, b as u32);
        }

        loop {
            write_volatile(GPIOB_BSHR, 1 << 2);
            delay(1500);
            write_volatile(GPIOB_BSHR, 1 << (2 + 16));
            delay(1500);
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
