#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// nRF52840 UART0 Base Address
const UART0_BASE: u32 = 0x40002000;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_TXD: *mut u32 = (UART0_BASE + 0x51C) as *mut u32;

const GPIO0_BASE: u32 = 0x50000000;
const GPIO0_OUTSET: *mut u32 = (GPIO0_BASE + 0x508) as *mut u32;
const GPIO0_OUTCLR: *mut u32 = (GPIO0_BASE + 0x50C) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;
const LED_RED: u32 = 1 << 26;
const LED_GREEN: u32 = 1 << 30;
const LED_BLUE: u32 = 1 << 6;

const SPIM0_BASE: u32 = 0x40003000;
const SPIM0_TASKS_START: *mut u32 = (SPIM0_BASE + 0x010) as *mut u32;
const SPIM0_ENABLE: *mut u32 = (SPIM0_BASE + 0x500) as *mut u32;
const SPIM0_PSEL_SCK: *mut u32 = (SPIM0_BASE + 0x508) as *mut u32;
const SPIM0_PSEL_MOSI: *mut u32 = (SPIM0_BASE + 0x50C) as *mut u32;
const SPIM0_PSEL_MISO: *mut u32 = (SPIM0_BASE + 0x510) as *mut u32;
const SPIM0_FREQUENCY: *mut u32 = (SPIM0_BASE + 0x524) as *mut u32;
const SPIM0_TXD_PTR: *mut u32 = (SPIM0_BASE + 0x544) as *mut u32;
const SPIM0_TXD_MAXCNT: *mut u32 = (SPIM0_BASE + 0x548) as *mut u32;

static SPI_SMOKE_BYTES: [u8; 4] = [0x9A, 0xBC, 0xDE, 0xF0];

#[entry]
fn main() -> ! {
    unsafe {
        // Enable UART (value 4 = ENABLE)
        core::ptr::write_volatile(UART0_ENABLE, 4);
        configure_gpio();
        configure_spim0();
    }

    loop {
        unsafe {
            core::ptr::write_volatile(GPIO0_OUTCLR, LED_RED);
            core::ptr::write_volatile(GPIO0_OUTSET, LED_GREEN | LED_BLUE);
            core::ptr::write_volatile(SPIM0_TASKS_START, 1);
        }

        // Write a message to UART
        print_uart("NRF52840_SMOKE_OK\n");

        // Small delay
        for _ in 0..1000u32 {
            cortex_m::asm::nop();
        }
    }
}

unsafe fn configure_gpio() {
    let leds = LED_RED | LED_GREEN | LED_BLUE;
    core::ptr::write_volatile(GPIO0_DIRSET, leds);
    core::ptr::write_volatile(GPIO0_OUTSET, leds);
}

unsafe fn configure_spim0() {
    // XIAO SPI pins: D8/SCK=P1.13, D10/MOSI=P1.15, D9/MISO=P1.14.
    core::ptr::write_volatile(SPIM0_PSEL_SCK, 32 + 13);
    core::ptr::write_volatile(SPIM0_PSEL_MOSI, 32 + 15);
    core::ptr::write_volatile(SPIM0_PSEL_MISO, 32 + 14);
    core::ptr::write_volatile(SPIM0_FREQUENCY, 0x0200_0000);
    core::ptr::write_volatile(SPIM0_TXD_PTR, SPI_SMOKE_BYTES.as_ptr() as u32);
    core::ptr::write_volatile(SPIM0_TXD_MAXCNT, SPI_SMOKE_BYTES.len() as u32);
    core::ptr::write_volatile(SPIM0_ENABLE, 7);
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_TXD, b as u32);
        }
    }
}
