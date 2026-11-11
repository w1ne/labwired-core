#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const UART0_BASE: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;
const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x38) as *mut u32;
const GPIO4_OUT_SEL: *mut u32 = (GPIO_BASE + 0xAD4 + 4 * 4) as *mut u32;
const GPIO4_MASK: u32 = 1 << 4;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable UART0's APB and function clocks using the C61 PCR registers.
        let conf = core::ptr::read_volatile(PCR_UART0_CONF);
        core::ptr::write_volatile(PCR_UART0_CONF, conf | 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));

        // C61 GPIO uses SIG_GPIO_OUT_IDX=256 and the C61 register offsets:
        // FUNC4_OUT_SEL_CFG at GPIO+0xAE4, ENABLE_W1TS at GPIO+0x38, and
        // OUT_W1TS/W1TC at GPIO+0x08/+0x0C. GPIO4 is on header J1 pin 3.
        core::ptr::write_volatile(GPIO4_OUT_SEL, 256);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO4_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO4_MASK);
        for _ in 0..4 {
            core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO4_MASK);
            core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO4_MASK);
        }

        // Set the documented 115200 baud divider, then send through UART0 FIFO.
        core::ptr::write_volatile(UART0_CLKDIV, 694);
        for &byte in b"ESP32C61_DEVKITC1_UART0_OK\n" {
            core::ptr::write_volatile(UART0_BASE, u32::from(byte));
        }
    }

    loop {}
}
