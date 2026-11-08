#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4002_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4002_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4002_0018 as *mut u32;
const USART2_SR: *const u32 = 0x4000_4400 as *const u32;
const USART2_DR: *mut u8 = 0x4000_4404 as *mut u8;
const USART2_BRR: *mut u32 = 0x4000_4408 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_440C as *mut u32;
const LED_PIN: u32 = 5;

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
        // Enable GPIOA/GPIOC and USART2 on the reset HSI clock.
        core::ptr::write_volatile(
            RCC_AHB1ENR,
            core::ptr::read_volatile(RCC_AHB1ENR) | (1 << 0) | (1 << 2),
        );
        core::ptr::write_volatile(
            RCC_APB1ENR,
            core::ptr::read_volatile(RCC_APB1ENR) | (1 << 17),
        );

        // PA2/PA3 use USART2 AF7; PA5 drives the on-board LD2 LED.
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << 10)))
                | (0b10 << 4)
                | (0b10 << 6)
                | (0b01 << 10),
        );
        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        core::ptr::write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (7 << 8) | (7 << 12),
        );

        // USART2 at 115200 8N1 from the 16 MHz HSI reset clock.
        core::ptr::write_volatile(USART2_BRR, 0x008B);
        core::ptr::write_volatile(USART2_CR1, (1 << 13) | (1 << 3) | (1 << 2));
        for byte in b"NUCLEO_F446RE_OK\n" {
            while core::ptr::read_volatile(USART2_SR) & (1 << 7) == 0 {}
            core::ptr::write_volatile(USART2_DR, *byte);
        }

        loop {
            core::ptr::write_volatile(GPIOA_BSRR, 1 << LED_PIN);
            short_delay();
            core::ptr::write_volatile(GPIOA_BSRR, 1 << (LED_PIN + 16));
            short_delay();
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
