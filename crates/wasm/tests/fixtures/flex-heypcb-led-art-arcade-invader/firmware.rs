// SPDX-License-Identifier: MIT
// Portable Flex test workload for an addressable 21-pixel strip.
// The only hardware-facing value is a typed port id supplied by the host.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

fn level(port: u32, high: bool) {
    unsafe { write(port, if high { 1.0 } else { 0.0 }) };
}

fn bit(port: u32, one: bool) {
    let high_ns = if one { 800 } else { 400 };
    level(port, true);
    unsafe { advance_ns(high_ns) };
    level(port, false);
    unsafe { advance_ns(1_250 - high_ns) };
}

fn byte(port: u32, value: u8) {
    for shift in (0..8).rev() {
        bit(port, value & (1 << shift) != 0);
    }
}

fn frame(port: u32, lit: bool) {
    for pixel in 0..21 {
        let grb = match (lit, pixel) {
            (true, 0) => [0x20, 0x10, 0x08],
            (true, 1) => [0x01, 0x02, 0x03],
            _ => [0, 0, 0],
        };
        for component in grb {
            byte(port, component);
        }
    }
    unsafe { advance_ns(60_000) };
}

#[no_mangle]
pub extern "C" fn run(data_port: u32, light: u32) -> u32 {
    level(data_port, false);
    let lit = light != 0;
    frame(data_port, lit);
    if lit { 21 } else { 0 }
}
