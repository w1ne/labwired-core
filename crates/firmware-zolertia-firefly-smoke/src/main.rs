//! Zolertia Firefly (Zoul, CC2538SF53) board smoke on the cc2538sf53 chip
//! surface.
//!
//! Board wiring (Contiki-NG arch/platform/zoul/firefly/board.h):
//!   LED1 red PD5, LED2 green PD4, LED3 blue PD3 (active high)
//!   BUTTON_USER PA3 (active low, IOC reset pull-up)
//!   UART0 RX PA0 / TX PA1, UART1 RX PC1 / TX PC0
//!   SSI0 SCLK PB2 / MOSI PB1 / MISO PB3 (the CC1200, not modelled)
//!
//! Checks, one letter each on the summary line (P pass, F fail):
//!   1 clk     CLOCK_STA reports the 16-MHz HF-RC reset clock
//!   2 leds    each LED on/off through a one-pin GPIO_DATA mask; the
//!             three-pin mask reads back only the pin that was written
//!   3 dir     GPIO_D DIR reads back the LED mask
//!   4 button  PA3 reads 1 (released; the board button holds the pad high)
//!   5 ssi0    SSI0 internal loopback returns 0xA5, 0x3C
//!   6 gpt0    GPTimer0 32-bit periodic: three time-outs
//!   7 sleep   the 32-kHz sleep timer advances
//!   8 uart1   UART1 divisors read back
//!   9 sram    a pattern at the top of the 32 KiB SRAM reads back
//! The summary word 0xF1EF_0000 | pass mask (all pass: 0xF1EF_01FF) goes to
//! 0x2000_7F00. The red LED then pulses three times, and UART1 prints one
//! line last.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── SYS_CTRL (SWRU319C Table 7-3, base 0x400D_2000) ─────────────────────────
const SYS_CTRL: u32 = 0x400D_2000;
const SYS_CTRL_CLOCK_STA: u32 = SYS_CTRL + 0x04;
const SYS_CTRL_RCGCGPT: u32 = SYS_CTRL + 0x08;
const SYS_CTRL_RCGCSSI: u32 = SYS_CTRL + 0x18;
const SYS_CTRL_RCGCUART: u32 = SYS_CTRL + 0x28;
/// CLOCK_STA.SYS_DIV (bits 2:0): 32 MHz >> SYS_DIV.
const CLOCK_STA_SYS_DIV: u32 = 0x7;

// ── IOC (SWRU319C Table 9-7, base 0x400D_4000) ──────────────────────────────
const IOC: u32 = 0x400D_4000;
const IOC_PA1_SEL: u32 = IOC + 0x004;
const IOC_PC0_SEL: u32 = IOC + 0x040;
const IOC_PA0_OVER: u32 = IOC + 0x080;
const IOC_PA1_OVER: u32 = IOC + 0x084;
const IOC_PC0_OVER: u32 = IOC + 0x0C0;
const IOC_PC1_OVER: u32 = IOC + 0x0C4;
const IOC_UARTRXD_UART0: u32 = IOC + 0x100;
const IOC_UARTRXD_UART1: u32 = IOC + 0x108;
/// Table 9-1 peripheral output selects.
const IOC_SEL_UART0_TXD: u32 = 0x0;
const IOC_SEL_UART1_TXD: u32 = 0x2;
const IOC_OVER_OE: u32 = 0x8;
const IOC_OVER_DIS: u32 = 0x0;
/// INPUT_SEL = port × 8 + pin.
const IOC_INPUT_PA0: u32 = 0;
const IOC_INPUT_PC1: u32 = 2 * 8 + 1;

// ── GPIO (SWRU319C Table 9-2) ───────────────────────────────────────────────
const GPIO_A: u32 = 0x400D_9000;
const GPIO_C: u32 = 0x400D_B000;
const GPIO_D: u32 = 0x400D_C000;
const GPIO_DIR: u32 = 0x400;
const GPIO_AFSEL: u32 = 0x420;

const LED_RED: u32 = 5; // PD5
const LED_GREEN: u32 = 4; // PD4
const LED_BLUE: u32 = 3; // PD3
const LED_MASK: u32 = (1 << LED_RED) | (1 << LED_GREEN) | (1 << LED_BLUE);
const BUTTON_USER: u32 = 3; // PA3

// ── UART0/1 (SWRU319C Table 18-4) ───────────────────────────────────────────
const UART0: u32 = 0x4000_C000;
const UART1: u32 = 0x4000_D000;
const UART_DR: u32 = 0x000;
const UART_FR: u32 = 0x018;
const UART_IBRD: u32 = 0x024;
const UART_FBRD: u32 = 0x028;
const UART_LCRH: u32 = 0x02C;
const UART_CTL: u32 = 0x030;
const UART_CC: u32 = 0xFC8;
const UART_FR_TXFF: u32 = 1 << 5;
const UART_LCRH_8N1: u32 = 0x3 << 5;
const UART_CTL_EN: u32 = (1 << 0) | (1 << 8) | (1 << 9);
const BAUD: u32 = 115_200;

// ── SSI0 (SWRU319C §19, base 0x4000_8000) ───────────────────────────────────
const SSI0: u32 = 0x4000_8000;
const SSI_CR0: u32 = SSI0;
const SSI_CR1: u32 = SSI0 + 0x004;
const SSI_DR: u32 = SSI0 + 0x008;
const SSI_SR: u32 = SSI0 + 0x00C;
const SSI_CPSR: u32 = SSI0 + 0x010;
const SSI_CC: u32 = SSI0 + 0xFC8;
const SSI_CR0_DSS8: u32 = 0x7;
const SSI_CR1_LBM: u32 = 1 << 0;
const SSI_CR1_SSE: u32 = 1 << 1;
const SSI_SR_RNE: u32 = 1 << 2;

// ── GPTimer0 (SWRU319C §11, base 0x4003_0000) ───────────────────────────────
const GPT0: u32 = 0x4003_0000;
const GPT_CFG: u32 = GPT0;
const GPT_TAMR: u32 = GPT0 + 0x004;
const GPT_CTL: u32 = GPT0 + 0x00C;
const GPT_RIS: u32 = GPT0 + 0x01C;
const GPT_ICR: u32 = GPT0 + 0x024;
const GPT_TAILR: u32 = GPT0 + 0x028;
const GPT_MR_PERIODIC: u32 = 0x2;
const GPT_CTL_TAEN: u32 = 1 << 0;
const GPT_INT_TATO: u32 = 1 << 0;

// ── SMWDTHROSC sleep timer (SWRU319C §13, base 0x400D_5000) ────────────────
const SLEEP_ST0: u32 = 0x400D_5040;
const SLEEP_ST1: u32 = 0x400D_5044;
const SLEEP_ST2: u32 = 0x400D_5048;
const SLEEP_ST3: u32 = 0x400D_504C;

const SRAM_TOP_WORD: u32 = 0x2000_7FFC;
const SUMMARY: u32 = 0x2000_7F00;
const SUMMARY_TAG: u32 = 0xF1EF_0000;

/// Customer Configuration Area (SWRU319C §8.6): valid image, vectors at
/// 0x0020_0000, backdoor enabled active low on PA3 (the Firefly user button,
/// board.h "shared with bootloader"), nothing locked. The simulator enters
/// the ELF directly; the CCA is here so the same image boots on silicon.
#[link_section = ".flashcca"]
#[used]
static CCA: [u32; 11] = [
    0xF3FF_FFFF,
    0x0000_0000,
    0x0020_0000,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
    0xFFFF_FFFF,
];

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// GPIO_DATA through the address mask: offset bits [9:2] select the pins a
/// read returns or a write alters (SWRU319C §9.2.2.1.2).
fn data_addr(port: u32, mask: u32) -> u32 {
    port + ((mask & 0xFF) << 2)
}

fn pin(port: u32, n: u32) -> u32 {
    (rd(data_addr(port, 1 << n)) >> n) & 1
}

fn putc(uart: u32, byte: u8) {
    while rd(uart + UART_FR) & UART_FR_TXFF != 0 {}
    wr(uart + UART_DR, u32::from(byte));
}

fn puts(uart: u32, text: &[u8]) {
    for byte in text {
        putc(uart, *byte);
    }
}

/// UART init (§18.5): disable, SYS_DIV baud clock, divisors, LCRH, enable.
fn uart_init(uart: u32, sysclk: u32) {
    let brd64 = (sysclk * 8 / BAUD).div_ceil(2); // round(64 × sysclk / (16 × baud))
    wr(uart + UART_CTL, 0);
    wr(uart + UART_CC, 0);
    wr(uart + UART_IBRD, brd64 / 64);
    wr(uart + UART_FBRD, brd64 % 64);
    wr(uart + UART_LCRH, UART_LCRH_8N1);
    wr(uart + UART_CTL, UART_CTL_EN);
}

fn wait_set(addr: u32, mask: u32) -> bool {
    for _ in 0..100_000 {
        if rd(addr) & mask != 0 {
            return true;
        }
    }
    false
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn led_set(n: u32, on: bool) {
    wr(data_addr(GPIO_D, 1 << n), if on { 0xFF } else { 0x00 });
}

fn check_leds() -> bool {
    let mut ok = true;
    for n in [LED_RED, LED_GREEN, LED_BLUE] {
        led_set(n, true);
        ok &= pin(GPIO_D, n) == 1 && rd(data_addr(GPIO_D, LED_MASK)) == 1 << n;
        led_set(n, false);
        ok &= pin(GPIO_D, n) == 0 && rd(data_addr(GPIO_D, LED_MASK)) == 0;
    }
    ok
}

fn check_ssi0() -> bool {
    wr(SYS_CTRL_RCGCSSI, rd(SYS_CTRL_RCGCSSI) | 1);
    wr(SSI_CR1, 0);
    wr(SSI_CC, 0);
    wr(SSI_CPSR, 2);
    wr(SSI_CR0, SSI_CR0_DSS8);
    wr(SSI_CR1, SSI_CR1_LBM | SSI_CR1_SSE);
    wr(SSI_DR, 0xA5);
    wr(SSI_DR, 0x3C);
    let ok = wait_set(SSI_SR, SSI_SR_RNE) && rd(SSI_DR) == 0xA5 && rd(SSI_DR) == 0x3C;
    wr(SSI_CR1, 0);
    ok
}

fn check_gpt0() -> bool {
    wr(SYS_CTRL_RCGCGPT, rd(SYS_CTRL_RCGCGPT) | 1);
    wr(GPT_CTL, 0);
    wr(GPT_CFG, 0);
    wr(GPT_TAMR, GPT_MR_PERIODIC);
    wr(GPT_TAILR, 1999);
    wr(GPT_CTL, GPT_CTL_TAEN);
    let mut timeouts = 0;
    for _ in 0..3 {
        if !wait_set(GPT_RIS, GPT_INT_TATO) {
            break;
        }
        wr(GPT_ICR, GPT_INT_TATO);
        timeouts += 1;
    }
    wr(GPT_CTL, 0);
    timeouts == 3
}

/// Read ST0 first to latch the 32-kHz count, then the upper bytes.
fn sleep_count() -> u32 {
    (rd(SLEEP_ST0) & 0xFF)
        | ((rd(SLEEP_ST1) & 0xFF) << 8)
        | ((rd(SLEEP_ST2) & 0xFF) << 16)
        | ((rd(SLEEP_ST3) & 0xFF) << 24)
}

fn check_sleep_timer() -> bool {
    let before = sleep_count();
    spin(4_000);
    sleep_count() != before
}

fn check_sram_top() -> bool {
    wr(SRAM_TOP_WORD, 0x5A5A_C3C3);
    let ok = rd(SRAM_TOP_WORD) == 0x5A5A_C3C3;
    wr(SRAM_TOP_WORD, !0x5A5A_C3C3);
    ok && rd(SRAM_TOP_WORD) == !0x5A5A_C3C3
}

#[entry]
fn main() -> ! {
    wr(SYS_CTRL_RCGCUART, rd(SYS_CTRL_RCGCUART) | 0x3);

    // UART0 on PA0/PA1, UART1 on PC1/PC0 (board.h), under hardware control.
    wr(IOC_PA1_SEL, IOC_SEL_UART0_TXD);
    wr(IOC_PA1_OVER, IOC_OVER_OE);
    wr(IOC_PA0_OVER, IOC_OVER_DIS);
    wr(IOC_UARTRXD_UART0, IOC_INPUT_PA0);
    wr(GPIO_A + GPIO_AFSEL, rd(GPIO_A + GPIO_AFSEL) | 0x03);
    wr(IOC_PC0_SEL, IOC_SEL_UART1_TXD);
    wr(IOC_PC0_OVER, IOC_OVER_OE);
    wr(IOC_PC1_OVER, IOC_OVER_DIS);
    wr(IOC_UARTRXD_UART1, IOC_INPUT_PC1);
    wr(GPIO_C + GPIO_AFSEL, rd(GPIO_C + GPIO_AFSEL) | 0x03);

    let sysclk = 32_000_000 >> (rd(SYS_CTRL_CLOCK_STA) & CLOCK_STA_SYS_DIV);
    uart_init(UART0, sysclk);
    uart_init(UART1, sysclk);

    // LEDs PD3..PD5 as software-controlled outputs, off.
    wr(GPIO_D + GPIO_AFSEL, rd(GPIO_D + GPIO_AFSEL) & !LED_MASK);
    wr(data_addr(GPIO_D, LED_MASK), 0);
    wr(GPIO_D + GPIO_DIR, rd(GPIO_D + GPIO_DIR) | LED_MASK);

    puts(UART0, b"ZOLERTIA FIREFLY CC2538 Cortex-M3 UART0 smoke\n");

    let checks = [
        sysclk == 16_000_000,
        check_leds(),
        rd(GPIO_D + GPIO_DIR) & 0xFF == LED_MASK,
        pin(GPIO_A, BUTTON_USER) == 1,
        check_ssi0(),
        check_gpt0(),
        check_sleep_timer(),
        rd(UART1 + UART_IBRD) == 8 && rd(UART1 + UART_FBRD) == 44,
        check_sram_top(),
    ];

    let mut mask = 0u32;
    puts(UART0, b"FIREFLY checks ");
    for (i, ok) in checks.iter().enumerate() {
        if *ok {
            mask |= 1 << i;
        }
        putc(UART0, if *ok { b'P' } else { b'F' });
    }
    puts(UART0, b" DONE\n");
    wr(SUMMARY, SUMMARY_TAG | mask);

    for _ in 0..3 {
        led_set(LED_RED, true);
        spin(2_000);
        led_set(LED_RED, false);
        spin(2_000);
    }

    puts(UART1, b"FIREFLY UART1 line\n");

    loop {
        core::hint::spin_loop();
    }
}
