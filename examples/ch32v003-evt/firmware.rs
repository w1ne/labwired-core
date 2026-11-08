#![no_std]
#![no_main]

use core::arch::global_asm;

// Keep this first smoke freestanding and ABI-free: every instruction uses only
// x0..x7, which exercises the CH32V003's RV32E register file without relying
// on the upstream Rust toolchain's unsupported rv32e calling convention.
global_asm!(
    r#"
    .section .text.entry
    .global _start
    .type _start,@function
_start:
    .option push
    .option rvc
    lui   t0, 0x40021          # RCC @ 0x40021000
    addi  t1, zero, 0x20       # APB2PCENR.GPIODEN
    sw    t1, 0x18(t0)
    lui   t0, 0x40011          # APB2 GPIO window @ 0x40011000
    addi  t0, t0, 0x400       # GPIOD @ 0x40011400
    addi  t1, zero, 1          # CFGLR: PD0 output, push-pull, 10 MHz
    sw    t1, 0(t0)
1:
    addi  t2, zero, 2000
2:  addi  t2, t2, -1
    bnez  t2, 2b
    addi  t1, zero, 1
    sw    t1, 0x10(t0)         # BSHR: set PD0
    addi  t2, zero, 2000
3:  addi  t2, t2, -1
    bnez  t2, 3b
    lui   t1, 0x10             # BSHR[31:16]: reset PD0
    sw    t1, 0x10(t0)
    j     1b
    .option pop
    "#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
