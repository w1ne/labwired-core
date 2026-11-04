#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// The smoke does not enable IRQs. Provide the Cortex-M7 device-vector region
// explicitly so the same image links with cortex-m-rt's `device` feature.
#[repr(C)]
#[derive(Clone, Copy)]
pub union Vector {
    handler: unsafe extern "C" fn(),
    reserved: usize,
}

unsafe extern "C" fn unhandled_interrupt() {
    loop {
        core::hint::spin_loop();
    }
}

#[link_section = ".vector_table.interrupts"]
#[no_mangle]
pub static __INTERRUPTS: [Vector; 240] = [Vector {
    handler: unhandled_interrupt,
}; 240];

const RCC: u32 = 0x5802_4400;
const RCC_AHB4ENR: *mut u32 = (RCC + 0xE0) as *mut u32;
const RCC_APB1LENR: *mut u32 = (RCC + 0xE8) as *mut u32;

const GPIOC: u32 = 0x5802_0800;
const GPIOD: u32 = 0x5802_0C00;
const MODER: u32 = 0x00;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const AFRH: u32 = 0x24;

const USART3: u32 = 0x4000_4800;
const USART_CR1: *mut u32 = USART3 as *mut u32;
const USART_BRR: *mut u32 = (USART3 + 0x0C) as *mut u32;
const USART_ISR: *const u32 = (USART3 + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART3 + 0x28) as *mut u32;
const USART_ISR_TXE: u32 = 1 << 7;
const USART_CR1_UE_TE: u32 = (1 << 0) | (1 << 3);

const USER_LEDS_PC2_PC3: u32 = (1 << 2) | (1 << 3);

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let register = addr as *mut u32;
        write_volatile(register, (read_volatile(register) & !clear) | set);
    }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART_ISR) & USART_ISR_TXE == 0 {}
        write_volatile(USART_TDR, byte as u32);
    }
}

fn puts(text: &[u8]) {
    for &byte in text {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOC/GPIOD and USART3 on the H7 reset-clock tree.
        write_volatile(RCC_AHB4ENR, (1 << 2) | (1 << 3));
        write_volatile(RCC_APB1LENR, read_volatile(RCC_APB1LENR) | (1 << 18));

        // LD1 (PC3) and LD2 (PC2) are active-low user LEDs.
        reg_rmw(
            GPIOC + MODER,
            (0b11 << 4) | (0b11 << 6),
            (0b01 << 4) | (0b01 << 6),
        );
        // Drive both active-low LEDs low initially, which turns them on.
        write_volatile((GPIOC + BSRR) as *mut u32, USER_LEDS_PC2_PC3 << 16);

        // PD8/PD9 are USART3 TX/RX on AF7; the default core clock is 64 MHz.
        reg_rmw(GPIOD + MODER, (0b11 << 16) | (0b11 << 18), (0b10 << 16) | (0b10 << 18));
        reg_rmw(GPIOD + AFRH, 0xFF, 0x77);
        write_volatile(USART_BRR, 556);
        write_volatile(USART_CR1, USART_CR1_UE_TE);
    }

    puts(b"STM32H735G_DK_OK USART3=PD8 LD1=PC3 LD2=PC2\n");

    // Toggle both physical user LEDs together to make each pad observable.
    // The remote oracle can also observe the initial low state after the UART
    // marker has been printed.
    for _ in 0..32 {
        unsafe {
            let high = read_volatile((GPIOC + ODR) as *const u32) & (1 << 3) != 0;
            write_volatile(
                (GPIOC + BSRR) as *mut u32,
                if high {
                    USER_LEDS_PC2_PC3 << 16
                } else {
                    USER_LEDS_PC2_PC3
                },
            );
        }
        core::hint::spin_loop();
    }

    loop {
        core::hint::spin_loop();
    }
}
