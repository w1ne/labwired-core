#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB1ENR2: *mut u32 = 0x4002_105C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = 0x4800_0020 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4800_0018 as *mut u32;
const LPUART1_CR1: *mut u32 = 0x4000_8000 as *mut u32;
const LPUART1_BRR: *mut u32 = 0x4000_800C as *mut u32;
const LPUART1_ISR: *const u32 = 0x4000_801C as *const u32;
const LPUART1_TDR: *mut u32 = 0x4000_8028 as *mut u32;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 1 << 5;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn putc(byte: u8) {
    unsafe {
        while core::ptr::read_volatile(LPUART1_ISR) & TXE == 0 {}
        core::ptr::write_volatile(LPUART1_TDR, byte as u32);
    }
}

fn main() -> ! {
    unsafe {
        // Enable GPIOA and the board's default VCP peripheral, LPUART1.
        let ahb2enr = core::ptr::read_volatile(RCC_AHB2ENR);
        core::ptr::write_volatile(RCC_AHB2ENR, ahb2enr | 1);
        let apb1enr2 = core::ptr::read_volatile(RCC_APB1ENR2);
        core::ptr::write_volatile(RCC_APB1ENR2, apb1enr2 | 1);

        // LD2 is PA5, active high. PA2/PA3 are LPUART1 TX/RX (AF12).
        let moder = core::ptr::read_volatile(GPIOA_MODER);
        let moder = (moder & !((0b11 << 4) | (0b11 << 6) | (0b11 << 10)))
            | (0b10 << 4)
            | (0b10 << 6)
            | (0b01 << 10);
        core::ptr::write_volatile(GPIOA_MODER, moder);

        let afrl = core::ptr::read_volatile(GPIOA_AFRL);
        let afrl = (afrl & !((0xF << 8) | (0xF << 12))) | (12 << 8) | (12 << 12);
        core::ptr::write_volatile(GPIOA_AFRL, afrl);

        // The reset clock is HSI16; LPUART BRR uses the 256x baud ratio.
        core::ptr::write_volatile(LPUART1_BRR, 35_556);
        core::ptr::write_volatile(LPUART1_CR1, (1 << 0) | (1 << 3)); // UE | TE

        for byte in b"NUCLEO_G431RB_OK\n" {
            putc(*byte);
        }
    }

    let mut led_high = false;
    loop {
        led_high = !led_high;
        unsafe {
            core::ptr::write_volatile(
                GPIOA_BSRR,
                if led_high { LED_PA5 } else { LED_PA5 << 16 },
            );
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
