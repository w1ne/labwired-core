#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_IOPENR: *mut u32 = 0x4002_1034 as *mut u32;
const RCC_APBENR1: *mut u32 = 0x4002_103C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x5000_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;
const LED_PIN: u32 = 5;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA on IOPENR and USART2 on APBENR1 (RM0444).
        let iopenr = core::ptr::read_volatile(RCC_IOPENR);
        core::ptr::write_volatile(RCC_IOPENR, iopenr | 1);
        let apbenr1 = core::ptr::read_volatile(RCC_APBENR1);
        core::ptr::write_volatile(RCC_APBENR1, apbenr1 | (1 << 17));

        // PA2/PA3: USART2 AF1. PA5: LD4 output, active high.
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
                | (0b0001 << (2 * 4))
                | (0b0001 << (3 * 4)),
        );

        // Reset HSI16 clock, 115200 baud: BRR = 16 MHz / 115200 ≈ 139.
        core::ptr::write_volatile(USART2_BRR, 139);
        core::ptr::write_volatile(USART2_CR1, (1 << 0) | (1 << 3));
        for byte in b"NUCLEO_G0B1RE_OK\n" {
            while core::ptr::read_volatile(USART2_ISR) & (1 << 7) == 0 {}
            core::ptr::write_volatile(USART2_TDR, *byte as u32);
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
