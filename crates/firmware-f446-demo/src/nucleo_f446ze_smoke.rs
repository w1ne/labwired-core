#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4002_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4002_0418 as *mut u32;
const GPIOD_MODER: *mut u32 = 0x4002_0C00 as *mut u32;
const GPIOD_AFRH: *mut u32 = 0x4002_0C24 as *mut u32;
const USART3_SR: *const u32 = 0x4000_4800 as *const u32;
const USART3_DR: *mut u8 = 0x4000_4804 as *mut u8;
const USART3_BRR: *mut u32 = 0x4000_4808 as *mut u32;
const USART3_CR1: *mut u32 = 0x4000_480C as *mut u32;

const LED_MASK: u32 = (1 << 0) | (1 << 7) | (1 << 14);
const LED_MODE_MASK: u32 = (0b11 << 0) | (0b11 << 14) | (0b11 << 28);
const LED_OUTPUT_MODE: u32 = (0b01 << 0) | (0b01 << 14) | (0b01 << 28);

#[inline(never)]
fn short_delay() {
    for _ in 0..192 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) }
    }
}

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        // Enable GPIOB/GPIOD and USART3 on the reset HSI clock.
        core::ptr::write_volatile(
            RCC_AHB1ENR,
            core::ptr::read_volatile(RCC_AHB1ENR) | (1 << 1) | (1 << 3),
        );
        core::ptr::write_volatile(
            RCC_APB1ENR,
            core::ptr::read_volatile(RCC_APB1ENR) | (1 << 18),
        );

        // LD1/PB0, LD2/PB7, and LD3/PB14 are active-high user LEDs.
        let moder = core::ptr::read_volatile(GPIOB_MODER);
        core::ptr::write_volatile(GPIOB_MODER, (moder & !LED_MODE_MASK) | LED_OUTPUT_MODE);
        core::ptr::write_volatile(GPIOB_BSRR, LED_MASK << 16);

        // The default ST-LINK VCP route is USART3 on PD8/PD9, alternate function 7.
        let moder = core::ptr::read_volatile(GPIOD_MODER);
        core::ptr::write_volatile(
            GPIOD_MODER,
            (moder & !((0b11 << 16) | (0b11 << 18))) | (0b10 << 16) | (0b10 << 18),
        );
        let afrh = core::ptr::read_volatile(GPIOD_AFRH);
        core::ptr::write_volatile(
            GPIOD_AFRH,
            (afrh & !0xFF) | (7 << 0) | (7 << 4),
        );

        // USART3 at 115200 8N1 from the 16 MHz HSI reset clock.
        core::ptr::write_volatile(USART3_BRR, 0x008B);
        core::ptr::write_volatile(USART3_CR1, (1 << 13) | (1 << 3) | (1 << 2));
        for byte in b"NUCLEO_F446ZE_OK\n" {
            while core::ptr::read_volatile(USART3_SR) & (1 << 7) == 0 {}
            core::ptr::write_volatile(USART3_DR, *byte);
        }

        loop {
            core::ptr::write_volatile(GPIOB_BSRR, LED_MASK);
            short_delay();
            core::ptr::write_volatile(GPIOB_BSRR, LED_MASK << 16);
            short_delay();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
