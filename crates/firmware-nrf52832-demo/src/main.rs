#![no_std]
#![no_main]

//! nRF52832 smoke firmware for the `nrf52832_demo` survival fixture.
//!
//! This exists because `tests/fixtures/nrf52832-demo.elf` was, for its whole
//! life, a build of `crates/firmware-nrf52840-demo` — the mangled symbols in
//! it read `_ZN22firmware_nrf52840_demo…`, its debug info points at
//! `crates/firmware-nrf52840-demo/src/main.rs`, and it prints
//! `NRF52840_SMOKE_OK`. Its vector table therefore carries an nRF52840 initial
//! stack pointer of `0x20040000` (top of 256 KB), while real nRF52832 silicon
//! has 64 KB of RAM ending at `0x20010000` — which is exactly what
//! `configs/chips/nrf52832.yaml` models. Every push went to unmapped memory
//! and was silently discarded; the survival case had to give up on asserting
//! UART output entirely and said so in a comment.
//!
//! The chip config was right and the fixture was wrong, so this is the side
//! that changes. Growing the model's RAM to 256 KB to match the mis-targeted
//! binary would have made the twin lie about a real part.
//!
//! Kept deliberately close to the nRF52840 demo, minus what nRF52832 does not
//! have:
//!   * a single GPIO port (P0); there is no P1, so no `32 + n` pin numbers.
//!   * no SPIM exercised here. On nRF52832 SPI0/TWI0 share `0x40003000`, and
//!     `configs/chips/nrf52832.yaml` models that address as the TWI (I2C)
//!     peripheral. Driving SPIM registers into an I2C model would be
//!     make-believe, so this firmware sticks to what the chip config actually
//!     models.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// UARTE0 (nRF52832 PS §35). Same EasyDMA console as the nRF52840: point the
// DMA at a RAM buffer, trigger STARTTX, wait for ENDTX. There is no legacy
// byte-at-a-time TXD register in this personality (ENABLE=8).
const UART0_BASE: u32 = 0x40002000;
const UART0_TASKS_STARTTX: *mut u32 = (UART0_BASE + 0x008) as *mut u32;
const UART0_EVENTS_ENDTX: *mut u32 = (UART0_BASE + 0x120) as *mut u32;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_TXD_PTR: *mut u32 = (UART0_BASE + 0x544) as *mut u32;
const UART0_TXD_MAXCNT: *mut u32 = (UART0_BASE + 0x548) as *mut u32;
const UARTE_ENABLE: u32 = 8;

// GPIO P0 (nRF52832 PS §21). nRF52-DK LED1..LED4 are P0.17..P0.20, active low.
const GPIO0_BASE: u32 = 0x50000000;
const GPIO0_OUTSET: *mut u32 = (GPIO0_BASE + 0x508) as *mut u32;
const GPIO0_OUTCLR: *mut u32 = (GPIO0_BASE + 0x50C) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;
const LED1: u32 = 1 << 17;
const LED2: u32 = 1 << 18;
const LED3: u32 = 1 << 19;
const LED4: u32 = 1 << 20;
const ALL_LEDS: u32 = LED1 | LED2 | LED3 | LED4;

#[entry]
fn main() -> ! {
    // The banner lives on the stack (RAM) so EasyDMA can fetch it — the DMA
    // engine cannot read from flash on real silicon. With the 64 KB memory.x
    // the stack is inside the modelled RAM, so this buffer is actually
    // readable; under the old nRF52840-targeted fixture it was not.
    let msg = *b"NRF52832_SMOKE_OK\n";

    unsafe {
        write_volatile(UART0_ENABLE, UARTE_ENABLE);
        configure_gpio();
    }

    loop {
        unsafe {
            // Active-low DK LEDs: clear LED1 to light it, set the rest.
            write_volatile(GPIO0_OUTCLR, LED1);
            write_volatile(GPIO0_OUTSET, LED2 | LED3 | LED4);

            // EasyDMA console TX.
            write_volatile(UART0_EVENTS_ENDTX, 0);
            write_volatile(UART0_TXD_PTR, msg.as_ptr() as u32);
            write_volatile(UART0_TXD_MAXCNT, msg.len() as u32);
            write_volatile(UART0_TASKS_STARTTX, 1);
            while read_volatile(UART0_EVENTS_ENDTX) == 0 {}
        }

        // Idle a little. A volatile read keeps the loop without a cortex-m
        // intrinsic (cortex_m::asm::nop's __nop is unavailable in this build).
        for _ in 0..1000u32 {
            unsafe {
                let _ = read_volatile(UART0_ENABLE);
            }
        }
    }
}

unsafe fn configure_gpio() {
    write_volatile(GPIO0_DIRSET, ALL_LEDS);
    write_volatile(GPIO0_OUTSET, ALL_LEDS);
}
