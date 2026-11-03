#![no_std]
#![no_main]

use core::arch::asm;
use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// STM32L151 USART1 uses the classic STM32 USART register layout.
const USART1_BASE: usize = 0x4001_3800;
const USART1_SR: *const u32 = USART1_BASE as *const u32;
const USART1_DR: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_BRR: *mut u32 = (USART1_BASE + 0x08) as *mut u32;
const USART1_CR1: *mut u32 = (USART1_BASE + 0x0C) as *mut u32;
const USART_SR_TXE: u32 = 1 << 7;

#[entry]
fn main() -> ! {
    unsafe {
        // The smoke checks the modeled UART register path, not clock-tree or
        // pad routing. The imported profile intentionally omits those blocks.
        write_volatile(USART1_BRR, 0x45);
        write_volatile(USART1_CR1, (1 << 13) | (1 << 3)); // UE | TE
    }

    send(b"STM32L151 Cortex-M3 smoke\r\n");
    loop {
        unsafe { asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

fn send(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            let mut remaining = 100_000;
            while core::ptr::read_volatile(USART1_SR) & USART_SR_TXE == 0 && remaining > 0 {
                remaining -= 1;
            }
            write_volatile(USART1_DR, *byte as u32);
        }
    }
}
