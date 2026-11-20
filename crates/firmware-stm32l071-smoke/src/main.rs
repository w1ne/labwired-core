#![no_std]
#![no_main]

//! STM32L071 smoke for the imported Renode platform; the checks live in
//! `firmware-stm32l071-smoke/src/l0_smoke.rs`, shared with the
//! STM32L072 smoke.

use cortex_m_rt::entry;
use panic_halt as _;

mod l0_smoke;

#[entry]
fn main() -> ! {
    l0_smoke::run(l0_smoke::Variant::L071)
}
