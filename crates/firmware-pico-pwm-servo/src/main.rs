#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const RESETS_BASE: u32 = 0x4000_C000;
const CLOCKS_BASE: u32 = 0x4000_8000;
const UART0_BASE: u32 = 0x4003_4000;
const IO_BANK0_BASE: u32 = 0x4001_4000;
const PWM_BASE: u32 = 0x4005_0000;

const RESETS_RESET: *mut u32 = RESETS_BASE as *mut u32;
const RESETS_RESET_DONE: *const u32 = (RESETS_BASE + 0x08) as *const u32;
const RESET_IO_BANK0: u32 = 1 << 5;
const RESET_PADS_BANK0: u32 = 1 << 8;
const RESET_PWM: u32 = 1 << 14;
const RESET_UART0: u32 = 1 << 22;

const CLK_PERI_CTRL: *mut u32 = (CLOCKS_BASE + 0x48) as *mut u32;
const CLK_PERI_ENABLE: u32 = 1 << 11;

const GPIO0_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04) as *mut u32;
const GPIO16_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 16 * 8) as *mut u32;
const GPIO_FUNC_UART: u32 = 2;
const GPIO_FUNC_PWM: u32 = 4;

const PWM_SLICE0_CSR: *mut u32 = PWM_BASE as *mut u32;
const PWM_SLICE0_DIV: *mut u32 = (PWM_BASE + 0x04) as *mut u32;
const PWM_SLICE0_CC: *mut u32 = (PWM_BASE + 0x0C) as *mut u32;
const PWM_SLICE0_TOP: *mut u32 = (PWM_BASE + 0x10) as *mut u32;
const PWM_ENABLE: u32 = 1;
const PWM_DIV_125: u32 = 125 << 4; // RP2040 8.4 fixed-point divider, 125.0
const SERVO_PERIOD_TICKS: u32 = 20_000;
const SERVO_PULSE_TICKS: u32 = 1_500;

const UART0_DR: *mut u32 = UART0_BASE as *mut u32;
const UART0_IBRD: *mut u32 = (UART0_BASE + 0x24) as *mut u32;
const UART0_FBRD: *mut u32 = (UART0_BASE + 0x28) as *mut u32;
const UART0_LCR_H: *mut u32 = (UART0_BASE + 0x2C) as *mut u32;
const UART0_CR: *mut u32 = (UART0_BASE + 0x30) as *mut u32;
const UART_IBRD_115200_AT_125MHZ: u32 = 67;
const UART_FBRD_115200_AT_125MHZ: u32 = 52;
const UART_LCR_H_8N1_FIFO: u32 = (0b11 << 5) | (1 << 4);
const UART_CR_ENABLE_TX: u32 = (1 << 0) | (1 << 8);

#[entry]
fn main() -> ! {
    unsafe {
        // The RP2040 reset value holds peripheral blocks in reset. Release the
        // GPIO/pad banks, PWM slice, and UART0, then wait for their ACKs.
        let reset_mask = RESET_IO_BANK0 | RESET_PADS_BANK0 | RESET_PWM | RESET_UART0;
        let reset = core::ptr::read_volatile(RESETS_RESET);
        core::ptr::write_volatile(RESETS_RESET, reset & !reset_mask);
        while core::ptr::read_volatile(RESETS_RESET_DONE) & reset_mask != reset_mask {}

        // Route UART0 TX to GP0 and start clk_peri (the reset clock source is
        // clk_sys). The board system runs the Pico SDK's 125 MHz default.
        core::ptr::write_volatile(GPIO0_CTRL, GPIO_FUNC_UART);
        let clk_peri = core::ptr::read_volatile(CLK_PERI_CTRL);
        core::ptr::write_volatile(CLK_PERI_CTRL, clk_peri | CLK_PERI_ENABLE);
        core::ptr::write_volatile(UART0_IBRD, UART_IBRD_115200_AT_125MHZ);
        core::ptr::write_volatile(UART0_FBRD, UART_FBRD_115200_AT_125MHZ);
        core::ptr::write_volatile(UART0_LCR_H, UART_LCR_H_8N1_FIFO);
        core::ptr::write_volatile(UART0_CR, UART_CR_ENABLE_TX);

        // GP16 is PWM slice 0 channel A (Raspberry Pi Pico pin 21). With the
        // Pico's 125 MHz system clock, DIV=125 and TOP=19,999 produce 50 Hz;
        // CC_A=1,500 produces the centre 1.5 ms servo pulse.
        core::ptr::write_volatile(GPIO16_CTRL, GPIO_FUNC_PWM);
        core::ptr::write_volatile(PWM_SLICE0_DIV, PWM_DIV_125);
        core::ptr::write_volatile(PWM_SLICE0_TOP, SERVO_PERIOD_TICKS - 1);
        core::ptr::write_volatile(PWM_SLICE0_CC, SERVO_PULSE_TICKS);
        core::ptr::write_volatile(PWM_SLICE0_CSR, PWM_ENABLE);
    }

    print_uart("PICO_PWM_SERVO_GP16_OK\n");

    loop {
        cortex_m::asm::nop();
    }
}

fn print_uart(message: &str) {
    for byte in message.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, byte as u32) };
    }
}
