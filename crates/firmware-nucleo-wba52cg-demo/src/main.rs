#![no_std]
#![no_main]

//! NUCLEO-WBA52CG demo firmware: a running light over the board's user LED(s) plus a
//! heartbeat line on the console USART, in bare registers (no HAL).
//!
//! LD1 blue PB4, LD2 green PA9, LD3 red PB8, ACTIVE LOW (the LED is lit while the pin is low, as in the Zephyr board description). The console is USART1 on PB12 (AF8, ST-LINK VCP), 115200 8N1 from the 16 MHz HSI16.
//!
//! Each beat lights ONE LED, prints `hb=<n> <led>=on`, then switches it off and
//! prints `... =off`. The state in the printed line is READ BACK from the GPIO
//! ODR register after the write, so the text reports the pad the firmware
//! actually drove and not what it intended to drive.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RCC (STM32WBA, RM0493): AHB2ENR gates the GPIO ports, APB2ENR gates USART1.
const RCC: u32 = 0x4602_0C00;
const RCC_AHB2ENR: u32 = RCC + 0x8C;
const RCC_APB2ENR: u32 = RCC + 0xA4;

// USART1 (v2 block, ISR/TDR).
const USART: u32 = 0x4001_3800;
const USART_ISR: *const u32 = (USART + 0x1C) as *const u32;
const USART_TDR: *mut u32 = (USART + 0x28) as *mut u32;
const USART_BRR: *mut u32 = (USART + 0x0C) as *mut u32;
const USART_CR1: *mut u32 = USART as *mut u32;
const TXE: u32 = 1 << 7;
const TC: u32 = 1 << 6;
const TCIE: u32 = 1 << 6;

// NVIC word 1 covers lines 32..63; USART1 is line 46 (RM0493).
const NVIC_ICER1: *mut u32 = 0xE000_E184 as *mut u32;
const NVIC_ISPR1: *const u32 = 0xE000_E204 as *const u32;
const NVIC_ICPR1: *mut u32 = 0xE000_E284 as *mut u32;
/// Lines 40..47 (bits 8..15 of word 1) bracket USART1 line 46.
const NVIC_WINDOW: u32 = 0xFF << 8;
const GPIOA: u32 = 0x4202_0000;
const GPIOB: u32 = 0x4202_0400;

/// GPIO register offsets (STM32 GPIO v2 block: MODER/ODR/BSRR).
const MODER: u32 = 0x00;
const AFRL: u32 = 0x20;
const AFRH: u32 = 0x24;
const ODR: u32 = 0x14;
const BSRR: u32 = 0x18;

/// NUCLEO-WBA52CG: LD1 blue = PB4, LD2 green = PA9, LD3 red = PB8, active low.
const LED_ACTIVE_HIGH: bool = false;
/// User LEDs, in running-light order: (console name, GPIO port base, pin).
const LEDS: [(&[u8], u32, u32); 3] = [(b"ld1", GPIOB, 4), (b"ld2", GPIOA, 9), (b"ld3", GPIOB, 8)];

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

fn put_hex8(v: u32) {
    for shift in [4, 0] {
        let d = ((v >> shift) & 0xF) as u8;
        putc(if d < 10 { b'0' + d } else { b'a' + d - 10 });
    }
}

/// With TX idle (TC set), TCIE must pend only USART1's NVIC line 46 in the
/// window 40..47; clearing TCIE and ICPR must release it. The lines stay
/// masked in ISER, so no handler runs.
fn check_usart1_irq() {
    unsafe {
        while read_volatile(USART_ISR) & TC == 0 {}
        write_volatile(NVIC_ICER1, NVIC_WINDOW);
        write_volatile(NVIC_ICPR1, NVIC_WINDOW);
        reg_rmw(USART, 0, TCIE);
        let pend = (read_volatile(NVIC_ISPR1) & NVIC_WINDOW) >> 8;
        reg_rmw(USART, TCIE, 0);
        write_volatile(NVIC_ICPR1, NVIC_WINDOW);
        let rel = (read_volatile(NVIC_ISPR1) & NVIC_WINDOW) >> 8;
        send(b"usart1 tcie ispr40=");
        put_hex8(pend);
        send(b" rel=");
        put_hex8(rel);
        send(if pend == 1 << 6 && rel == 0 {
            b" ok\r\n"
        } else {
            b" FAIL\r\n"
        });
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
    // GPIO clocks and the console USART clock, then the TX pin and the USART.
    reg_rmw(RCC_AHB2ENR, 0, (1 << 0) | (1 << 1)); // GPIOA | GPIOB
    reg_rmw(RCC_APB2ENR, 0, 1 << 14); // USART1
    pin_af(GPIOB, 12, 8);
    unsafe {
        write_volatile(USART_BRR, 139 /* 16 MHz / 115200 */);
        write_volatile(USART_CR1, (1 << 0) | (1 << 3)); // UE | TE
    }
    for (_, base, pin) in LEDS {
        reg_rmw(base + MODER, 0b11 << (pin * 2), 0b01 << (pin * 2));
        led_drive(base, pin, false);
    }
    send(b"NUCLEO-WBA52CG ready\r\n");
    check_usart1_irq();

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
