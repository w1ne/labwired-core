#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::ptr::{read_volatile, write_volatile};

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR2: *mut u32 = 0x4002_105C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4800_0018 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const LPUART1_CR1: *mut u32 = 0x4000_8000 as *mut u32;
const LPUART1_BRR: *mut u32 = 0x4000_800C as *mut u32;
const LPUART1_ISR: *const u32 = 0x4000_801C as *const u32;
const LPUART1_TDR: *mut u8 = 0x4000_8028 as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        let ahb2enr = read_volatile(RCC_AHB2ENR);
        write_volatile(RCC_AHB2ENR, ahb2enr | 1); // GPIOAEN
        let apb1enr2 = read_volatile(RCC_APB1ENR2);
        write_volatile(RCC_APB1ENR2, apb1enr2 | 1); // LPUART1EN

        // Default ST-LINK VCP uses LPUART1 on PA2/PA3.
        let mut moder = read_volatile(GPIOA_MODER);
        moder &= !((0b11 << 4) | (0b11 << 6) | (0b11 << 10));
        moder |= (0b10 << 4) | (0b10 << 6) | (0b01 << 10); // PA2/3 AF, PA5 output
        write_volatile(GPIOA_MODER, moder);

        let mut afrl = read_volatile(GPIOA_AFRL);
        afrl &= !((0xF << 8) | (0xF << 12));
        afrl |= (0xC << 8) | (0xC << 12); // PA2/PA3 AF12 = LPUART1
        write_volatile(GPIOA_AFRL, afrl);

        // LD2 is active high on PA5. Observe a real low→high→low→high path.
        write_volatile(GPIOA_BSRR, 1 << 21); // reset PA5
        write_volatile(GPIOA_BSRR, 1 << 5); // set PA5
        write_volatile(GPIOA_BSRR, 1 << 21); // reset PA5
        write_volatile(GPIOA_BSRR, 1 << 5); // set PA5

        // At the reset HSI16 clock, BRR = 256 * 16 MHz / 115200.
        write_volatile(LPUART1_BRR, 35_556);
        write_volatile(LPUART1_CR1, (1 << 0) | (1 << 3)); // UE | TE
        for byte in b"NUCLEO_G431RB_LPUART1_OK\r\n" {
            while read_volatile(LPUART1_ISR) & (1 << 7) == 0 {}
            write_volatile(LPUART1_TDR, *byte);
        }
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
