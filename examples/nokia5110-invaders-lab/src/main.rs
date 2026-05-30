// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.

//! Space Invaders on a Nokia 5110 (PCD8544), steered by an HC-SR04 — for the
//! NUCLEO-L476RG. The same .elf runs on real silicon and in the LabWired
//! simulator (digital twin).
//!
//! Default 4 MHz MSI clock (no PLL), matching `firmware-l476-demo`.
//!
//! Wiring (Nokia 5110 → Nucleo Arduino headers; 3V3 logic, no level shift):
//!   CLK  → PA5  (SPI1_SCK, AF5)      DIN → PA7 (SPI1_MOSI, AF5)
//!   DC   → PC7                       CE  → PB6        RST → PA9
//!   VCC  → 3V3   BL → 3V3            GND → GND
//! HC-SR04 (5V module; ECHO needs a divider to 3V3 on real hardware):
//!   TRIG → PA8 (output)              ECHO → PB10 (input)
//!
//! Hand distance → ship X: the firmware counts loop iterations while ECHO is
//! high. Because the echo pulse is a fixed number of CPU cycles (sim and HW
//! both run 4 MHz), the count — and therefore the ship position — is identical
//! on silicon and in the simulator.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Register map (RM0351) ──────────────────────────────────────────────────
const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1060 as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_AFRL: *mut u32 = (0x4800_0000 + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (0x4800_0000 + 0x18) as *mut u32;
const GPIOA_BRR: *mut u32 = (0x4800_0000 + 0x28) as *mut u32;

const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_IDR: *const u32 = (0x4800_0400 + 0x10) as *const u32;
const GPIOB_BSRR: *mut u32 = (0x4800_0400 + 0x18) as *mut u32;
const GPIOB_BRR: *mut u32 = (0x4800_0400 + 0x28) as *mut u32;

const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = (0x4800_0800 + 0x18) as *mut u32;
const GPIOC_BRR: *mut u32 = (0x4800_0800 + 0x28) as *mut u32;

const SPI1_CR1: *mut u16 = 0x4001_3000 as *mut u16;
const SPI1_CR2: *mut u16 = (0x4001_3000 + 0x04) as *mut u16;
const SPI1_SR: *const u16 = (0x4001_3000 + 0x08) as *const u16;
const SPI1_DR: *mut u16 = (0x4001_3000 + 0x0C) as *mut u16;

// Pin bits
const SCK: u32 = 5; // PA5
const MOSI: u32 = 7; // PA7
const TRIG: u32 = 8; // PA8
const RST: u32 = 9; // PA9
const CS: u32 = 6; // PB6
const ECHO: u32 = 10; // PB10
const DC: u32 = 7; // PC7

// Display geometry
const W: usize = 84;
const H: usize = 48;
const BANKS: usize = H / 8; // 6

// ── Low-level helpers ───────────────────────────────────────────────────────
#[inline(always)]
fn rmw(ptr: *mut u32, clear: u32, set: u32) {
    unsafe {
        let v = (read_volatile(ptr) & !clear) | set;
        write_volatile(ptr, v);
    }
}

#[inline(always)]
fn delay(n: u32) {
    for _ in 0..n {
        unsafe { core::arch::asm!("nop") }
    }
}

fn gpio_init() {
    // Clock GPIOA/B/C + SPI1.
    rmw(RCC_AHB2ENR, 0, (1 << 0) | (1 << 1) | (1 << 2));
    rmw(RCC_APB2ENR, 0, 1 << 12);
    delay(50);

    // GPIOA: PA5/PA7 = AF (0b10), PA8/PA9 = output (0b01).
    rmw(
        GPIOA_MODER,
        (0b11 << (SCK * 2)) | (0b11 << (MOSI * 2)) | (0b11 << (TRIG * 2)) | (0b11 << (RST * 2)),
        (0b10 << (SCK * 2)) | (0b10 << (MOSI * 2)) | (0b01 << (TRIG * 2)) | (0b01 << (RST * 2)),
    );
    // AFRL: PA5 → AF5, PA7 → AF5 (SPI1).
    rmw(
        GPIOA_AFRL,
        (0xF << (SCK * 4)) | (0xF << (MOSI * 4)),
        (5 << (SCK * 4)) | (5 << (MOSI * 4)),
    );
    // GPIOB: PB6 = output (CS), PB10 = input (ECHO, 0b00).
    rmw(
        GPIOB_MODER,
        (0b11 << (CS * 2)) | (0b11 << (ECHO * 2)),
        0b01 << (CS * 2),
    );
    // GPIOC: PC7 = output (DC).
    rmw(GPIOC_MODER, 0b11 << (DC * 2), 0b01 << (DC * 2));
}

fn spi_init() {
    unsafe {
        // 8-bit data, RX threshold = 1 byte.
        write_volatile(SPI1_CR2, (0x7 << 8) | (1 << 12));
        // MSTR | SSM | SSI | BR=/4 | SPE.
        write_volatile(
            SPI1_CR1,
            (1 << 2) | (1 << 9) | (1 << 8) | (0x1 << 3) | (1 << 6),
        );
    }
}

/// Push one byte out SPI1. Bounded waits: a write-only panel never drives
/// MISO, so we must not block forever on RXNE.
fn spi_write(byte: u8) {
    unsafe {
        for _ in 0..4096 {
            if read_volatile(SPI1_SR) & (1 << 1) != 0 {
                break; // TXE
            }
        }
        write_volatile(SPI1_DR, byte as u16);
        for _ in 0..4096 {
            if read_volatile(SPI1_SR) & (1 << 7) == 0 {
                break; // BSY cleared → transfer done, safe to toggle D/C
            }
        }
    }
}

// ── Pin helpers ─────────────────────────────────────────────────────────────
fn set(bsrr: *mut u32, bit: u32) {
    unsafe { write_volatile(bsrr, 1 << bit) }
}
fn clr(brr: *mut u32, bit: u32) {
    unsafe { write_volatile(brr, 1 << bit) }
}

// ── PCD8544 (Nokia 5110) driver ─────────────────────────────────────────────
fn lcd_cmd(c: u8) {
    clr(GPIOC_BRR, DC); // D/C low = command
    spi_write(c);
}
fn lcd_data(d: u8) {
    set(GPIOC_BSRR, DC); // D/C high = data
    spi_write(d);
}

fn lcd_init() {
    // Reset pulse.
    clr(GPIOA_BRR, RST);
    delay(2000);
    set(GPIOA_BSRR, RST);
    delay(2000);
    clr(GPIOB_BRR, CS); // CS held low (single device on the bus)

    lcd_cmd(0x21); // function set: extended instruction set (H=1)
    lcd_cmd(0xBF); // set Vop (contrast)
    lcd_cmd(0x04); // temperature coefficient
    lcd_cmd(0x14); // bias system 1:48
    lcd_cmd(0x20); // function set: basic instruction set (H=0)
    lcd_cmd(0x0C); // display control: normal mode
}

fn lcd_frame(fb: &[u8; W * BANKS]) {
    lcd_cmd(0x40); // Y = 0
    lcd_cmd(0x80); // X = 0
    for &b in fb.iter() {
        lcd_data(b);
    }
}

// ── HC-SR04: count loop iterations while ECHO is high ───────────────────────
fn echo_high() -> bool {
    unsafe { read_volatile(GPIOB_IDR) & (1 << ECHO) != 0 }
}

/// Trigger a ranging and return a distance proxy: the number of poll
/// iterations ECHO stays high. Larger = farther. Bounded so a missing sensor
/// can't hang the game.
fn measure() -> u32 {
    // 10 µs trigger pulse (well over spec at 4 MHz).
    set(GPIOA_BSRR, TRIG);
    delay(60);
    clr(GPIOA_BRR, TRIG);

    // Wait for ECHO to rise (timeout).
    let mut guard = 0u32;
    while !echo_high() {
        guard += 1;
        if guard > 200_000 {
            return 0;
        }
    }
    // Count while high.
    let mut count = 0u32;
    while echo_high() {
        count += 1;
        if count > 200_000 {
            break;
        }
    }
    count
}

// ── Framebuffer drawing ─────────────────────────────────────────────────────
fn px(fb: &mut [u8; W * BANKS], x: i32, y: i32) {
    if x < 0 || y < 0 || x >= W as i32 || y >= H as i32 {
        return;
    }
    let (x, y) = (x as usize, y as usize);
    fb[(y / 8) * W + x] |= 1 << (y % 8);
}

fn rect(fb: &mut [u8; W * BANKS], x: i32, y: i32, w: i32, h: i32) {
    for dy in 0..h {
        for dx in 0..w {
            px(fb, x + dx, y + dy);
        }
    }
}

// ── Game ─────────────────────────────────────────────────────────────────────
const COLS: usize = 5;
const ROWS: usize = 3;
const ALIEN_W: i32 = 7;
const ALIEN_H: i32 = 5;
const CELL_W: i32 = 15; // column pitch
const CELL_H: i32 = 11; // row pitch
const SHIP_W: i32 = 9;

struct Game {
    alive: [[bool; COLS]; ROWS],
    ax: i32, // alien block left edge
    ay: i32, // alien block top
    dir: i32,
    ship_x: i32,
    bx: i32,
    by: i32,
    bullet_on: bool,
    frame: u32,
}

impl Game {
    fn new() -> Self {
        Self {
            alive: [[true; COLS]; ROWS],
            ax: 4,
            ay: 2,
            dir: 1,
            ship_x: (W as i32 - SHIP_W) / 2,
            bx: 0,
            by: 0,
            bullet_on: false,
            frame: 0,
        }
    }

    fn any_alive(&self) -> bool {
        self.alive.iter().any(|r| r.iter().any(|&a| a))
    }

    fn alien_xy(&self, c: usize, r: usize) -> (i32, i32) {
        (self.ax + c as i32 * CELL_W, self.ay + r as i32 * CELL_H)
    }

    fn step(&mut self, ship_x: i32) {
        self.frame = self.frame.wrapping_add(1);
        self.ship_x = ship_x.clamp(0, W as i32 - SHIP_W);

        if !self.any_alive() {
            *self = {
                let mut g = Game::new();
                g.ship_x = self.ship_x;
                g
            };
            return;
        }

        // March the swarm every 6 frames; bounce + descend at the edges.
        if self.frame % 6 == 0 {
            // Find populated column span for edge detection.
            let mut min_x = W as i32;
            let mut max_x = 0;
            for r in 0..ROWS {
                for c in 0..COLS {
                    if self.alive[r][c] {
                        let (x, _) = self.alien_xy(c, r);
                        min_x = min_x.min(x);
                        max_x = max_x.max(x + ALIEN_W);
                    }
                }
            }
            if (self.dir > 0 && max_x >= W as i32) || (self.dir < 0 && min_x <= 0) {
                self.dir = -self.dir;
                self.ay += 3;
            } else {
                self.ax += self.dir;
            }
        }

        // Auto-fire a bullet from the ship.
        if !self.bullet_on {
            self.bx = self.ship_x + SHIP_W / 2;
            self.by = (H as i32) - 8;
            self.bullet_on = true;
        } else {
            self.by -= 2;
            if self.by < 0 {
                self.bullet_on = false;
            } else {
                // Collision check.
                for r in 0..ROWS {
                    for c in 0..COLS {
                        if !self.alive[r][c] {
                            continue;
                        }
                        let (x, y) = self.alien_xy(c, r);
                        if self.bx >= x
                            && self.bx < x + ALIEN_W
                            && self.by >= y
                            && self.by < y + ALIEN_H
                        {
                            self.alive[r][c] = false;
                            self.bullet_on = false;
                        }
                    }
                }
            }
        }
    }

    fn render(&self, fb: &mut [u8; W * BANKS]) {
        for b in fb.iter_mut() {
            *b = 0;
        }
        // Aliens: a little 7×5 crab.
        for r in 0..ROWS {
            for c in 0..COLS {
                if !self.alive[r][c] {
                    continue;
                }
                let (x, y) = self.alien_xy(c, r);
                rect(fb, x + 1, y, ALIEN_W - 2, ALIEN_H); // body
                px(fb, x, y + 1); // left arm
                px(fb, x + ALIEN_W - 1, y + 1); // right arm
                px(fb, x + 1, y + ALIEN_H); // legs
                px(fb, x + ALIEN_W - 2, y + ALIEN_H);
            }
        }
        // Player ship at the bottom (turret + base).
        let sx = self.ship_x;
        let sy = (H as i32) - 5;
        rect(fb, sx, sy + 2, SHIP_W, 3);
        rect(fb, sx + SHIP_W / 2 - 1, sy, 3, 2);
        // Bullet.
        if self.bullet_on {
            px(fb, self.bx, self.by);
            px(fb, self.bx, self.by + 1);
        }
    }
}

// Map the echo-pulse count to a ship X position. The count range depends on
// the configured distance, but a simple linear map over a sane span keeps the
// ship on screen; clamped in Game::step regardless.
fn count_to_ship_x(count: u32) -> i32 {
    // ~2–60 cm of hand travel covers the screen; tune the divisor for feel.
    let span = (W as i32 - SHIP_W).max(1);
    let v = (count / 16) as i32;
    v.clamp(0, span)
}

#[entry]
fn main() -> ! {
    gpio_init();
    spi_init();
    lcd_init();

    let mut fb = [0u8; W * BANKS];
    let mut game = Game::new();

    loop {
        let count = measure();
        let ship_x = count_to_ship_x(count);
        game.step(ship_x);
        game.render(&mut fb);
        lcd_frame(&fb);
        delay(20_000); // frame pacing
    }
}
