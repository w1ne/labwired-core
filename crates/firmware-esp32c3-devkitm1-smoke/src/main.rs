#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! Drive the ESP32-C3-DevKitM-1-N4X's onboard RGB LED through RMT TX0.
//!
//! The firmware writes one GRB WS2812 frame to C3 RMT symbol RAM, routes
//! RMT_SIG_OUT0 to GPIO8, and waits for the modeled CH0_TX_END interrupt flag.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = 0x6000_4574 as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x6000_9024 as *mut u32;
const GPIO8_MASK: u32 = 1 << 8;

const RMT_TX_CONF0: *mut u32 = 0x6001_6010 as *mut u32;
const RMT_INT_RAW: *const u32 = 0x6001_6038 as *const u32;
const RMT_MEM: *mut u32 = 0x6001_6400 as *mut u32;
const RMT_SIG_OUT0_IDX: u32 = 51;
const RMT_TX_END_CH0: u32 = 1;

// C3 RMT uses the 80 MHz APB clock. At divider 1, 20 ticks = 250 ns and
// 48 ticks = 600 ns, on opposite sides of the NeoPixel model's 500 ns bit
// threshold. Both halves total 95 ticks (1.1875 us), within the 1.25 us cell.
const SYMBOL_ZERO: u32 = 20 | (1 << 15) | (75 << 16);
const SYMBOL_ONE: u32 = 48 | (1 << 15) | (47 << 16);
const DIV_CNT_1_IDLE_LOW: u32 = (1 << 8) | (1 << 6);

#[inline(always)]
fn reset_gap() {
    // 16,000 CPU cycles = 100 us at 160 MHz, past the LED's 80 us reset gap.
    unsafe { core::arch::asm!(".rept 16000\n nop\n .endr", options(nomem, nostack)) }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Select the GPIO function on GPIO8, route RMT TX0 to the pad, and
        // force the output enable from GPIO_ENABLE.
        let gpio8_mux = core::ptr::read_volatile(IO_MUX_GPIO8);
        core::ptr::write_volatile(IO_MUX_GPIO8, gpio8_mux & !(0b111 << 12));
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, RMT_SIG_OUT0_IDX | (1 << 10));
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);

        // The onboard SK68XXMINI-HS consumes a 24-bit GRB frame. 00 FF 00 is red.
        let mut symbol = 0;
        for byte in [0x00u8, 0xFF, 0x00] {
            for bit in (0..8).rev() {
                let one = (byte & (1 << bit)) != 0;
                core::ptr::write_volatile(
                    RMT_MEM.add(symbol),
                    if one { SYMBOL_ONE } else { SYMBOL_ZERO },
                );
                symbol += 1;
            }
        }

        core::ptr::write_volatile(RMT_TX_CONF0, DIV_CNT_1_IDLE_LOW | 1);
        while core::ptr::read_volatile(RMT_INT_RAW) & RMT_TX_END_CH0 == 0 {
            // The write above starts TX; completion is set only after the
            // model has replayed all 24 timed symbols.
            core::arch::asm!("nop", options(nomem, nostack));
        }
        reset_gap();
    }

    loop {}
}
