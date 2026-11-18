// Bare-metal NUCLEO-H533RE smoke: USART2 through ST-LINK VCP and LD2 on PA5.
// Register locations and board routes follow ST RM0481 and MB1814 C-02.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4402_0C00;
const RCC_AHB2ENR: *mut u32 = (RCC_BASE + 0x8C) as *mut u32;
const RCC_APB1LENR: *mut u32 = (RCC_BASE + 0x9C) as *mut u32;

const GPIOA_BASE: u32 = 0x4202_0000;
const GPIOA_MODER: *mut u32 = GPIOA_BASE as *mut u32;
const GPIOA_AFRL: *mut u32 = (GPIOA_BASE + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (GPIOA_BASE + 0x18) as *mut u32;

const USART2_BASE: u32 = 0x4000_4400;
const USART2_CR1: *mut u32 = USART2_BASE as *mut u32;
const USART2_BRR: *mut u32 = (USART2_BASE + 0x0C) as *mut u32;
const USART2_ISR: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const USART2_TDR: *mut u8 = (USART2_BASE + 0x28) as *mut u8;

const TXE: u32 = 1 << 7;
const LED_PA5: u32 = 5;

#[entry]
fn main() -> ! {
    unsafe {
        // H5 reset clock is HSI 64 MHz divided by two: enable GPIOA and USART2.
        write_volatile(RCC_AHB2ENR, read_volatile(RCC_AHB2ENR) | 1);
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 17));

        // PA5 / LD2 is an active-high push-pull output.
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(
            GPIOA_MODER,
            (moder & !(0b11 << (LED_PA5 * 2))) | (0b01 << (LED_PA5 * 2)),
        );

        // USART2 VCP is routed to PA2/PA3, alternate function 7.
        let moder = read_volatile(GPIOA_MODER);
        let uart_pins = (0b11 << (2 * 2)) | (0b11 << (3 * 2));
        write_volatile(
            GPIOA_MODER,
            (moder & !uart_pins) | (0b10 << (2 * 2)) | (0b10 << (3 * 2)),
        );
        let afrl = read_volatile(GPIOA_AFRL);
        write_volatile(GPIOA_AFRL, (afrl & !0x0000_FF00) | 0x0000_7700);

        // 32 MHz PCLK / 115200 baud, 8 data bits, no parity, one stop bit.
        write_volatile(USART2_BRR, 278);
        write_volatile(USART2_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }

    send(b"NUCLEO_H533RE_OK\n");
    loop {
        led_set(true);
        delay(10_000);
        led_set(false);
        delay(10_000);
    }
}

fn led_set(on: bool) {
    let value = if on {
        1 << LED_PA5
    } else {
        1 << (LED_PA5 + 16)
    };
    unsafe { write_volatile(GPIOA_BSRR, value) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            while read_volatile(USART2_ISR) & TXE == 0 {
                core::hint::spin_loop();
            }
            write_volatile(USART2_TDR, *byte);
        }
    }
}

fn delay(count: u32) {
    for _ in 0..count {
        core::hint::spin_loop();
    }
}
