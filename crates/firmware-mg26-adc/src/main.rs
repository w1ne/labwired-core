#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! BRD2709A IADC smoke — reads PA05 and prints the 12-bit code.
//!
//! This runs on the physical board as well as the twin: every register it
//! touches is real silicon. On the bench PA05 reads whatever is wired to the
//! EXP header; in the twin it reads the `system.yaml` analog source.
//!
//! Register facts (simplicity_sdk sisdk-2025.6):
//!   IADC0_S_BASE      0x4900_4000 (`efr32mg26b510f3200im48.h`) — the ANALOG
//!                     peripheral group at 0x4900_0000, NOT the digital block
//!                     at 0x4000_0000.
//!   CMU_CLKEN0.IADC0  bit 10 (`_CMU_CLKEN0_IADC0_SHIFT`)
//!   IADC EN @0x04, CMD @0x0C (SINGLESTART bit 0), STATUS @0x14
//!                     (SINGLEFIFODV bit 8), SINGLEFIFODATA @0x74,
//!                     SINGLE @0x98 (PINPOS [11:8], PORTPOS [15:12];
//!                     PORTPOS 8 = PORTA), all walked from `IADC_TypeDef`.
//!
//! ⚠️ `IADC_TypeDef` embeds `CFG[2]` (stride 0x10) and `SCANTABLE[16]`
//! (stride 0x04). Reading those as flat words shifts every later offset — the
//! check is that `IPVERSION_SET` lands at exactly +0x1000.

// ── VCOM console: identical to firmware-mg26-demo, see its header ─────────
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN0_IADC0: u32 = 1 << 10;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const GPIOB_MODEL: *mut u32 = 0x4003_C064 as *mut u32;
const USART1_ROUTEEN: *mut u32 = 0x4003_C840 as *mut u32;
const USART1_TXROUTE: *mut u32 = 0x4003_C858 as *mut u32;
const GPIO_USART_ROUTEEN_TXPEN: u32 = 1 << 4;
const TXROUTE_PB02: u32 = 1 | (2 << 16);

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;
const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART1_CLKDIV_115200: u32 = 2384;
const MODE_PUSHPULL: u32 = 0x4;

// ── IADC0 ────────────────────────────────────────────────────────────────
const IADC0_BASE: usize = 0x4900_4000;
const IADC_EN: *mut u32 = (IADC0_BASE + 0x04) as *mut u32;
const IADC_CMD: *mut u32 = (IADC0_BASE + 0x0C) as *mut u32;
const IADC_STATUS: *const u32 = (IADC0_BASE + 0x14) as *const u32;
const IADC_SINGLEFIFODATA: *const u32 = (IADC0_BASE + 0x74) as *const u32;
const IADC_SINGLE: *mut u32 = (IADC0_BASE + 0x98) as *mut u32;

const IADC_EN_EN: u32 = 1 << 0;
const IADC_CMD_SINGLESTART: u32 = 1 << 0;
const IADC_STATUS_SINGLEFIFODV: u32 = 1 << 8;
/// `SINGLE.PORTPOS` = 8 selects PORTA; PA05 is therefore PORTPOS 8, PINPOS 5.
const SINGLE_PA05: u32 = (8 << 12) | (5 << 8);

/// Give up rather than spin forever: a conversion that never completes means
/// the IADC was never clocked or never enabled, and a hung board says less
/// than a printed failure.
const POLL_LIMIT: u32 = 100_000;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn read_u32(p: *const u32) -> u32 {
    unsafe { core::ptr::read_volatile(p) }
}

fn write_u32(p: *mut u32, v: u32) {
    unsafe { core::ptr::write_volatile(p, v) }
}

fn putc(c: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_STATUS) & USART_STATUS_TXBL == 0 {}
        core::ptr::write_volatile(USART1_TXDATA, c as u32);
    }
}

fn puts(s: &str) {
    for &b in s.as_bytes() {
        putc(b);
    }
}

fn put_u32(mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0;
    if v == 0 {
        putc(b'0');
        return;
    }
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        putc(buf[n]);
    }
}

fn main() -> ! {
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO | CMU_CLKEN0_IADC0);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    write_u32(GPIOB_MODEL, MODE_PUSHPULL << 8);
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    write_u32(USART1_EN, 1);
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN);

    puts("MG26-ADC\n");

    write_u32(IADC_EN, IADC_EN_EN);
    write_u32(IADC_SINGLE, SINGLE_PA05);
    write_u32(IADC_CMD, IADC_CMD_SINGLESTART);

    let mut spins = 0;
    while read_u32(IADC_STATUS) & IADC_STATUS_SINGLEFIFODV == 0 {
        spins += 1;
        if spins >= POLL_LIMIT {
            puts("adc timeout\n");
            loop {}
        }
    }

    puts("code=");
    put_u32(read_u32(IADC_SINGLEFIFODATA) & 0xFFF);
    putc(b'\n');

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
