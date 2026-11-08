#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 IO_BANK0 / SIO / PL011 addresses. Waveshare routes the onboard
// WS2812B DIN to GP16; its UART0 header is GP0 TX and GP1 RX.
const GPIO16_CTRL: *mut u32 = 0x4002_8104 as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;

const GPIO_FUNC_SIO: u32 = 5;
const WS2812_PIN: u32 = 1 << 16;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(GPIO16_CTRL, GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OUT_CLR, WS2812_PIN);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, WS2812_PIN);
    }

    // One red WS2812 pixel, transmitted in the device's GRB wire order.
    send_byte(0x00);
    send_byte(0xFF);
    send_byte(0x00);

    // Hold the data line low long enough for the WS2812 reset/latch interval.
    cortex_m::asm::delay(10_000);
    print_uart("RP2350_SMOKE_OK\n");

    loop {
        cortex_m::asm::nop();
    }
}

#[inline(never)]
fn send_byte(byte: u8) {
    for bit in (0..8).rev() {
        let one = byte & (1 << bit) != 0;
        unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_SET, WS2812_PIN) };

        // The simulator measures this instruction stream in RP2350 cycles.
        // These calibrated loops produce ~102 cycles (680 ns) for one and
        // ~52 cycles (347 ns) for zero, on opposite sides of the 500 ns
        // WS2812 decoder threshold at 150 MHz.
        cortex_m::asm::delay(if one { 90 } else { 40 });
        unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_CLR, WS2812_PIN) };

        // The low portion pads each bit cell. The final extended low is the
        // separate reset/latch gap above.
        cortex_m::asm::delay(if one { 76 } else { 106 });
    }
}

fn print_uart(s: &str) {
    for byte in s.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, byte as u32) };
    }
}
