// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// KW41Z "cattle activity tag" display firmware. Reads the on-board FXOS8700
// accelerometer over the Kinetis I2C (I2C1) and renders a cute activity-
// reactive cow face onto a Nokia-5110 (PCD8544) LCD over the Kinetis DSPI
// (SPI0), with the display's D/C line driven from a GPIOC output. Also
// echoes the readings over LPUART0 so the boot is observable as text.
//
// Boots on the LabWired KW41Z model unchanged: the behavioural Kinetis I2C /
// DSPI / GPIO peripherals route exactly the register pokes below, and the
// PCD8544 device model renders the framebuffer the firmware streams out.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── LPUART0 (console) ────────────────────────────────────────────────────────
const LPUART0_STAT: *mut u32 = 0x4005_4004 as *mut u32;
const LPUART0_CTRL: *mut u32 = 0x4005_4008 as *mut u32;
const LPUART0_DATA: *mut u32 = 0x4005_400C as *mut u32;
const STAT_TDRE: u32 = 1 << 23;
const CTRL_TE: u32 = 1 << 19;

// ── I2C1 (FXOS8700 @ 0x1f) — byte registers ──────────────────────────────────
const I2C1_C1: *mut u8 = 0x4006_7002 as *mut u8;
const I2C1_S: *mut u8 = 0x4006_7003 as *mut u8;
const I2C1_D: *mut u8 = 0x4006_7004 as *mut u8;
const C1_IICEN: u8 = 0x80;
const C1_MST: u8 = 0x20;
const C1_TX: u8 = 0x10;
const C1_TXAK: u8 = 0x08;
const C1_RSTA: u8 = 0x04;
const S_IICIF: u8 = 0x02;

const FXOS_ADDR: u8 = 0x1f;
const FXOS_WHOAMI: u8 = 0x0D;
const FXOS_OUT_X_MSB: u8 = 0x01;
const FXOS_CTRL_REG1: u8 = 0x2A;

// ── DSPI / SPI0 (PCD8544) — 32-bit registers ─────────────────────────────────
const SPI0_MCR: *mut u32 = 0x4002_C000 as *mut u32;
const SPI0_SR: *mut u32 = 0x4002_C02C as *mut u32;
const SPI0_PUSHR: *mut u32 = 0x4002_C034 as *mut u32;
const MCR_MSTR: u32 = 0x8000_0000;
const SR_TFFF: u32 = 0x0200_0000;
const SR_TCF: u32 = 0x8000_0000;

// ── GPIOC (PCD8544 D/C line = PTC0) — 32-bit PDOR ─────────────────────────────
const GPIOC_PDOR: *mut u32 = 0x400F_F080 as *mut u32;
const DC_BIT: u32 = 1 << 0;

const LCD_W: usize = 84;
const LCD_H: usize = 48;
const LCD_BANKS: usize = 6;

type FrameBuffer = [u8; LCD_BANKS * LCD_W];

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(LPUART0_CTRL, CTRL_TE);
        write_volatile(SPI0_MCR, MCR_MSTR); // DSPI master, HALT cleared

        // Probe the sensor, then put it in active mode (the model returns data
        // regardless, but this is the real bring-up a driver does).
        let who = i2c_read_one(FXOS_ADDR, FXOS_WHOAMI);
        i2c_write_one(FXOS_ADDR, FXOS_CTRL_REG1, 0x01); // ACTIVE

        pcd8544_init();

        print(b"KW41Z_LCD_OK whoami=");
        print_hex(who);
        print(b"\n");

        let mut frame: u32 = 0;
        loop {
            // Read the 6 accel bytes (X/Y/Z MSB:LSB), 14-bit left-justified.
            let mut raw = [0u8; 6];
            i2c_read(FXOS_ADDR, FXOS_OUT_X_MSB, &mut raw);
            let ax = ((raw[0] as i16) << 8 | raw[1] as i16) >> 2;
            let ay = ((raw[2] as i16) << 8 | raw[3] as i16) >> 2;
            let az = ((raw[4] as i16) << 8 | raw[5] as i16) >> 2;
            let _ = az; // Z stays ~constant (gravity) at rest; X/Y drive the mood.

            let activity = axis_len(ax) + axis_len(ay);
            let active = activity > ACTIVE_THRESHOLD;

            render_cow(activity, active, frame);
            frame = frame.wrapping_add(1);

            print(b"X=");
            print_dec(ax);
            print(b" Y=");
            print_dec(ay);
            print(b" Z=");
            print_dec(az);
            print(b" ACT=");
            print_dec(activity as i16);
            print(if active {
                b" MOOD=ACTIVE\n"
            } else {
                b" MOOD=CALM\n"
            });

            // Coarse pacing so the trace shows a few frames.
            for _ in 0..2000 {
                let _ = read_volatile(LPUART0_STAT);
            }
        }
    }
}

// ── PCD8544 ───────────────────────────────────────────────────────────────────

unsafe fn dc(level: bool) {
    write_volatile(GPIOC_PDOR, if level { DC_BIT } else { 0 });
}

unsafe fn spi_send(byte: u8) {
    while read_volatile(SPI0_SR) & SR_TFFF == 0 {}
    write_volatile(SPI0_PUSHR, byte as u32);
    while read_volatile(SPI0_SR) & SR_TCF == 0 {}
    write_volatile(SPI0_SR, SR_TCF); // clear TCF
}

unsafe fn pcd_cmd(c: u8) {
    dc(false);
    spi_send(c);
}

unsafe fn pcd_data(d: u8) {
    dc(true);
    spi_send(d);
}

unsafe fn pcd8544_init() {
    pcd_cmd(0x21); // function set: extended
    pcd_cmd(0xB1); // set Vop (contrast)
    pcd_cmd(0x04); // temperature coefficient
    pcd_cmd(0x14); // bias system
    pcd_cmd(0x20); // function set: basic
    pcd_cmd(0x0C); // display control: normal mode
}

// ── Cow face rendering ───────────────────────────────────────────────────────
//
// The whole 84x48 panel is composed as an in-RAM framebuffer (6 banks x 84
// columns, matching the PCD8544's own vertical-byte addressing) and then
// streamed out in one pass, exactly the way the original bar-graph blitted
// its bytes. Every shape below is drawn with pure-integer primitives (line /
// midpoint-ellipse outline / ellipse fill) — deterministic, no floating
// point, no RNG. The mood flips the whole composition, not just details: the
// calm cow grazes (head low, grass, heart banner) while the active cow pops
// its head up, wobbles ±2 px per frame and gets an inverted "MOO!" band, so
// the reaction is obvious even on a thumbnail-sized render.
//
// At rest the FXOS8700 reports ~0 on X/Y (gravity loads onto Z), so summing
// the raw X/Y magnitudes IS the deviation from the resting pose — no extra
// bias subtraction needed. `ACTIVE_THRESHOLD` is the crossover between the
// "calm" and "active" moods.
const ACTIVE_THRESHOLD: i32 = 32;

/// Render the cow reacting to the activity level, then blit the whole
/// framebuffer out over DSPI (same Y=0/X=0 + streamed-bytes protocol the
/// bar-graph used).
///
/// The two moods are deliberately macro-different so the change reads even on
/// a thumbnail-sized render of the 84x48 panel:
///   calm   → head drawn LOW (grazing), grass tufts, heart + "MOO :)" banner
///   active → head drawn HIGH, wide eyes, motion lines, INVERTED "MOO!" band,
///            and the whole sprite bounces ±2 px per rendered frame.
unsafe fn render_cow(activity: i32, active: bool, frame: u32) {
    let mut fb: FrameBuffer = [0u8; LCD_BANKS * LCD_W];

    // Grazing = head low; alert = head up. Active frames also wobble
    // horizontally so a shaken cow visibly animates.
    let dx = if active {
        if frame & 1 == 0 {
            2
        } else {
            -2
        }
    } else {
        0
    };
    let cy_head = if active { 16 } else { 22 };

    let (cx, cy) = draw_static_cow(&mut fb, dx, cy_head);
    draw_eyes(&mut fb, cx, cy, active);
    if active {
        draw_motion_marks(&mut fb, cy);
        // Inverted banner: a solid band with "MOO!" knocked out of it.
        fill_rect(&mut fb, 0, 37, LCD_W as i32 - 1, 44);
        draw_text_inverted(&mut fb, b"MOO!", 38);
    } else {
        draw_grass(&mut fb);
        draw_glyph(&mut fb, &GLYPH_HEART, 20, 38);
        draw_text(&mut fb, b"MOO :)", 38);
    }
    draw_meter(&mut fb, activity, active);

    pcd_cmd(0x40); // Y = 0
    pcd_cmd(0x80); // X = 0
    for byte in fb.iter() {
        pcd_data(*byte);
    }
}

/// Scale a raw 14-bit accel axis reading down to the 0..=LCD_W range used for
/// both the activity meter and the calm/active threshold.
fn axis_len(v: i16) -> i32 {
    let m = (v.unsigned_abs() as i32) >> 4;
    if m > LCD_W as i32 {
        LCD_W as i32
    } else {
        m
    }
}

// NOTE: several of these primitives are `#[inline(always)]`. This started as
// a workaround for a real divergence found while bringing up the cow: with a
// plain (non-inlined) 5-argument `fn draw_ellipse_outline(fb, cx, cy, rx, ry)`
// — where the 5th arg (ry) is stack-passed per AAPCS — the *simulator*
// rendered a blank outline (0 pixels) while the identical logic, built
// natively for the host, was pixel-perfect. Forcing inlining (so the call
// site never materializes a stack-passed argument) made the simulator's
// output match the host reference exactly. Kept on the small hot-path
// helpers too, both to sidestep the same class of bug and because inlining
// tiny per-pixel functions is the right call for an embedded framebuffer
// renderer anyway.
#[inline(always)]
fn set_pixel(fb: &mut FrameBuffer, x: i32, y: i32) {
    if x < 0 || y < 0 || x as usize >= LCD_W || y as usize >= LCD_H {
        return;
    }
    let (xu, yu) = (x as usize, y as usize);
    fb[(yu / 8) * LCD_W + xu] |= 1 << (yu % 8);
}

#[inline(always)]
fn clear_pixel(fb: &mut FrameBuffer, x: i32, y: i32) {
    if x < 0 || y < 0 || x as usize >= LCD_W || y as usize >= LCD_H {
        return;
    }
    let (xu, yu) = (x as usize, y as usize);
    fb[(yu / 8) * LCD_W + xu] &= !(1 << (yu % 8));
}

#[inline(always)]
fn fill_rect(fb: &mut FrameBuffer, x0: i32, y0: i32, x1: i32, y1: i32) {
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            set_pixel(fb, x, y);
            x += 1;
        }
        y += 1;
    }
}

#[inline(always)]
fn draw_line(fb: &mut FrameBuffer, x0: i32, y0: i32, x1: i32, y1: i32) {
    let (mut x0, mut y0) = (x0, y0);
    let dx = (x1 - x0).abs();
    let sx: i32 = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy: i32 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        set_pixel(fb, x0, y0);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

#[inline(always)]
fn plot_ellipse_points(fb: &mut FrameBuffer, cx: i32, cy: i32, px: i32, py: i32) {
    set_pixel(fb, cx + px, cy + py);
    set_pixel(fb, cx - px, cy + py);
    set_pixel(fb, cx + px, cy - py);
    set_pixel(fb, cx - px, cy - py);
}

/// Single-pixel midpoint ellipse outline (integer-only Bresenham variant).
#[inline(always)]
fn draw_ellipse_outline(fb: &mut FrameBuffer, cx: i32, cy: i32, rx: i32, ry: i32) {
    let mut x = 0i32;
    let mut y = ry;
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let two_rx2 = 2 * rx2;
    let two_ry2 = 2 * ry2;
    let mut p = ry2 - rx2 * ry + rx2 / 4;
    let mut dx = 2 * ry2 * x;
    let mut dy = 2 * rx2 * y;

    while dx < dy {
        plot_ellipse_points(fb, cx, cy, x, y);
        x += 1;
        dx += two_ry2;
        if p < 0 {
            p += dx + ry2;
        } else {
            y -= 1;
            dy -= two_rx2;
            p += dx - dy + ry2;
        }
    }
    let mut p2 = ry2 * (2 * x + 1) * (2 * x + 1) / 4 + rx2 * (y - 1) * (y - 1) - rx2 * ry2;
    while y >= 0 {
        plot_ellipse_points(fb, cx, cy, x, y);
        y -= 1;
        dy -= two_rx2;
        if p2 > 0 {
            p2 += rx2 - dy;
        } else {
            x += 1;
            dx += two_ry2;
            p2 += dx - dy + rx2;
        }
    }
}

#[inline(always)]
fn draw_ellipse_fill(fb: &mut FrameBuffer, cx: i32, cy: i32, rx: i32, ry: i32) {
    let rx2 = rx * rx;
    let ry2 = ry * ry;
    let mut yy = -ry;
    while yy <= ry {
        let mut xx = -rx;
        while xx <= rx {
            if xx * xx * ry2 + yy * yy * rx2 <= rx2 * ry2 {
                set_pixel(fb, cx + xx, cy + yy);
            }
            xx += 1;
        }
        yy += 1;
    }
}

/// A tiny 5-row bitmap glyph. `rows` are ASCII art ('#'/'.') so the source is
/// self-documenting — this is the "hand-authored sprite table" the design
/// brief asks for, just split per-character instead of one giant bitmap.
struct Glyph {
    width: i32,
    rows: [&'static [u8]; 5],
}

const GLYPH_SPACE: Glyph = Glyph {
    width: 3,
    rows: [b"...", b"...", b"...", b"...", b"..."],
};
const GLYPH_M: Glyph = Glyph {
    width: 5,
    rows: [b"#...#", b"##.##", b"#.#.#", b"#...#", b"#...#"],
};
const GLYPH_O: Glyph = Glyph {
    width: 5,
    rows: [b".###.", b"#...#", b"#...#", b"#...#", b".###."],
};
const GLYPH_EXCL: Glyph = Glyph {
    width: 1,
    rows: [b"#", b"#", b"#", b".", b"#"],
};
const GLYPH_COLON: Glyph = Glyph {
    width: 2,
    rows: [b"..", b"##", b"..", b"##", b".."],
};
const GLYPH_RPAREN: Glyph = Glyph {
    // Bulges RIGHT of its endpoints — a ')' smile, not a '(' frown.
    width: 3,
    rows: [b"#..", b".#.", b"..#", b".#.", b"#.."],
};
const GLYPH_HEART: Glyph = Glyph {
    width: 5,
    rows: [b".#.#.", b"#####", b"#####", b".###.", b"..#.."],
};

fn glyph_for(c: u8) -> &'static Glyph {
    match c {
        b'M' => &GLYPH_M,
        b'O' => &GLYPH_O,
        b'!' => &GLYPH_EXCL,
        b':' => &GLYPH_COLON,
        b')' => &GLYPH_RPAREN,
        _ => &GLYPH_SPACE,
    }
}

fn draw_glyph(fb: &mut FrameBuffer, g: &Glyph, x0: i32, y0: i32) {
    for (ry, row) in g.rows.iter().enumerate() {
        for (rx, b) in row.iter().enumerate() {
            if *b == b'#' {
                set_pixel(fb, x0 + rx as i32, y0 + ry as i32);
            }
        }
    }
}

fn text_width(s: &[u8]) -> i32 {
    let mut w = 0;
    for &c in s {
        w += glyph_for(c).width + 1;
    }
    if w > 0 {
        w - 1
    } else {
        0
    }
}

/// Draw a banner of text centered horizontally, top-left at row `y0`.
fn draw_text(fb: &mut FrameBuffer, s: &[u8], y0: i32) {
    let total = text_width(s);
    let mut x = (LCD_W as i32 - total) / 2;
    for &c in s {
        let g = glyph_for(c);
        draw_glyph(fb, g, x, y0);
        x += g.width + 1;
    }
}

/// Knock a centered banner OUT of an already-filled band (inverted text).
fn draw_text_inverted(fb: &mut FrameBuffer, s: &[u8], y0: i32) {
    let total = text_width(s);
    let mut x = (LCD_W as i32 - total) / 2;
    for &c in s {
        let g = glyph_for(c);
        for (ry, row) in g.rows.iter().enumerate() {
            for (rx, b) in row.iter().enumerate() {
                if *b == b'#' {
                    clear_pixel(fb, x + rx as i32, y0 + ry as i32);
                }
            }
        }
        x += g.width + 1;
    }
}

/// A few 'Λ' grass tufts in the bottom corners, shown only while grazing.
fn draw_grass(fb: &mut FrameBuffer) {
    for &x in &[2i32, 10, 66, 74] {
        draw_line(fb, x, 44, x + 2, 40);
        draw_line(fb, x + 4, 44, x + 2, 40);
    }
}

/// The fixed part of the sprite: rounded head, two ears, horn/tuft ticks, a
/// muzzle oval with nostrils, and two spots. The caller positions the head
/// (`dx` wobble, `cy` height — low while grazing, high when alert). Returns
/// the head center so eyes can be positioned relative to it. Eyes are drawn
/// separately since they are the one part of the "face" that changes with
/// mood.
fn draw_static_cow(fb: &mut FrameBuffer, dx: i32, cy: i32) -> (i32, i32) {
    let cx = 42 + dx;

    // Head outline.
    draw_ellipse_outline(fb, cx, cy, 25, 15);

    // Ears, pushed outward so they only just kiss the head's silhouette.
    draw_ellipse_outline(fb, cx - 33, cy - 6, 8, 6);
    draw_ellipse_outline(fb, cx + 33, cy - 6, 8, 6);

    // Horn / tuft ticks above the head's top curve (clip harmlessly at the
    // panel edge on the highest active head position).
    let top = cy - 19;
    draw_line(fb, cx - 7, top, cx - 4, top + 4);
    draw_line(fb, cx - 3, top, cx - 4, top + 4);
    draw_line(fb, cx + 7, top, cx + 4, top + 4);
    draw_line(fb, cx + 3, top, cx + 4, top + 4);

    // Muzzle: outline oval nested inside the head, with two nostril dots.
    let (mx, my) = (cx, cy + 9);
    draw_ellipse_outline(fb, mx, my, 11, 6);
    draw_ellipse_fill(fb, mx - 5, my - 1, 2, 2);
    draw_ellipse_fill(fb, mx + 5, my - 1, 2, 2);

    // Two spots, tucked inside the head away from the other features.
    draw_ellipse_fill(fb, cx - 17, cy + 3, 3, 2);
    draw_ellipse_fill(fb, cx + 17, cy - 9, 3, 2);

    (cx, cy)
}

/// Calm = content, half-closed eyes (a short horizontal line each).
/// Active = wide open eyes (filled circles).
fn draw_eyes(fb: &mut FrameBuffer, cx: i32, cy: i32, active: bool) {
    if active {
        draw_ellipse_fill(fb, cx - 11, cy - 4, 3, 3);
        draw_ellipse_fill(fb, cx + 11, cy - 4, 3, 3);
    } else {
        draw_line(fb, cx - 14, cy - 3, cx - 8, cy - 3);
        draw_line(fb, cx + 8, cy - 3, cx + 14, cy - 3);
    }
}

/// Small "shaken" speed-lines flanking the ears, only shown when active.
fn draw_motion_marks(fb: &mut FrameBuffer, cy: i32) {
    for i in 0..3i32 {
        let yb = cy - 6 + i * 5;
        draw_line(fb, 2, yb, 8, yb - 2);
        draw_line(fb, LCD_W as i32 - 3, yb, LCD_W as i32 - 9, yb - 2);
    }
}

/// Activity meter along the very bottom edge: a full-width track line (row
/// 46) plus a fill proportional to `level` (0..=LCD_W) — one row calm, two
/// rows (chunkier) while active.
fn draw_meter(fb: &mut FrameBuffer, level: i32, active: bool) {
    let level = level.clamp(0, LCD_W as i32);
    for x in 0..LCD_W as i32 {
        set_pixel(fb, x, 46);
        if x < level {
            set_pixel(fb, x, 47);
            if active {
                set_pixel(fb, x, 45);
            }
        }
    }
}

// ── Kinetis I2C ────────────────────────────────────────────────────────────────

unsafe fn i2c_wait_clear() {
    while read_volatile(I2C1_S) & S_IICIF == 0 {}
    write_volatile(I2C1_S, S_IICIF);
}

unsafe fn i2c_read(addr: u8, reg: u8, buf: &mut [u8]) {
    // START + write register pointer.
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX);
    write_volatile(I2C1_D, addr << 1);
    i2c_wait_clear();
    write_volatile(I2C1_D, reg);
    i2c_wait_clear();
    // Repeated START + read address.
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX | C1_RSTA);
    write_volatile(I2C1_D, (addr << 1) | 1);
    i2c_wait_clear();
    // Enter receive. NAK immediately if a single byte is requested.
    let n = buf.len();
    if n <= 1 {
        write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TXAK);
    } else {
        write_volatile(I2C1_C1, C1_IICEN | C1_MST);
    }
    let _ = read_volatile(I2C1_D); // dummy read kicks the first byte
    i2c_wait_clear();
    for (i, byte) in buf.iter_mut().enumerate().take(n) {
        if i == n - 1 {
            // STOP before the final byte read.
            write_volatile(I2C1_C1, C1_IICEN);
        } else if i == n - 2 {
            // NAK the byte after this one (the last).
            write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TXAK);
        }
        *byte = read_volatile(I2C1_D);
        if i != n - 1 {
            i2c_wait_clear();
        }
    }
}

unsafe fn i2c_read_one(addr: u8, reg: u8) -> u8 {
    let mut b = [0u8; 1];
    i2c_read(addr, reg, &mut b);
    b[0]
}

unsafe fn i2c_write_one(addr: u8, reg: u8, val: u8) {
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX);
    write_volatile(I2C1_D, addr << 1);
    i2c_wait_clear();
    write_volatile(I2C1_D, reg);
    i2c_wait_clear();
    write_volatile(I2C1_D, val);
    i2c_wait_clear();
    write_volatile(I2C1_C1, C1_IICEN); // STOP
}

// ── Console helpers ─────────────────────────────────────────────────────────

unsafe fn print(s: &[u8]) {
    for &b in s {
        while read_volatile(LPUART0_STAT) & STAT_TDRE == 0 {}
        write_volatile(LPUART0_DATA, b as u32);
    }
}

unsafe fn print_hex(v: u8) {
    let hex = b"0123456789ABCDEF";
    print(&[hex[(v >> 4) as usize], hex[(v & 0xF) as usize]]);
}

unsafe fn print_dec(v: i16) {
    let mut n = v;
    if n < 0 {
        print(b"-");
        n = -n;
    }
    let mut buf = [0u8; 6];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    print(&buf[i..]);
}
