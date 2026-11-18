// STM32F103C8T6 Blue Pill board smoke: PC13 LED and USART1 header output.
// The board LED is active-low. USART1 TX/RX are PA9/PA10; a physical serial
// adapter connects PA9 to RX and shares ground. The board has no USB-UART bridge.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_APB2ENR: *mut u32 = 0x4002_1018 as *mut u32;

const GPIOA_CRH: *mut u32 = 0x4001_0804 as *mut u32;
const GPIOC_CRH: *mut u32 = 0x4001_1004 as *mut u32;
const GPIOC_BSRR: *mut u32 = 0x4001_1010 as *mut u32;

const USART1_SR: *const u32 = 0x4001_3800 as *const u32;
const USART1_DR: *mut u32 = 0x4001_3804 as *mut u32;
const USART1_BRR: *mut u32 = 0x4001_3808 as *mut u32;
const USART1_CR1: *mut u32 = 0x4001_380C as *mut u32;

const PC13: u32 = 13;
const TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Reset HSI is 8 MHz. Enable GPIOA, GPIOC, and USART1 on APB2.
        write_volatile(
            RCC_APB2ENR,
            read_volatile(RCC_APB2ENR) | (1 << 2) | (1 << 4) | (1 << 14),
        );

        // PA9 USART1_TX: AF push-pull, 50 MHz; PA10 USART1_RX: floating input.
        let mut crh_a = read_volatile(GPIOA_CRH);
        crh_a = (crh_a & !((0xF << 4) | (0xF << 8))) | (0xB << 4) | (0x4 << 8);
        write_volatile(GPIOA_CRH, crh_a);

        // PC13 is the Blue Pill's active-low user LED, push-pull output.
        let crh_c = read_volatile(GPIOC_CRH);
        write_volatile(GPIOC_CRH, (crh_c & !(0xF << 20)) | (0x2 << 20));

        // USART1 baud: 8 MHz / 115200 ≈ 0x45. Enable USART and transmitter.
        write_volatile(USART1_BRR, 0x45);
        write_volatile(USART1_CR1, (1 << 13) | (1 << 3));
    }

    send(b"BLUEPILL_F103C8_OK\n");

    loop {
        led_set(true);
        delay(2_000);
        led_set(false);
        delay(2_000);
    }
}

fn led_set(on: bool) {
    // The LED is tied to 3V3 through a resistor: low sinks current and lights it.
    let bits = if on { 1 << (PC13 + 16) } else { 1 << PC13 };
    unsafe { write_volatile(GPIOC_BSRR, bits) };
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 20_000;
            while read_volatile(USART1_SR) & TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART1_DR, *byte as u32);
        }
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}
