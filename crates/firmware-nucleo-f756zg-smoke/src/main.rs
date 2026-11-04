#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const RCC_APB1ENR: *mut u32 = 0x4002_3840 as *mut u32;

const GPIOB: u32 = 0x4002_0400;
const GPIOD: u32 = 0x4002_0C00;
const GPIO_MODER: u32 = 0x00;
const GPIO_BSRR: u32 = 0x18;
const GPIO_AFRH: u32 = 0x24;

const USART3: u32 = 0x4000_4800;
const USART_CR1: *mut u32 = USART3 as *mut u32;
const USART_BRR: *mut u32 = (USART3 + 0x0C) as *mut u32;
const USART_ISR: *const u32 = (USART3 + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART3 + 0x28) as *mut u32;

const USART_UE_TE: u32 = (1 << 0) | (1 << 3);
const USART_TXE: u32 = 1 << 7;
const LD2_PB7: u32 = 1 << 7;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_ISR) & USART_TXE == 0 {}
        write_volatile(USART_TDR, byte as u32);
    }
}

fn puts(message: &[u8]) {
    for &byte in message {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB/GPIOD and the APB1 USART3 peripheral clock.
        write_volatile(RCC_AHB1ENR, (1 << 1) | (1 << 3));
        write_volatile(RCC_APB1ENR, (1 << 18));

        // LD2 on PB7 as a push-pull output.
        let moder = (GPIOB + GPIO_MODER) as *mut u32;
        let moder_value = read_volatile(moder);
        write_volatile(moder, (moder_value & !(0b11 << 14)) | (0b01 << 14));

        // USART3 TX/RX use PD8/PD9 AF7 for the ST-LINK VCP connection.
        let moder = (GPIOD + GPIO_MODER) as *mut u32;
        let moder_value = read_volatile(moder);
        write_volatile(
            moder,
            (moder_value & !((0b11 << 16) | (0b11 << 18)))
                | (0b10 << 16)
                | (0b10 << 18),
        );
        let afrh = (GPIOD + GPIO_AFRH) as *mut u32;
        let afrh_value = read_volatile(afrh);
        write_volatile(afrh, (afrh_value & !0xFF) | 0x77);

        // The reset HSI clock is 16 MHz; BRR 139 is approximately 115200 baud.
        write_volatile(USART_BRR, 139);
        write_volatile(USART_CR1, USART_UE_TE);
    }

    puts(b"NUCLEO_F756ZG_OK USART3=PD8 LD2=PB7\n");

    // Exercise both levels so the board LED signal is observable in the smoke.
    for _ in 0..32 {
        unsafe {
            write_volatile((GPIOB + GPIO_BSRR) as *mut u32, LD2_PB7);
            write_volatile((GPIOB + GPIO_BSRR) as *mut u32, LD2_PB7 << 16);
        }
        for _ in 0..32 {
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
