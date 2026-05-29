// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT
//
// IRQ-driven blinky on the XIAO nRF52840 Sense pinout (LED_RED on
// P0.26). Configures TIMER0 to fire COMPARE interrupts, enables IRQ 8
// in the NVIC, then enters a WFI loop. The TIMER0 ISR clears the event,
// toggles the LED via GPIO0.OUT writes, and increments a counter in RAM
// the test can read out.
//
// Used to prove that:
//   - Vector table at flash base is consumed correctly.
//   - NVIC ISER0 writes enable the IRQ.
//   - TIMER0 raising irq:true ends up pending the configured exception.
//   - CortexM dispatches the exception through VTOR to our handler.
//   - WFI is non-blocking (decoded as NOP in the sim, so the main loop
//     continues stepping, ticking peripherals, until the IRQ fires).

#![no_std]
#![no_main]

use core::ptr::write_volatile;
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

const GPIO0_BASE: usize = 0x5000_0000;
const GPIO0_OUT: *mut u32 = (GPIO0_BASE + 0x504) as *mut u32;
const GPIO0_DIRSET: *mut u32 = (GPIO0_BASE + 0x518) as *mut u32;

const TIMER0_BASE: usize = 0x4000_8000;
const TIMER0_TASKS_START: *mut u32 = TIMER0_BASE as *mut u32;
const TIMER0_TASKS_CLEAR: *mut u32 = (TIMER0_BASE + 0x00C) as *mut u32;
const TIMER0_EVENTS_COMPARE0: *mut u32 = (TIMER0_BASE + 0x140) as *mut u32;
const TIMER0_SHORTS: *mut u32 = (TIMER0_BASE + 0x200) as *mut u32;
const TIMER0_INTENSET: *mut u32 = (TIMER0_BASE + 0x304) as *mut u32;
const TIMER0_BITMODE: *mut u32 = (TIMER0_BASE + 0x508) as *mut u32;
const TIMER0_PRESCALER: *mut u32 = (TIMER0_BASE + 0x510) as *mut u32;
const TIMER0_CC0: *mut u32 = (TIMER0_BASE + 0x540) as *mut u32;

const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;

const LED_RED_BIT: u32 = 1 << 26;

const TIMER0_IRQ: i16 = 8;

/// Counter incremented by the ISR; the integration test reads this to
/// confirm the handler actually ran.
#[no_mangle]
static ISR_COUNT: AtomicU32 = AtomicU32::new(0);

/// Current LED level (0 or LED_RED_BIT); toggled by the ISR.
static LED_LEVEL: AtomicU32 = AtomicU32::new(0);

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(GPIO0_DIRSET, LED_RED_BIT);

        write_volatile(TIMER0_BITMODE, 3);
        write_volatile(TIMER0_PRESCALER, 0);
        write_volatile(TIMER0_CC0, 8);
        write_volatile(TIMER0_SHORTS, 1);
        // INTENSET.COMPARE[0] = bit 16
        write_volatile(TIMER0_INTENSET, 1 << 16);

        // Enable TIMER0 IRQ (NVIC IRQ 8) so the exception actually fires.
        write_volatile(NVIC_ISER0, 1 << TIMER0_IRQ);

        write_volatile(TIMER0_TASKS_CLEAR, 1);
        write_volatile(TIMER0_TASKS_START, 1);
    }

    loop {
        cortex_m::asm::wfi();
    }
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == TIMER0_IRQ {
        unsafe { write_volatile(TIMER0_EVENTS_COMPARE0, 0) };
        let next = LED_LEVEL.load(Ordering::Relaxed) ^ LED_RED_BIT;
        LED_LEVEL.store(next, Ordering::Relaxed);
        unsafe { write_volatile(GPIO0_OUT, next) };
        ISR_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}
