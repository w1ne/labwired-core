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
//! HC-SR04 distance → ship X: the firmware counts loop iterations while ECHO is
//! high. Because the echo pulse is a fixed number of CPU cycles (sim and HW
//! both run 4 MHz), the count — and therefore the ship position — is identical
//! on silicon and in the simulator.

#![no_std]
#![no_main]
#![allow(static_mut_refs)]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Register map (RM0351) ──────────────────────────────────────────────────
const RCC_AHB2ENR: *mut u32 = 0x4002_104C as *mut u32;
const RCC_APB2ENR: *mut u32 = 0x4002_1060 as *mut u32;

const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_OSPEEDR: *mut u32 = (0x4800_0000 + 0x08) as *mut u32;
const GPIOA_AFRL: *mut u32 = (0x4800_0000 + 0x20) as *mut u32;
const GPIOA_BSRR: *mut u32 = (0x4800_0000 + 0x18) as *mut u32;
const GPIOA_BRR: *mut u32 = (0x4800_0000 + 0x28) as *mut u32;

const GPIOB_MODER: *mut u32 = 0x4800_0400 as *mut u32;
const GPIOB_IDR: *const u32 = (0x4800_0400 + 0x10) as *const u32;
const GPIOB_BRR: *mut u32 = (0x4800_0400 + 0x28) as *mut u32;

const GPIOC_MODER: *mut u32 = 0x4800_0800 as *mut u32;
const GPIOC_BSRR: *mut u32 = (0x4800_0800 + 0x18) as *mut u32;
const GPIOC_BRR: *mut u32 = (0x4800_0800 + 0x28) as *mut u32;

const SPI1_CR1: *mut u16 = 0x4001_3000 as *mut u16;
const SPI1_CR2: *mut u16 = (0x4001_3000 + 0x04) as *mut u16;
const SPI1_SR: *const u16 = (0x4001_3000 + 0x08) as *const u16;
// 8-bit DR access: with DS=8 the L4 SPI packs a 16-bit DR write into TWO
// frames (RM0351 §40.4.9 data packing), sending a spurious 0x00 after each
// byte. A byte (strb) access sends exactly one frame — correct for the PCD8544.
const SPI1_DR: *mut u8 = (0x4001_3000 + 0x0C) as *mut u8;

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

/// Framebuffer at a fixed RAM address (static, not stack) so it can be read
/// back over SWD for bit-exact HW-vs-sim verification.
static mut FB: [u8; W * BANKS] = [0u8; W * BANKS];

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
    // High output speed on SCLK/MOSI for clean edges on real wiring.
    rmw(
        GPIOA_OSPEEDR,
        (0b11 << (SCK * 2)) | (0b11 << (MOSI * 2)),
        (0b11 << (SCK * 2)) | (0b11 << (MOSI * 2)),
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
        write_volatile(SPI1_DR, byte);
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

/// Fill the whole panel with a constant byte (0xFF = all pixels on/dark,
/// 0x00 = all off). Used by the bring-up diagnostic.
fn lcd_fill(byte: u8) {
    lcd_cmd(0x40);
    lcd_cmd(0x80);
    for _ in 0..(W * BANKS) {
        lcd_data(byte);
    }
}

/// Set the contrast (Vop) — extended instruction set, then back to basic.
fn lcd_set_vop(vop: u8) {
    lcd_cmd(0x21);
    lcd_cmd(0x80 | (vop & 0x7F));
    lcd_cmd(0x20);
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

    // Wait for ECHO to rise — short timeout so a missing/idle sensor doesn't
    // stall the animation (returns 0 = "no reading").
    let mut guard = 0u32;
    while !echo_high() {
        guard += 1;
        if guard > 30_000 {
            return 0;
        }
    }
    // Count while high (bounded).
    let mut count = 0u32;
    while echo_high() {
        count += 1;
        if count > 80_000 {
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
const BCOLS: usize = 6;
const BROWS: usize = 3;
const BRICK_W: i32 = 12;
const BRICK_H: i32 = 4;
const BRICK_PX: i32 = 14; // column pitch (6*14 = 84)
const BRICK_PY: i32 = 6; // row pitch
const BRICK_TOP: i32 = 2;
const PADDLE_W: i32 = 16;
const PADDLE_H: i32 = 3;
const PADDLE_Y: i32 = (H as i32) - PADDLE_H; // bottom row
const BALL_SZ: i32 = 2;

/// Breakout: a ball smashes the brick grid; the player moves the paddle (via
/// the HC-SR04 distance) to bounce it back up.
struct Game {
    bricks: [[bool; BCOLS]; BROWS],
    paddle_x: i32,
    bx: i32, // ball top-left
    by: i32,
    vx: i32, // ball velocity (±1)
    vy: i32,
}

impl Game {
    fn new() -> Self {
        Self {
            bricks: [[true; BCOLS]; BROWS],
            paddle_x: (W as i32 - PADDLE_W) / 2,
            bx: W as i32 / 2,
            by: PADDLE_Y - 10,
            vx: 1,
            vy: -1,
        }
    }

    fn any_brick(&self) -> bool {
        self.bricks.iter().any(|r| r.iter().any(|&b| b))
    }

    fn brick_rect(c: usize, r: usize) -> (i32, i32, i32, i32) {
        (
            c as i32 * BRICK_PX + 1,
            BRICK_TOP + r as i32 * BRICK_PY,
            BRICK_W,
            BRICK_H,
        )
    }

    fn step(&mut self, paddle_x: i32) {
        self.paddle_x = paddle_x.clamp(0, W as i32 - PADDLE_W);

        // New wave once every brick is cleared.
        if !self.any_brick() {
            let keep = self.paddle_x;
            *self = Game::new();
            self.paddle_x = keep;
            return;
        }

        // Advance the ball.
        self.bx += self.vx;
        self.by += self.vy;

        // Side + top walls.
        if self.bx <= 0 {
            self.bx = 0;
            self.vx = -self.vx;
        }
        if self.bx >= W as i32 - BALL_SZ {
            self.bx = W as i32 - BALL_SZ;
            self.vx = -self.vx;
        }
        if self.by <= 0 {
            self.by = 0;
            self.vy = -self.vy;
        }

        // Paddle bounce — reflection direction depends on where it lands.
        if self.vy > 0
            && self.by + BALL_SZ >= PADDLE_Y
            && self.by + BALL_SZ <= PADDLE_Y + PADDLE_H
            && self.bx + BALL_SZ > self.paddle_x
            && self.bx < self.paddle_x + PADDLE_W
        {
            self.vy = -self.vy;
            self.by = PADDLE_Y - BALL_SZ;
            let center = self.paddle_x + PADDLE_W / 2;
            self.vx = if self.bx + BALL_SZ / 2 < center {
                -1
            } else {
                1
            };
        }

        // Missed the paddle → relaunch from above it.
        if self.by >= H as i32 {
            self.bx = self.paddle_x + PADDLE_W / 2;
            self.by = PADDLE_Y - 10;
            self.vy = -1;
        }

        // Brick collisions — break one brick per frame.
        'bricks: for r in 0..BROWS {
            for c in 0..BCOLS {
                if !self.bricks[r][c] {
                    continue;
                }
                let (x, y, w, h) = Self::brick_rect(c, r);
                if self.bx + BALL_SZ > x
                    && self.bx < x + w
                    && self.by + BALL_SZ > y
                    && self.by < y + h
                {
                    self.bricks[r][c] = false;
                    self.vy = -self.vy;
                    break 'bricks;
                }
            }
        }
    }

    fn render(&self, fb: &mut [u8; W * BANKS]) {
        for b in fb.iter_mut() {
            *b = 0;
        }
        // Bricks.
        for r in 0..BROWS {
            for c in 0..BCOLS {
                if !self.bricks[r][c] {
                    continue;
                }
                let (x, y, w, h) = Self::brick_rect(c, r);
                rect(fb, x, y, w, h);
            }
        }
        // Ball.
        rect(fb, self.bx, self.by, BALL_SZ, BALL_SZ);
        // Paddle.
        rect(fb, self.paddle_x, PADDLE_Y, PADDLE_W, PADDLE_H);
    }
}

/// Map the echo-pulse count to a paddle X position (clamped in `step`).
fn count_to_paddle_x(count: u32) -> i32 {
    let span = (W as i32 - PADDLE_W).max(1);
    // The polling loop has a fixed overhead before it starts counting ECHO high
    // time. Subtract that floor, scale the remaining pulse width across the
    // playable range, and invert the result so the demo's maximum distance
    // reaches the left edge.
    let raw = (count / 16) as i32;
    let v = span - (((raw - span / 2) * 13) / 4);
    v.clamp(0, span)
}

/// Median of three — rejects a single spurious HC-SR04 reading (the spikes
/// that make the paddle jump). `sum - max - min` is the middle value.
fn median3(a: u32, b: u32, c: u32) -> u32 {
    let mx = a.max(b).max(c);
    let mn = a.min(b).min(c);
    (a + b + c) - mx - mn
}

/// Bring-up diagnostic: sweep contrast while blinking the whole screen
/// black↔clear. If the panel blinks at any Vop, wiring + data path are good
/// (note which Vop looks best); if it never blinks at all, it's a wiring/power
/// problem (most often RST not connected → panel stuck in reset).
#[allow(dead_code)]
fn diagnostic() -> ! {
    loop {
        for &vop in &[0x20u8, 0x28, 0x30, 0x38, 0x3F, 0x48, 0x50, 0x60] {
            lcd_set_vop(vop);
            lcd_fill(0xFF); // all pixels dark
            delay(2_000_000);
            lcd_fill(0x00); // all pixels clear
            delay(2_000_000);
        }
    }
}

/// Live sensor meter for HW bring-up: a 6×6 "alive" square top-left (proves the
/// display + loop run), and a horizontal bar whose length grows with the
/// HC-SR04 echo count (distance). Bar stays empty ⇒ no echo (wiring/power).
#[allow(dead_code)]
fn sensor_debug() -> ! {
    loop {
        let count = measure();
        let len = ((count >> 6) as i32).clamp(0, W as i32);
        unsafe {
            for b in FB.iter_mut() {
                *b = 0;
            }
            rect(&mut FB, 0, 0, 6, 6); // alive marker
            rect(&mut FB, 0, 20, len, 12); // echo bar
            lcd_frame(&FB);
        }
        delay(50_000);
    }
}

#[entry]
fn main() -> ! {
    gpio_init();
    spi_init();
    lcd_init();

    let mut game = Game::new();

    // Deterministic splash frame 0 (centered ship + full invader grid) — also
    // the bit-exact HW/sim verification frame (static FB @ 0x20000000).
    unsafe {
        game.render(&mut FB);
        lcd_frame(&FB);
    }
    delay(1_000_000);

    // Paddle control: HC-SR04 distance, smoothed (median-of-3 to drop
    // spikes + an exponential moving average to tame jitter). A missed echo is
    // treated as "object is at the nearest edge" so the valid minimum distance
    // remains visible even when the polling loop misses that very short pulse.
    let center = (W as i32 - PADDLE_W) / 2;
    let mut filt: i32 = center << 3; // paddle X in 1/8-px fixed point
    let mut c1 = 0u32;
    let mut c2 = 0u32;
    loop {
        let count = measure();
        let mut target = 0;
        if count > 0 {
            if c1 == 0 {
                c1 = count; // seed the window on the first reading
                c2 = count;
            }
            let m = median3(count, c1, c2);
            c2 = c1;
            c1 = count;
            target = count_to_paddle_x(m);
        }
        // EMA, α = 1/4: filt += (target - filt) / 4, in 1/8-px units.
        filt += ((target << 3) - filt) >> 2;
        let paddle = (filt >> 3).clamp(0, W as i32 - PADDLE_W);
        game.step(paddle);
        unsafe {
            game.render(&mut FB);
            lcd_frame(&FB);
        }
        delay(20_000); // frame pacing
    }
}
