#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;

const USART_TXE: u32 = 1 << 7;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA and USART2. The smoke leaves the reset HSI16 clock in
        // place, so BRR 139 selects 115200 baud for the VCP output.
        let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
        core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | 1);
        let apb1enr1 = core::ptr::read_volatile(RCC_APB1ENR1);
        core::ptr::write_volatile(RCC_APB1ENR1, apb1enr1 | (1 << 17));

        // The board routes the default ST-LINK VCP through USART2 PA2/PA3;
        // configure both pads for their G4 USART2 alternate function (AF7).
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(
            GPIOA_MODER,
            (moder & !((0b11 << 4) | (0b11 << 6))) | (0b10 << 4) | (0b10 << 6),
        );
        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        core::ptr::write_volatile(
            GPIOA_AFRL,
            (afrl & !((0xF << 8) | (0xF << 12))) | (7 << 8) | (7 << 12),
        );

        core::ptr::write_volatile(USART2_BRR, 139);
        core::ptr::write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE

        for byte in b"NUCLEO_G431KB_OK\n" {
            while core::ptr::read_volatile(USART2_ISR) & USART_TXE == 0 {}
            core::ptr::write_volatile(USART2_TDR, *byte as u32);
        }
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
