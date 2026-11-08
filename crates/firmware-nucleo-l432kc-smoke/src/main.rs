#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR1: *mut u32 = 0x4002_1058 as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4800_0418 as *mut u32;

const USART2_CR1: *mut u32 = 0x4000_4400 as *mut u32;
const USART2_BRR: *mut u32 = 0x4000_440C as *mut u32;
const USART2_ISR: *const u32 = 0x4000_441C as *const u32;
const USART2_TDR: *mut u32 = 0x4000_4428 as *mut u32;

const USART_ISR_TXE: u32 = 1 << 7;
const LED_LD3_PB3: u32 = 1 << 3;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(USART2_ISR) & USART_ISR_TXE == 0 {}
        core::ptr::write_volatile(USART2_TDR, byte as u32);
    }
}

fn main() -> ! {
    unsafe {
        // Enable the GPIOA/GPIOB buses and USART2 before touching their registers.
        let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
        core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | 0b11);
        let apb1enr1 = core::ptr::read_volatile(RCC_APB1ENR1);
        core::ptr::write_volatile(RCC_APB1ENR1, apb1enr1 | (1 << 17));

        // PA2 is the default ST-LINK VCP TX. USART2_TX uses alternate function 7.
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        core::ptr::write_volatile(GPIOA_MODER, (moder & !(0b11 << 4)) | (0b10 << 4));
        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        core::ptr::write_volatile(GPIOA_AFRL, (afrl & !(0xF << 8)) | (7 << 8));

        // LD3 is active-high on PB3; configure its pin as a push-pull output.
        let moder = core::ptr::read_volatile(GPIOB_MODER);
        core::ptr::write_volatile(GPIOB_MODER, (moder & !(0b11 << 6)) | (0b01 << 6));

        // The board uses the 4 MHz MSI reset clock; BRR 35 is 115200 baud.
        core::ptr::write_volatile(USART2_BRR, 35);
        core::ptr::write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE

        for byte in b"NUCLEO_L432KC_OK\n" {
            putc(*byte);
        }
    }

    let mut led_high = false;
    loop {
        led_high = !led_high;
        unsafe {
            core::ptr::write_volatile(
                GPIOB_BSRR,
                if led_high {
                    LED_LD3_PB3
                } else {
                    LED_LD3_PB3 << 16
                },
            );
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
