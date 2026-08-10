//! Original indexed art and RGB565 palette for the Doom-like lab.
//!
//! All textures, sprites, weapon art, and HUD glyphs are authored here under
//! MIT/CC0. Do not copy Doom or Freedoom data into this file.

/// Convert 8-bit R,G,B to RGB565.
#[inline]
pub const fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    (((r as u16) & 0xF8) << 8) | (((g as u16) & 0xFC) << 3) | ((b as u16) >> 3)
}

// ── Named colours used by tests and the renderer ────────────────────────────

/// Light stone wall texel.
pub const STONE_LIGHT: u16 = rgb565(160, 160, 160);
/// Dark stone wall texel.
pub const STONE_DARK: u16 = rgb565(96, 96, 96);
/// Ceiling colour.
pub const CEILING: u16 = rgb565(48, 48, 56);
/// Floor colour.
pub const FLOOR: u16 = rgb565(64, 48, 40);
/// Pistol body (dark metal).
pub const PISTOL_DARK: u16 = rgb565(40, 40, 48);
/// Pistol highlight.
pub const PISTOL_LIGHT: u16 = rgb565(120, 120, 128);
/// Muzzle flash.
pub const MUZZLE: u16 = rgb565(255, 220, 64);
/// HUD health red.
pub const HUD_RED: u16 = rgb565(200, 32, 32);
/// HUD ammo yellow.
pub const HUD_YELLOW: u16 = rgb565(220, 200, 48);
/// HUD panel background.
pub const HUD_BG: u16 = rgb565(24, 24, 28);
/// Transparent index for sprites (never drawn).
pub const TRANSPARENT: u8 = 0;

// ── Indexed palette (index 0 = transparent for sprites) ─────────────────────

pub const PALETTE: [u16; 16] = [
    0x0000,       // 0 transparent / black
    STONE_DARK,   // 1
    STONE_LIGHT,  // 2
    rgb565(120, 80, 64),  // 3 brown
    rgb565(180, 40, 40),  // 4 enemy red
    rgb565(80, 160, 80),  // 5 health green
    rgb565(200, 180, 40), // 6 ammo gold
    PISTOL_DARK,  // 7
    PISTOL_LIGHT, // 8
    MUZZLE,       // 9
    HUD_RED,      // 10
    HUD_YELLOW,   // 11
    rgb565(220, 220, 220), // 12 white
    rgb565(16, 16, 20),    // 13 near-black
    rgb565(100, 100, 110), // 14 mid gray
    rgb565(255, 128, 64),  // 15 fire orange
];

// ── 8×8 wall textures (row-major palette indices) ───────────────────────────

/// Stone brick pattern.
pub const WALL_STONE: [u8; 64] = [
    1, 2, 2, 1, 1, 2, 2, 1,
    2, 2, 1, 1, 2, 2, 1, 1,
    2, 1, 1, 2, 2, 1, 1, 2,
    1, 1, 2, 2, 1, 1, 2, 2,
    1, 2, 2, 1, 1, 2, 2, 1,
    2, 2, 1, 1, 2, 2, 1, 1,
    2, 1, 1, 2, 2, 1, 1, 2,
    1, 1, 2, 2, 1, 1, 2, 2,
];

/// Darker variant for door faces.
pub const WALL_DOOR: [u8; 64] = [
    3, 3, 1, 3, 3, 1, 3, 3,
    3, 1, 1, 1, 1, 1, 1, 3,
    1, 1, 14, 14, 14, 14, 1, 1,
    3, 1, 14, 12, 12, 14, 1, 3,
    3, 1, 14, 12, 12, 14, 1, 3,
    1, 1, 14, 14, 14, 14, 1, 1,
    3, 1, 1, 1, 1, 1, 1, 3,
    3, 3, 1, 3, 3, 1, 3, 3,
];

// ── 16×16 enemy billboard (0 = transparent) ─────────────────────────────────

pub const ENEMY_W: usize = 16;
pub const ENEMY_H: usize = 16;
/// Crude demon silhouette — original geometric art, not from Doom.
pub const ENEMY_SPRITE: [u8; ENEMY_W * ENEMY_H] = [
    0, 0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0,
    0, 0, 0, 4, 4, 12, 4, 4, 4, 4, 12, 4, 4, 0, 0, 0,
    0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0,
    0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0,
    0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0,
    0, 0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0, 0,
    0, 0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0, 0,
    0, 0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0, 0,
    0, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 0,
    0, 4, 4, 0, 4, 4, 4, 4, 4, 4, 4, 4, 0, 4, 4, 0,
    0, 0, 0, 0, 4, 4, 0, 0, 0, 0, 4, 4, 0, 0, 0, 0,
    0, 0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0, 0,
    0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0,
    0, 0, 4, 4, 0, 0, 0, 0, 0, 0, 0, 0, 4, 4, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];

// ── Pickup sprites 8×8 ──────────────────────────────────────────────────────

pub const PICKUP_W: usize = 8;
pub const PICKUP_H: usize = 8;

pub const HEALTH_SPRITE: [u8; 64] = [
    0, 0, 0, 5, 5, 0, 0, 0,
    0, 0, 5, 5, 5, 5, 0, 0,
    0, 5, 5, 12, 12, 5, 5, 0,
    5, 5, 12, 12, 12, 12, 5, 5,
    5, 5, 12, 12, 12, 12, 5, 5,
    0, 5, 5, 12, 12, 5, 5, 0,
    0, 0, 5, 5, 5, 5, 0, 0,
    0, 0, 0, 5, 5, 0, 0, 0,
];

pub const AMMO_SPRITE: [u8; 64] = [
    0, 0, 6, 6, 6, 6, 0, 0,
    0, 6, 6, 12, 12, 6, 6, 0,
    0, 6, 12, 6, 6, 12, 6, 0,
    0, 6, 12, 6, 6, 12, 6, 0,
    0, 6, 12, 6, 6, 12, 6, 0,
    0, 6, 6, 12, 12, 6, 6, 0,
    0, 0, 6, 6, 6, 6, 0, 0,
    0, 0, 0, 6, 6, 0, 0, 0,
];

// ── Pistol overlay 32×24 (drawn bottom-center) ──────────────────────────────

pub const PISTOL_W: usize = 32;
pub const PISTOL_H: usize = 24;

/// Sparse pistol shape — dark metal body, light slide.
pub const PISTOL_SPRITE: [u8; PISTOL_W * PISTOL_H] = {
    let mut s = [0u8; PISTOL_W * PISTOL_H];
    // Barrel
    let mut x = 14;
    while x < 28 {
        s[8 * PISTOL_W + x] = 7;
        s[9 * PISTOL_W + x] = 8;
        s[10 * PISTOL_W + x] = 7;
        x += 1;
    }
    // Body
    let mut y = 10;
    while y < 18 {
        let mut x = 10;
        while x < 20 {
            s[y * PISTOL_W + x] = if (x + y) % 2 == 0 { 7 } else { 8 };
            x += 1;
        }
        y += 1;
    }
    // Grip
    y = 16;
    while y < 23 {
        let mut x = 12;
        while x < 17 {
            s[y * PISTOL_W + x] = 7;
            x += 1;
        }
        y += 1;
    }
    s
};

// ── 3×5 digit font (bit rows, bit0 = left) ──────────────────────────────────

/// Digits 0-9 as 3-wide columns packed into 5 row bits each (col-major nibbles).
/// Each digit is 3 columns × 5 rows; stored as 3 bytes (one per column, bit0=top).
pub const DIGIT_FONT: [[u8; 3]; 10] = [
    [0x1F, 0x11, 0x1F], // 0
    [0x00, 0x1F, 0x00], // 1
    [0x1D, 0x15, 0x17], // 2
    [0x15, 0x15, 0x1F], // 3
    [0x07, 0x04, 0x1F], // 4
    [0x17, 0x15, 0x1D], // 5
    [0x1F, 0x15, 0x1D], // 6
    [0x01, 0x01, 0x1F], // 7
    [0x1F, 0x15, 0x1F], // 8
    [0x17, 0x15, 0x1F], // 9
];

#[inline]
pub fn palette_color(idx: u8) -> u16 {
    PALETTE[(idx as usize) & 0x0F]
}
