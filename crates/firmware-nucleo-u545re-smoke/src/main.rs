#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR1: *mut u32 = 0x4602_0C8C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4602_0CA4 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4202_0000 as *mut u32;
const GPIOA_AFRH: *mut u32 = 0x4202_0024 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4202_0018 as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_3800 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_380C as *mut u32;
const USART1_ISR: *const u32 = 0x4001_381C as *const u32;
const USART1_TDR: *mut u32 = 0x4001_3828 as *mut u32;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 1 << 5;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_ISR) & TXE == 0 {}
        core::ptr::write_volatile(USART1_TDR, byte as u32);
    }
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA and USART1 clocks before touching either peripheral.
        let ahb2enr1 = core::ptr::read_volatile(RCC_AHB2ENR1);
        core::ptr::write_volatile(RCC_AHB2ENR1, ahb2enr1 | 1);
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2enr | (1 << 14));

        // LD2 is active-high on PA5. The VCP's USART1 TX is PA9 (AF7).
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 10) | (0b11 << 18)))
            | (0b01 << 10)
            | (0b10 << 18);
        core::ptr::write_volatile(GPIOA_MODER, moder);
        let afrh = core::ptr::read_volatile(GPIOA_AFRH);
        core::ptr::write_volatile(GPIOA_AFRH, (afrh & !(0xF << 4)) | (7 << 4));
        core::ptr::write_volatile(GPIOA_BSRR, LED_PA5);

        // Reset MSI is 4 MHz; this is the USART v2 115200 baud divider.
        core::ptr::write_volatile(USART1_BRR, 35);
        core::ptr::write_volatile(USART1_CR1, (1 << 0) | (1 << 3)); // UE | TE
        for byte in b"NUCLEO_U545RE_OK\n" {
            putc(*byte);
        }
    }

    let mut iterations = 0u32;
    let mut led_high = true;
    loop {
        iterations += 1;
        if iterations == 64 {
            iterations = 0;
            led_high = !led_high;
            unsafe {
                core::ptr::write_volatile(
                    GPIOA_BSRR,
                    if led_high { LED_PA5 } else { LED_PA5 << 16 },
                );
            }
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
