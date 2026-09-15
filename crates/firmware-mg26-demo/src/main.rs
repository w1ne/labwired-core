#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

// BRD2709A (xG26-EK2709A) VCOM maps to USART1 (PB02/PB03, 115200 8N1 — UG594).
// This firmware runs on BOTH the simulator and the physical board, so it does
// the full Series-2 bring-up silicon needs (clock gates + pin route), which
// the sim accepts as no-ops (stub windows / direct TX sink).
//
// Register facts (simplicity_sdk sisdk-2025.6, efr32mg26_{cmu,gpio,usart}.h;
// struct offsets computed with offsetof against those headers):
//   CMU_CLKEN0      @ 0x40008064 — GPIO bit 26
//   CMU_CLKEN2      @ 0x4000806C — USART1 bit 7
//   GPIOB MODEL     @ 0x4003C064 — pin 2 nibble = PUSHPULL (0x4) for VCOM TX
//   USART1 ROUTEEN  @ 0x4003C840 — TXPEN bit 4
//   USART1 TXROUTE  @ 0x4003C858 — PORT bits [1:0] (B=1), PIN bits [19:16]
//   USART1 EN @0x04, CMD @0x14 (TXEN = bit 2), STATUS @0x18 (TXBL = bit 6),
//   CLKDIV @0x1C, TXDATA @0x38.
//
// Baud basis: out of reset the EM01 group A clock (PCLK) selects the
// HFRCODPLL path (CMU_EM01GRPACLKCTRL reset 0x1), i.e. HFRCO at its startup
// band — 19 MHz (HFRCODPLL_STARTUP_FREQ, system_efr32mg26.c). emlib's async
// CLKDIV formula (em_usart.c, Series-2 5-fraction-bit arm):
//   clkdiv = ((32*f + ovs*br/2) / (ovs*br) - 32) * 8
//   f=19 MHz, ovs=16, br=115200  →  2384  (~115152 baud, -0.04 %)
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const GPIOB_MODEL: *mut u32 = 0x4003_C064 as *mut u32;
const USART1_ROUTEEN: *mut u32 = 0x4003_C840 as *mut u32;
const USART1_TXROUTE: *mut u32 = 0x4003_C858 as *mut u32;
const GPIO_USART_ROUTEEN_TXPEN: u32 = 1 << 4;
const TXROUTE_PB02: u32 = 1 | (2 << 16); // PORT=B(1), PIN=2

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;

const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART1_CLKDIV_115200: u32 = 2384; // 19 MHz PCLK, ovs16 — see header

// Series-2 GPIO port structs (efr32mg26_gpio.h P[4] at block+0x30, stride
// 0x30): PB @ 0x4003C060, PC @ 0x4003C090. Within a port: MODEL @0x04 (pins
// 0..7), MODEH @0x0C (pins 8..15), DOUT @0x10, DIN @0x14; mode nibble 0x4 =
// PUSHPULL. LED0/LED1 = PC08/PC09 (active-high), BTN0/BTN1 = PB00/PB01
// (active-low) — UG594.
const GPIOB_DIN: *const u32 = 0x4003_C074 as *const u32;
const GPIOC_MODEH: *mut u32 = 0x4003_C09C as *mut u32;
const GPIOC_DOUT: *mut u32 = 0x4003_C0A0 as *mut u32;
const GPIOC_DIN: *const u32 = 0x4003_C0A4 as *const u32;

const MODE_PUSHPULL: u32 = 0x4;
const LED_MASK: u32 = (1 << 8) | (1 << 9); // PC08 | PC09

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

fn putbit(v: u32) {
    putc(if v != 0 { b'1' } else { b'0' });
}

/// Report the LED pin levels as DIN reads them — the pin path, not the DOUT
/// latch. A model where DOUT writes never reach the pins prints zeros here.
fn report_leds() {
    let din = read_u32(GPIOC_DIN);
    puts("PC08=");
    putbit((din >> 8) & 1);
    puts(" PC09=");
    putbit((din >> 9) & 1);
    puts("\n");
}

fn main() -> ! {
    // ── Silicon bring-up (sim-safe no-ops there) ──────────────────────────
    // 1. Clock gates: GPIO and USART1 bus clocks. Without these, every APB
    //    access to the blocks bus-faults on real silicon.
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    // 2. VCOM TX pin: PB02 push-pull + route USART1 TX onto it.
    write_u32(GPIOB_MODEL, MODE_PUSHPULL << 8); // pin 2 nibble
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    // 3. USART1: enable, 115200 8N1 from the out-of-reset PCLK, TX on.
    write_u32(USART1_EN, 1); // EN.EN
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN);

    puts("brd2709a: MG26 OK\n");

    // ── IO smoke: LED0/LED1 (PC08/PC09) toggle + BTN0/BTN1 (PB00/PB01) read ──
    puts("MG26-IO\n");
    // PC08/PC09 = PUSHPULL outputs (MODEH nibbles 0 and 1).
    write_u32(GPIOC_MODEH, MODE_PUSHPULL | (MODE_PUSHPULL << 4));

    write_u32(GPIOC_DOUT, LED_MASK); // both LEDs on
    report_leds();

    write_u32(GPIOC_DOUT, 0); // both LEDs off
    report_leds();

    let btn = read_u32(GPIOB_DIN);
    puts("BTN0=");
    putbit(btn & 1);
    puts(" BTN1=");
    putbit((btn >> 1) & 1);
    puts("\n");

    write_u32(GPIOC_DOUT, LED_MASK); // final state: both LEDs on
    puts("MG26-IO DONE\n");

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
