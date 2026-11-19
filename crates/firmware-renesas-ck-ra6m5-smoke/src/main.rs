// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// CK-RA6M5 board smoke: Cortex-M33 MULS, SCI0 polled async TX, the three
// board LEDs on PORT6 and the button on PORT8. Bare-register driver, no HAL.
// The red LED (P610) blinks as a heartbeat; green (P609) mirrors the button
// (P804, high while pressed as in Renode), and presses are counted in SRAM.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0000 as *mut u32;
const SCR_READBACK: *mut u32 = 0x2000_0004 as *mut u32;
const PODR_READBACK: *mut u32 = 0x2000_0008 as *mut u32;
const BUTTON_IDLE: *mut u32 = 0x2000_000C as *mut u32;
const BUTTON_PRESSES: *mut u32 = 0x2000_0010 as *mut u32;
const FPU_RESULT: *mut u32 = 0x2000_0014 as *mut u32;
const SCB_CPACR: *mut u32 = 0xE000_ED88 as *mut u32;

// SCI0 @ 0x40118000 (Renode renesas-r7fa6m5b.repl): SCR @ +0x02,
// TDR @ +0x03, SSR @ +0x04 (TDRE bit 7, TEND bit 2).
const SCI0_SCR: *mut u8 = 0x4011_8002 as *mut u8;
const SCI0_TDR: *mut u8 = 0x4011_8003 as *mut u8;
const SCI0_SSR: *const u8 = 0x4011_8004 as *const u8;
const SCR_TE: u8 = 1 << 5;
const SSR_TDRE: u8 = 1 << 7;
const SSR_TEND: u8 = 1 << 2;

// PORTn: PCNTR1 (PODR[31:16] | PDR[15:0]) @ +0x00, PCNTR2 (PIDR[15:0])
// @ +0x04, PCNTR3 (PORR[31:16] | POSR[15:0]) @ +0x08.
const PORT6_PCNTR1: *mut u32 = 0x4008_00C0 as *mut u32;
const PORT6_PCNTR3: *mut u32 = 0x4008_00C8 as *mut u32;
const PORT8_PCNTR2: *const u32 = 0x4008_0104 as *const u32;
const LED_RED: u32 = 1 << 10; // P610
const LED_GREEN: u32 = 1 << 9; // P609
const LED_BLUE: u32 = 1 << 1; // P601
const BUTTON: u32 = 1 << 4; // P804

fn tx(byte: u8) {
    unsafe {
        while read_volatile(SCI0_SSR) & SSR_TDRE == 0 {}
        write_volatile(SCI0_TDR, byte);
    }
}

fn fpu_add_probe() -> u32 {
    unsafe {
        // Grant CP10/CP11 before executing the scalar FPU instructions.
        write_volatile(SCB_CPACR, 0x00F0_0000);
        core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
        let result: u32;
        // `.fpu` lets the probe assemble on the soft-float target too.
        core::arch::asm!(
            ".fpu fpv5-sp-d16",
            "vmov s0, r0",
            "vmov s1, r1",
            "vadd.f32 s2, s0, s1",
            "vmov r2, s2",
            in("r0") 0x3F80_0000u32,
            in("r1") 0x4000_0000u32,
            lateout("r2") result,
            out("s0") _,
            out("s1") _,
            out("s2") _,
            options(nostack, preserves_flags),
        );
        result
    }
}

fn button_pressed() -> bool {
    unsafe { read_volatile(PORT8_PCNTR2) & BUTTON != 0 }
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        let fpu_result = fpu_add_probe();
        write_volatile(FPU_RESULT, fpu_result);
        for &b in b"FPU_SUM=" {
            tx(b);
        }
        for shift in (0..8).rev() {
            let nibble = ((fpu_result >> (shift * 4)) & 0xF) as u8;
            tx(if nibble < 10 {
                b'0' + nibble
            } else {
                b'A' + nibble - 10
            });
        }
        tx(b'\n');

        core::arch::asm!(
            "muls {product}, {right}",
            product = inout(reg) 17u32 => product,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        write_volatile(RESULT, product);

        write_volatile(SCI0_SCR, SCR_TE);
        write_volatile(SCR_READBACK, u32::from(read_volatile(SCI0_SCR)));
        for &b in b"CK-RA6M5 SCI0 SMOKE\n" {
            tx(b);
        }
        while read_volatile(SCI0_SSR) & SSR_TEND == 0 {}

        // All three LEDs outputs; red and blue on, green off.
        let leds = LED_RED | LED_GREEN | LED_BLUE;
        write_volatile(PORT6_PCNTR1, read_volatile(PORT6_PCNTR1) | leds);
        write_volatile(PORT6_PCNTR3, LED_RED | LED_BLUE | (LED_GREEN << 16));
        write_volatile(PODR_READBACK, read_volatile(PORT6_PCNTR1) >> 16);
        write_volatile(BUTTON_IDLE, read_volatile(PORT8_PCNTR2) & BUTTON);
        write_volatile(BUTTON_PRESSES, 0);
    }

    let mut on = true;
    let mut was_pressed = false;
    loop {
        for _ in 0..2000 {
            unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
        }
        on = !on;
        let pressed = button_pressed();
        if pressed && !was_pressed {
            unsafe { write_volatile(BUTTON_PRESSES, read_volatile(BUTTON_PRESSES) + 1) };
        }
        was_pressed = pressed;
        // POSR sets, PORR clears.
        let heartbeat = if on { LED_RED } else { LED_RED << 16 };
        let mirror = if pressed { LED_GREEN } else { LED_GREEN << 16 };
        unsafe { write_volatile(PORT6_PCNTR3, heartbeat | mirror) };
    }
}
