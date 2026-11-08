#![no_std]
#![no_main]

//! NUCLEO-WB55RG (MB1355C) smoke firmware: a running light over the board's
//! user LEDs plus a heartbeat on the default ST-LINK VCP USART1, in bare
//! registers (no HAL).
//!
//! LD1 blue PB5, LD2 green PB0, LD3 red PB1, all active high. The default
//! VCP1 path is USART1 TX/RX on PB6/PB7 (AF7), 115200 8N1 from the 4 MHz MSI
//! clock at reset. LPUART1 on PA2/PA3 is an Arduino-header UART, not the
//! default ST-LINK VCP.
//!
//! Each beat lights ONE LED, prints `hb=<n> <led>=on`, then switches it off and
//! prints `... =off`. The state in the printed line is READ BACK from the GPIO
//! ODR register after the write, so the text reports the pad the firmware
//! actually drove and not what it intended to drive.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (STM32WB, RM0434): AHB2ENR gates GPIO ports; APB2ENR gates USART1.
const RCC: u32 = 0x5800_0000;
const RCC_AHB2ENR: u32 = RCC + 0x4C;
const RCC_APB2ENR: u32 = RCC + 0x60;

// USART1: ISR/TDR register layout; BRR = fck / baud at oversampling by 16.
const USART: u32 = 0x4001_3800;
const USART_ISR: *const u32 = (USART + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART + 0x28) as *mut u32;
const USART_BRR: *mut u32 = (USART + 0x0C) as *mut u32;
const USART_CR1: *mut u32 = USART as *mut u32;
const TXE: u32 = 1 << 7;
const GPIOB: u32 = 0x4800_0400;

/// GPIO register offsets (STM32 GPIO v2 block: MODER/ODR/BSRR).
const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const AFRH: u32 = 0x24;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

/// MB1355C: LD1 blue = PB5, LD2 green = PB0, LD3 red = PB1, active high.
const LED_ACTIVE_HIGH: bool = true;
/// User LEDs, in running-light order: (console name, GPIO port base, pin).
const LEDS: [(&[u8], u32, u32); 3] = [(b"ld1", GPIOB, 5), (b"ld2", GPIOB, 0), (b"ld3", GPIOB, 1)];

fn reg_rmw(addr: u32, clear: u32, set: u32) {
    unsafe {
        let p = addr as *mut u32;
        write_volatile(p, (read_volatile(p) & !clear) | set);
    }
}

/// Route `pin` of the GPIO port at `base` to alternate function `af`.
fn pin_af(base: u32, pin: u32, af: u32) {
    reg_rmw(base + MODER, 0b11 << (pin * 2), 0b10 << (pin * 2));
    let (reg, shift) = if pin < 8 {
        (AFRL, pin * 4)
    } else {
        (AFRH, (pin - 8) * 4)
    };
    reg_rmw(base + reg, 0xF << shift, af << shift);
}

fn putc(b: u8) {
    unsafe {
        while read_volatile(USART_ISR) & TXE == 0 {}
        write_volatile(USART_TDR, b as u32);
    }
}

fn send(s: &[u8]) {
    for b in s {
        putc(*b);
    }
}

fn put_dec(mut v: u32) {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        putc(digits[n]);
    }
}

fn led_drive(base: u32, pin: u32, on: bool) {
    // BSRR: low half sets the pin, high half resets it.
    let high = on == LED_ACTIVE_HIGH;
    let bit = if high { 1 << pin } else { 1 << (pin + 16) };
    unsafe { write_volatile((base + BSRR) as *mut u32, bit) };
}

fn led_lit(base: u32, pin: u32) -> bool {
    let high = unsafe { read_volatile((base + ODR) as *const u32) } & (1 << pin) != 0;
    high == LED_ACTIVE_HIGH
}

fn delay() {
    for _ in 0..200000u32 {
        unsafe {
            let _ = read_volatile((LEDS[0].1 + ODR) as *const u32);
        }
    }
}

#[entry]
fn main() -> ! {
    // GPIOB and USART1 clocks; default VCP1 is USART1 on PB6/PB7, AF7.
    reg_rmw(RCC_AHB2ENR, 0, 1 << 1); // GPIOB
    reg_rmw(RCC_APB2ENR, 0, 1 << 14); // USART1
    pin_af(GPIOB, 6, 7); // USART1_TX -> ST-LINK VCP_RX
    pin_af(GPIOB, 7, 7); // USART1_RX <- ST-LINK VCP_TX
    unsafe {
        write_volatile(USART_BRR, 35 /* 4 MHz / 115200, rounded */);
        write_volatile(USART_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }
    for (_, base, pin) in LEDS {
        reg_rmw(base + MODER, 0b11 << (pin * 2), 0b01 << (pin * 2));
        led_drive(base, pin, false);
    }
    send(b"NUCLEO-WB55RG ready\r\n");

    let mut beat: u32 = 0;
    loop {
        let (name, base, pin) = LEDS[((beat / 2) as usize) % LEDS.len()];
        led_drive(base, pin, beat & 1 == 0);
        send(b"hb=");
        put_dec(beat);
        send(b" ");
        send(name);
        send(if led_lit(base, pin) {
            b"=on\r\n"
        } else {
            b"=off\r\n"
        });
        beat = beat.wrapping_add(1);
        delay();
    }
}
