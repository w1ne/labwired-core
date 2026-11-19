#![no_std]
#![no_main]

//! RAMN ECU (STM32L552) LPUART1 + GPIO smoke.
//!
//! Mirrors the RAMN firmware's own bring-up (ToyotaInfoTech/RAMN
//! `firmware/RAMNV1/Core/Src/main.c`, `stm32l5xx_hal_msp.c`): LPUART1 on
//! PA2/PA3 (AF8) at 115200 8N1, PB6 = FDCAN1_STB push-pull output, PA9 =
//! HW_TYPE input with pull-up. Clocks come from RCC AHB2ENR / APB1ENR2 on the
//! reset MSI 4 MHz clock. Results land in RAM words the smoke script asserts
//! and are echoed on the console.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC: u32 = 0x4002_1000;
const RCC_AHB2ENR: u32 = 0x4C;
const RCC_APB1ENR2: u32 = 0x5C;

const GPIOA: u32 = 0x4202_0000;
const GPIOB: u32 = 0x4202_0400;
const MODER: u32 = 0x00;
const PUPDR: u32 = 0x0C;
const IDR: u32 = 0x10;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;
const AFRL: u32 = 0x20;

const LPUART1: u32 = 0x4000_8000;
const CR1: u32 = 0x00;
const BRR: u32 = 0x0C;
const ISR: u32 = 0x1C;
const TDR: u32 = 0x28;
const CR1_UE: u32 = 1 << 0;
const CR1_RE: u32 = 1 << 2;
const CR1_TE: u32 = 1 << 3;
const ISR_TC: u32 = 1 << 6;
const ISR_TXE: u32 = 1 << 7;
// LPUART BRR = 256 * f_ck / baud (RM0438 LPUART baud rate generation):
// 256 * 4 MHz / 115200 = 8888.
const BRR_115200_MSI4: u32 = 8888;

const RESULT: u32 = 0x2000_1000;
const MAGIC: u32 = 0x5A4E_0000;

fn rd(base: u32, off: u32) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}

fn wr(base: u32, off: u32, value: u32) {
    unsafe { write_volatile((base + off) as *mut u32, value) }
}

fn putc(byte: u8) {
    while rd(LPUART1, ISR) & ISR_TXE == 0 {}
    wr(LPUART1, TDR, u32::from(byte));
}

fn puts(s: &[u8]) {
    for &b in s {
        putc(b);
    }
}

fn put_hex(value: u32) {
    for shift in (0..8).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        });
    }
}

fn put_field(name: &[u8], value: u32) {
    puts(name);
    putc(b'=');
    put_hex(value);
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = out(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack, preserves_flags),
        );
        write_volatile(RESULT as *mut u32, product);
    }

    let mut bits = 0u32;

    // GPIOAEN | GPIOBEN, LPUART1EN.
    wr(RCC, RCC_AHB2ENR, rd(RCC, RCC_AHB2ENR) | 0b11);
    wr(RCC, RCC_APB1ENR2, rd(RCC, RCC_APB1ENR2) | 1);

    // PA2/PA3 alternate function 8 (LPUART1_TX/RX); PA9 input, pull-up.
    let moder_a = (rd(GPIOA, MODER) & !((0b11 << 4) | (0b11 << 6) | (0b11 << 18)))
        | (0b10 << 4)
        | (0b10 << 6);
    wr(GPIOA, MODER, moder_a);
    wr(
        GPIOA,
        AFRL,
        (rd(GPIOA, AFRL) & !0xFF00) | (8 << 8) | (8 << 12),
    );
    wr(
        GPIOA,
        PUPDR,
        (rd(GPIOA, PUPDR) & !(0b11 << 18)) | (0b01 << 18),
    );
    let pa_moder = rd(GPIOA, MODER);
    let pa_afrl = rd(GPIOA, AFRL);
    if (pa_moder >> 4) & 0xF == 0b1010 && (pa_moder >> 18) & 0b11 == 0 && pa_afrl & 0xFF00 == 0x8800
    {
        bits |= 1 << 0;
    }
    let hw_type = (rd(GPIOA, IDR) >> 9) & 1;
    if hw_type == 1 {
        bits |= 1 << 1;
    }

    // LPUART1 115200 8N1, TX+RX, FIFO off (RAMN MX_LPUART1_UART_Init).
    wr(LPUART1, BRR, BRR_115200_MSI4);
    wr(LPUART1, CR1, CR1_UE | CR1_RE | CR1_TE);
    let brr = rd(LPUART1, BRR);
    let isr = rd(LPUART1, ISR);
    if brr == BRR_115200_MSI4 {
        bits |= 1 << 2;
    }
    if isr & (ISR_TXE | ISR_TC) == (ISR_TXE | ISR_TC) {
        bits |= 1 << 3;
    }

    // PB6 FDCAN1_STB push-pull output, driven through BSRR.
    wr(
        GPIOB,
        MODER,
        (rd(GPIOB, MODER) & !(0b11 << 12)) | (0b01 << 12),
    );
    wr(GPIOB, BSRR, 1 << 6);
    let set = (rd(GPIOB, ODR) >> 6) & 1;
    wr(GPIOB, BSRR, 1 << (6 + 16));
    let clr = (rd(GPIOB, ODR) >> 6) & 1;
    if set == 1 && clr == 0 {
        bits |= 1 << 4;
    }

    puts(b"RAMN LPUART1 SMOKE\r\n");
    put_field(b"lpuart1 brr", brr);
    put_field(b" isr", isr);
    puts(b"\r\n");
    put_field(b"pa moder", pa_moder);
    put_field(b" afrl", pa_afrl);
    put_field(b" hw_type", hw_type);
    puts(b"\r\n");
    put_field(b"pb6 set", set);
    put_field(b" clr", clr);
    puts(b"\r\n");
    while rd(LPUART1, ISR) & ISR_TC == 0 {}
    bits |= 1 << 5;
    put_field(b"pass", bits);
    puts(b"\r\n");
    unsafe { write_volatile((RESULT + 4) as *mut u32, MAGIC | bits) };

    // FDCAN1_STB heartbeat.
    let mut n = 0u32;
    loop {
        n = n.wrapping_add(1);
        if n % 2048 == 0 {
            wr(
                GPIOB,
                BSRR,
                if (n / 2048) & 1 == 1 { 1 << 6 } else { 1 << 22 },
            );
        }
    }
}
