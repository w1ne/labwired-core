// LabWired Flex portable firmware fixture. SPDX-License-Identifier: MIT
//
// This program compiles directly to WebAssembly. It has no target ISA,
// memory-mapped addresses, CPU emulator, device library, or WASI dependency.
// All hardware work goes through three portable host imports. A production
// application can place these calls behind its normal hardware abstraction.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// Drive the configured command, then measure the RC rise time. `mode == 1`
/// deliberately submits the wrong command. The loop is bounded, including
/// its timeout path, so browser execution always yields to its caller.
#[no_mangle]
pub extern "C" fn run(command_port: u32, voltage_port: u32, mode: u32) -> u32 {
    unsafe {
        write(command_port, if mode == 0 { 1.0 } else { 0.0 });
        for sample in 1..=160 {
            advance_ns(25_000);
            if read(voltage_port) >= 3.3 * 0.632 {
                return sample * 25;
            }
        }
    }
    0
}
