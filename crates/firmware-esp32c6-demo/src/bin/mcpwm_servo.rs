#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C6-DevKitC-1 v1.2 MCPWM0 servo output smoke.
//!
//! Runs timer 0 at 50 Hz, routes MCPWM0 PWM0A (matrix signal 87) to header
//! GPIO4, and programs a 1.5 ms high pulse for a midpoint SG90 command.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;
const IO_MUX_GPIO16: *mut u32 = 0x6009_0044 as *mut u32;

const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1024 as *mut u32;
const GPIO_FUNC4_OUT_SEL_CFG: *mut u32 = 0x6009_1564 as *mut u32;
const GPIO4_MASK: u32 = 1 << 4;
const MCPWM0_CLK_CFG: *mut u32 = 0x6001_4000 as *mut u32;
const MCPWM0_TIMER0_CFG0: *mut u32 = 0x6001_4004 as *mut u32;
const MCPWM0_TIMER0_CFG1: *mut u32 = 0x6001_4008 as *mut u32;
const MCPWM0_GEN0_TSTMP_A: *mut u32 = 0x6001_4040 as *mut u32;
const MCPWM0_GEN0_A: *mut u32 = 0x6001_4050 as *mut u32;
const MCPWM0_CLK: *mut u32 = 0x6001_4128 as *mut u32;

#[inline(always)]
unsafe fn uart_write(bytes: &[u8]) {
    for &byte in bytes {
        core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Enable UART0's APB/function clocks for the captured smoke marker.
        core::ptr::write_volatile(PCR_UART0_CONF, 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));
        core::ptr::write_volatile(IO_MUX_GPIO16, 1);
        core::ptr::write_volatile(UART0_CLKDIV, 694);

        // GPIO4 is an exposed DevKitC-1 header pad. Route C6 MCPWM0 PWM0A
        // (PWM0_OUT0A_IDX = 87) through the GPIO matrix to that pad.
        core::ptr::write_volatile(GPIO_FUNC4_OUT_SEL_CFG, 87);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO4_MASK);

        // The C6 MCPWM APB clock is enabled by MCPWM0.CLK.EN. CLK_160M / 16
        // gives a 10 MHz module clock; timer prescale 9 gives 1 MHz ticks.
        core::ptr::write_volatile(MCPWM0_CLK, 1);
        core::ptr::write_volatile(MCPWM0_CLK_CFG, 15);
        core::ptr::write_volatile(MCPWM0_GEN0_TSTMP_A, 1_500);
        core::ptr::write_volatile(MCPWM0_GEN0_A, 0x12); // TEZ high, TEA low
        core::ptr::write_volatile(MCPWM0_TIMER0_CFG0, 9 | (19_999 << 8));
        // C6 CFG1: START=2 (continuous) in [2:0], MOD=1 (up) in [4:3].
        core::ptr::write_volatile(MCPWM0_TIMER0_CFG1, 2 | (1 << 3));
    }

    unsafe { uart_write(b"ESP32_C6_MCPWM0_SERVO_OK\n") };
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}
