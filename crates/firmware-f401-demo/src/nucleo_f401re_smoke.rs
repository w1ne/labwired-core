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

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        let ahb1enr = core::ptr::read_volatile(RCC_AHB1ENR);
        core::ptr::write_volatile(RCC_AHB1ENR, ahb1enr | 1);
        let apb1enr = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1enr | (1 << 17));

        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << (2 * 2)) | (0b11 << (3 * 2)) | (0b11 << (LED_PIN * 2))))
                | (0b10 << (2 * 2))
                | (0b10 << (3 * 2))
                | (0b01 << (LED_PIN * 2)),
        );
        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        core::ptr::write_volatile(
            GPIOA_AFRL,
            (afrl & !((0b1111 << (2 * 4)) | (0b1111 << (3 * 4))))
                | (0b0111 << (2 * 4))
                | (0b0111 << (3 * 4)),
        );

        // The board starts from the 16 MHz HSI clock; BRR 0x8B selects 115200 baud.
        core::ptr::write_volatile(USART2_BRR, 0x008B);
        core::ptr::write_volatile(USART2_CR1, (1 << 13) | (1 << 3) | (1 << 2));

        for byte in b"NUCLEO_F401RE_OK\n" {
            while core::ptr::read_volatile(USART2_SR) & (1 << 7) == 0 {}
            core::ptr::write_volatile(USART2_DR, *byte);
        }

        loop {
            core::ptr::write_volatile(GPIOA_BSRR, 1 << LED_PIN);
            core::ptr::write_volatile(GPIOA_BSRR, 1 << (LED_PIN + 16));
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
