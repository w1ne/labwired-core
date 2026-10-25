// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Minimal RP2040 loop lab: the shipped demo's hot shape, isolated.
//!
//! `head: bl leaf; adds r0, #1; subs r4, #1; bne head` with a leaf callee
//! (`adds r0, #1; bx lr`), 200 iterations, then a park. Built for
//! thumbv6m-none-eabi so the loop crosses the Cortex-M JIT unchanged: it is
//! exactly the shape `emit_loop_trace` claims (conditional backedge to its
//! own entry, pure-ALU body, one inlined leaf call), and the run must not
//! depend on whether the trace is on.
//!
//! The vector table sits at the very start of `.text` (the RP2040 boots by
//! reading SP and the reset vector from 0x10000000), so no separate section
//! start is needed.
//!
//! Rebuild with `crates/cli/tests/fixtures/rp2040_loop_lab/build.sh`.

#![no_std]
#![no_main]

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_: &PanicInfo) -> ! {
    loop {}
}

core::arch::global_asm!(
    r#"
    .syntax unified
    .thumb
    .section .text
    .thumb_func
    .global reset
reset:
    .word 0x20001000
    .word _start
    .thumb_func
    .global _start
_start:
    movs r4, #200
    movs r0, #0
    .global head
head:
    bl leaf
    adds r0, #1
    subs r4, #1
    bne head
    movs r1, #1
    movs r2, #0
    ldr r3, =0x40034000
    str r0, [r3]
    str r1, [r3]
    str r2, [r3]
    .global park
park:
    b park
    .thumb_func
leaf:
    adds r0, #1
    bx lr
"#
);
