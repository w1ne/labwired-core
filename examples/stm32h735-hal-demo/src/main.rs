//! STM32H735 demo using the real community HAL (stm32h7xx-hal).
//!
//! This exercises the FULL H7 HAL bring-up that the raw-register tier-1 fixture
//! skips: PWR voltage scaling (VOSRDY), FLASH ACR latency, the PLL1 config +
//! PLL1RDY wait, and the SYSCLK switch (SW->SWS) — then GPIO + USART3. Each
//! stage prints a marker over USART3 so we can see exactly how far the sim gets.

#![no_std]
#![no_main]

use core::fmt::Write;
use cortex_m_rt::entry;
use panic_halt as _;
use stm32h7xx_hal::{pac, prelude::*};

// Raw USART3 TDR/ISR so we can emit a boot marker BEFORE the HAL serial is up
// (USART3 @ 0x4000_4800, stm32v2 layout: ISR 0x1C TXE bit7, TDR 0x28).
const USART3: u32 = 0x4000_4800;
fn raw_puts(s: &[u8]) {
    for &b in s {
        for _ in 0..10_000 {
            if unsafe { core::ptr::read_volatile((USART3 + 0x1C) as *const u32) } & (1 << 7) != 0 {
                break;
            }
        }
        unsafe { core::ptr::write_volatile((USART3 + 0x28) as *mut u8, b) };
    }
}

#[entry]
fn main() -> ! {
    raw_puts(b"HAL boot: entry\n");

    let dp = pac::Peripherals::take().unwrap();

    // 1) PWR voltage scaling — HAL polls PWR for VOSRDY-equivalent readiness.
    let pwr = dp.PWR.constrain();
    let pwrcfg = pwr.freeze();
    raw_puts(b"HAL: pwr.freeze ok\n");

    // 2) RCC: configure PLL1 -> sys_ck, set flash latency, switch SYSCLK.
    let rcc = dp.RCC.constrain();
    let ccdr = rcc.sys_ck(200.MHz()).freeze(pwrcfg, &dp.SYSCFG);
    raw_puts(b"HAL: rcc.freeze ok (clocks up)\n");

    // 3) GPIO: LED on PA5.
    let gpioa = dp.GPIOA.split(ccdr.peripheral.GPIOA);
    let mut led = gpioa.pa5.into_push_pull_output();
    led.set_high();
    raw_puts(b"HAL: gpio ok\n");

    // 4) USART3 via the HAL (PD8 TX / PD9 RX, AF7).
    let gpiod = dp.GPIOD.split(ccdr.peripheral.GPIOD);
    let tx = gpiod.pd8.into_alternate();
    let rx = gpiod.pd9.into_alternate();
    let mut serial = dp
        .USART3
        .serial((tx, rx), 115_200.bps(), ccdr.peripheral.USART3, &ccdr.clocks)
        .unwrap();
    writeln!(serial, "HAL: USART3 up @ {} Hz sysclk", ccdr.clocks.sys_ck().raw()).ok();
    raw_puts(b"HAL: DONE\n");

    let mut n = 0u32;
    loop {
        led.toggle();
        n = n.wrapping_add(1);
        writeln!(serial, "tick {}", n).ok();
        for _ in 0..200_000 {
            cortex_m::asm::nop();
        }
    }
}
