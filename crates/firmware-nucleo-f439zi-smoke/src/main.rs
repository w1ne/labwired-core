#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHB1ENR: *mut u32 = 0x4002_3830 as *mut u32;
const GPIOB_MODER: *mut u32 = 0x4002_0400 as *mut u32;
const GPIOB_BSRR: *mut u32 = 0x4002_0418 as *mut u32;
const LD2_PIN: u32 = 7;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable GPIOB on the reset HSI clock, then configure PB7 as output.
        let ahb1enr = core::ptr::read_volatile(RCC_AHB1ENR);
        core::ptr::write_volatile(RCC_AHB1ENR, ahb1enr | (1 << 1));

        let moder = core::ptr::read_volatile(GPIOB_MODER);
        core::ptr::write_volatile(
            GPIOB_MODER,
            (moder & !(0b11 << (LD2_PIN * 2))) | (0b01 << (LD2_PIN * 2)),
        );

        loop {
            // On UM1974/MB1137, LD2 is blue, connected to PB7, and active-high.
            core::ptr::write_volatile(GPIOB_BSRR, 1 << LD2_PIN);
            core::ptr::write_volatile(GPIOB_BSRR, 1 << (LD2_PIN + 16));
        }
    }
}
