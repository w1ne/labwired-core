#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP32-C61-DevKitC-1 v2.0 LEDC smoke: a 49.8 Hz PWM signal on GPIO6.
//!
//! This configures C61 LEDC timer 2 and channel 0 directly from the C61 TRM,
//! routes LEDC channel 0 through the GPIO matrix to header J1 GPIO6, commits a
//! midpoint SG90 duty, and leaves the timer running for pad-edge capture.

use panic_halt as _;
use riscv_rt::entry;

const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const PCR_LEDC_CONF: *mut u32 = 0x6009_6034 as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6009_1038 as *mut u32;
const GPIO_FUNC6_OUT_SEL_CFG: *mut u32 = 0x6009_1AEC as *mut u32;
const IO_MUX_GPIO6: *mut u32 = 0x6009_0018 as *mut u32;
const LEDC_TIMER2_CONF: *mut u32 = 0x6000_70B0 as *mut u32;
const LEDC_CH0_CONF0: *mut u32 = 0x6000_7000 as *mut u32;
const LEDC_CH0_HPOINT: *mut u32 = 0x6000_7004 as *mut u32;
const LEDC_CH0_DUTY: *mut u32 = 0x6000_7008 as *mut u32;
const LEDC_CH0_CONF1: *mut u32 = 0x6000_700C as *mut u32;
const GPIO6_MASK: u32 = 1 << 6;

fn uart_str(text: &str) {
    for byte in text.bytes() {
        unsafe { core::ptr::write_volatile(UART0_FIFO, u32::from(byte)) }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // The C61 PCR reset selects XTAL_40M for LEDC. Preserve that source
        // selection and enable the peripheral clock gate.
        core::ptr::write_volatile(PCR_LEDC_CONF, 1);

        // Select GPIO function and LEDC output signal 0 on header J1 GPIO6.
        let mux = core::ptr::read_volatile(IO_MUX_GPIO6);
        core::ptr::write_volatile(IO_MUX_GPIO6, (mux & !(0b111 << 12)) | (1 << 12));
        core::ptr::write_volatile(GPIO_FUNC6_OUT_SEL_CFG, 1 << 10);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO6_MASK);

        // C61 TIMER2_CONF: DUTY_RES=14, CLK_DIV=49.0 in Q8 at bits [22:5],
        // PARA_UP bit 26. RST is asserted for setup, then released.
        let timer = 14 | (49 << 13) | (1 << 24) | (1 << 26);
        core::ptr::write_volatile(LEDC_TIMER2_CONF, timer);
        core::ptr::write_volatile(LEDC_TIMER2_CONF, timer & !(1 << 24));

        // C61 places TIMER_SEL in bits [2:1]. Route channel 0 from timer 2,
        // enable output, and pulse PARA_UP bit 5.
        core::ptr::write_volatile(LEDC_CH0_HPOINT, 0);
        core::ptr::write_volatile(LEDC_CH0_CONF0, (2 << 1) | (1 << 3) | (1 << 5));
        // A 1.45 ms high pulse in the approximately 20 ms frame is near the
        // center of an SG90's range. DUTY carries four fractional bits.
        core::ptr::write_volatile(LEDC_CH0_DUTY, 1187 << 4);
        core::ptr::write_volatile(LEDC_CH0_CONF1, 1 << 31);
    }

    uart_str("ESP32_C61_LEDC_SERVO_OK\n");
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) }
    }
}
