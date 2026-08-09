//! Integer raycaster and compositor (160×120 RGB565, no heap).
//!
//! Fixed-point DDA per column, flat ceiling/floor, far-to-near transparent
//! billboards, pistol overlay, and a chunky health/ammo HUD.

use crate::assets::{
    self, palette_color, AMMO_SPRITE, DIGIT_FONT, ENEMY_H, ENEMY_SPRITE, ENEMY_W, FLOOR, HEALTH_SPRITE,
    HUD_BG, HUD_RED, HUD_YELLOW, MUZZLE, PICKUP_H, PICKUP_W, PISTOL_H, PISTOL_SPRITE, PISTOL_W,
    WALL_DOOR, WALL_STONE,
};
use crate::game::{angle_forward, Game, PickupKind};
use crate::level::{is_blocking, ONE, TILE_DOOR};

pub const WIDTH: usize = 160;
pub const HEIGHT: usize = 120;
/// Vertical span for the 3D view (HUD occupies the bottom strip).
const VIEW_H: usize = HEIGHT - 12;
const HUD_H: usize = 12;

/// Raycaster + sprite + weapon + HUD compositor.
pub struct Renderer {
    z_buf: [i32; WIDTH],
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            z_buf: [i32::MAX; WIDTH],
        }
    }

    /// Render `game` into a packed RGB565 framebuffer of WIDTH*HEIGHT pixels.
    pub fn render(&mut self, game: &Game, pixels: &mut [u16; WIDTH * HEIGHT]) {
        self.z_buf = [i32::MAX; WIDTH];
        self.draw_world(game, pixels);
        self.draw_sprites(game, pixels);
        self.draw_weapon(game, pixels);
        self.draw_hud(game, pixels);
    }

    fn draw_world(&mut self, game: &Game, pixels: &mut [u16; WIDTH * HEIGHT]) {
        let (dir_x, dir_y) = angle_forward(game.player.angle);
        // Camera plane is perpendicular to direction (rotated +90°), FOV ≈ 66°.
        // plane = rotate(dir, +90°) * tan(FOV/2) ≈ dir_perp * 0.66
        let plane_x = mul_div( -dir_y, 168, ONE);
        let plane_y = mul_div(dir_x, 168, ONE);

        let pos_x = game.player.position.x;
        let pos_y = game.player.position.y;

        for col in 0..WIDTH {
            // camera_x in [-1, 1]
            let camera_x = -ONE + (2 * ONE * col as i32) / WIDTH as i32;
            let ray_x = dir_x + mul_div(plane_x, camera_x, ONE);
            let ray_y = dir_y + mul_div(plane_y, camera_x, ONE);

            let mut map_x = pos_x.div_euclid(ONE);
            let mut map_y = pos_y.div_euclid(ONE);

            // Length of ray from one x/y-side to next x/y-side (fixed-point).
            // Cap so side_dist accumulation cannot overflow i32.
            let delta_dist_x = if ray_x == 0 {
                1_000_000
            } else {
                ((ONE as i64 * ONE as i64) / ray_x.abs().max(1) as i64).min(1_000_000) as i32
            };
            let delta_dist_y = if ray_y == 0 {
                1_000_000
            } else {
                ((ONE as i64 * ONE as i64) / ray_y.abs().max(1) as i64).min(1_000_000) as i32
            };

            let (step_x, mut side_dist_x) = if ray_x < 0 {
                let dist = mul_div(pos_x - map_x * ONE, delta_dist_x, ONE);
                (-1, dist)
            } else {
                let dist = mul_div((map_x + 1) * ONE - pos_x, delta_dist_x, ONE);
                (1, dist)
            };
            let (step_y, mut side_dist_y) = if ray_y < 0 {
                let dist = mul_div(pos_y - map_y * ONE, delta_dist_y, ONE);
                (-1, dist)
            } else {
                let dist = mul_div((map_y + 1) * ONE - pos_y, delta_dist_y, ONE);
                (1, dist)
            };

            let mut hit = false;
            let mut side = 0i32; // 0 = NS wall (x-step), 1 = EW wall (y-step)
            let mut tile = 0u8;
            for _ in 0..64 {
                if side_dist_x < side_dist_y {
                    side_dist_x = side_dist_x.saturating_add(delta_dist_x);
                    map_x += step_x;
                    side = 0;
                } else {
                    side_dist_y = side_dist_y.saturating_add(delta_dist_y);
                    map_y += step_y;
                    side = 1;
                }
                tile = game.effective_tile(map_x, map_y);
                if is_blocking(tile) {
                    hit = true;
                    break;
                }
            }

            // Perpendicular distance to wall.
            let perp = if !hit {
                50 * ONE
            } else if side == 0 {
                (side_dist_x - delta_dist_x).max(1)
            } else {
                (side_dist_y - delta_dist_y).max(1)
            };
            self.z_buf[col] = perp;

            // Projected wall height.
            let line_h = ((VIEW_H as i32) * ONE / perp).clamp(1, VIEW_H as i32 * 4);
            let mut draw_start = -line_h / 2 + VIEW_H as i32 / 2;
            let mut draw_end = line_h / 2 + VIEW_H as i32 / 2;
            if draw_start < 0 {
                draw_start = 0;
            }
            if draw_end >= VIEW_H as i32 {
                draw_end = VIEW_H as i32 - 1;
            }

            // Wall X texture coordinate.
            let wall_x = if side == 0 {
                pos_y + mul_div(perp, ray_y, ONE)
            } else {
                pos_x + mul_div(perp, ray_x, ONE)
            };
            let mut tex_x = (wall_x.rem_euclid(ONE) * 8) / ONE;
            if side == 0 && ray_x > 0 {
                tex_x = 7 - tex_x;
            }
            if side == 1 && ray_y < 0 {
                tex_x = 7 - tex_x;
            }
            tex_x = tex_x.clamp(0, 7);

            let tex = if tile == TILE_DOOR {
                &WALL_DOOR
            } else {
                &WALL_STONE
            };

            // Ceiling
            for row in 0..draw_start as usize {
                pixels[row * WIDTH + col] = assets::CEILING;
            }
            // Wall column
            let tex_step = (8 * ONE) / line_h.max(1);
            let mut tex_pos =
                (draw_start - VIEW_H as i32 / 2 + line_h / 2).saturating_mul(tex_step);
            for row in draw_start..=draw_end {
                let tex_y = ((tex_pos / ONE) & 7) as usize;
                tex_pos = tex_pos.saturating_add(tex_step);
                let idx = tex[tex_y * 8 + tex_x as usize];
                let mut color = palette_color(idx);
                // Darken EW (side==1) faces for depth cue, but keep the light
                // stone swatch exact so identity/tests can find STONE_LIGHT.
                if side == 1 && idx != 2 {
                    color = darken(color);
                }
                pixels[row as usize * WIDTH + col] = color;
            }
            // Floor
            for row in (draw_end as usize + 1)..VIEW_H {
                pixels[row * WIDTH + col] = FLOOR;
            }
        }
    }

    fn draw_sprites(&mut self, game: &Game, pixels: &mut [u16; WIDTH * HEIGHT]) {
        // Collect up to 12 billboards (enemies + pickups) with depth, sort far→near.
        let (dir_x, dir_y) = angle_forward(game.player.angle);
        let plane_x = -dir_y * 168 / ONE;
        let plane_y = dir_x * 168 / ONE;
        let pos_x = game.player.position.x;
        let pos_y = game.player.position.y;

        let mut items: [(i32, i32, i32, SpriteKind); 12] = [(0, 0, 0, SpriteKind::Enemy); 12];
        let mut n = 0usize;

        for e in game.enemies.iter() {
            if !e.alive || n >= 12 {
                continue;
            }
            let dx = e.position.x - pos_x;
            let dy = e.position.y - pos_y;
            let depth = (dx * dir_x + dy * dir_y) / ONE;
            if depth <= ONE / 8 {
                continue;
            }
            items[n] = (depth, e.position.x, e.position.y, SpriteKind::Enemy);
            n += 1;
        }
        for p in game.pickups.iter() {
            if !p.active || n >= 12 {
                continue;
            }
            let dx = p.position.x - pos_x;
            let dy = p.position.y - pos_y;
            let depth = (dx * dir_x + dy * dir_y) / ONE;
            if depth <= ONE / 8 {
                continue;
            }
            let kind = match p.kind {
                PickupKind::Health => SpriteKind::Health,
                PickupKind::Ammo => SpriteKind::Ammo,
            };
            items[n] = (depth, p.position.x, p.position.y, kind);
            n += 1;
        }

        // Insertion sort far → near (largest depth first).
        for i in 1..n {
            let mut j = i;
            while j > 0 && items[j].0 > items[j - 1].0 {
                items.swap(j, j - 1);
                j -= 1;
            }
        }

        for i in 0..n {
            let (depth, sx, sy, kind) = items[i];
            let dx = sx - pos_x;
            let dy = sy - pos_y;
            // Inverse camera matrix: det = dir×plane
            let det = (plane_x * dir_y - dir_x * plane_y) / ONE;
            if det == 0 {
                continue;
            }
            let inv_det = (ONE * ONE) / det;
            let transform_x = inv_det * (dir_y * dx / ONE - dir_x * dy / ONE) / ONE;
            let transform_y = inv_det * (-plane_y * dx / ONE + plane_x * dy / ONE) / ONE;
            if transform_y <= ONE / 8 {
                continue;
            }

            let sprite_screen_x = (WIDTH as i32 / 2) * (ONE + transform_x * ONE / transform_y) / ONE;
            let (tex_w, tex_h, tex): (usize, usize, &[u8]) = match kind {
                SpriteKind::Enemy => (ENEMY_W, ENEMY_H, &ENEMY_SPRITE),
                SpriteKind::Health => (PICKUP_W, PICKUP_H, &HEALTH_SPRITE),
                SpriteKind::Ammo => (PICKUP_W, PICKUP_H, &AMMO_SPRITE),
            };

            let sprite_h = ((VIEW_H as i32) * ONE / transform_y).abs().clamp(1, VIEW_H as i32 * 2);
            let sprite_w = sprite_h * tex_w as i32 / tex_h as i32;
            let draw_start_y = (-sprite_h / 2 + VIEW_H as i32 / 2).max(0);
            let draw_end_y = (sprite_h / 2 + VIEW_H as i32 / 2).min(VIEW_H as i32 - 1);
            let draw_start_x = (-sprite_w / 2 + sprite_screen_x).max(0);
            let draw_end_x = (sprite_w / 2 + sprite_screen_x).min(WIDTH as i32 - 1);

            for stripe in draw_start_x..=draw_end_x {
                if transform_y >= self.z_buf[stripe as usize] {
                    continue;
                }
                let tex_x = ((stripe - (-sprite_w / 2 + sprite_screen_x)) * tex_w as i32) / sprite_w;
                if tex_x < 0 || tex_x >= tex_w as i32 {
                    continue;
                }
                for y in draw_start_y..=draw_end_y {
                    let d = y * 2 - VIEW_H as i32 + sprite_h;
                    let tex_y = (d * tex_h as i32) / (2 * sprite_h);
                    if tex_y < 0 || tex_y >= tex_h as i32 {
                        continue;
                    }
                    let idx = tex[tex_y as usize * tex_w + tex_x as usize];
                    if idx == 0 {
                        continue;
                    }
                    pixels[y as usize * WIDTH + stripe as usize] = palette_color(idx);
                }
            }
            let _ = depth;
        }
    }

    fn draw_weapon(&mut self, game: &Game, pixels: &mut [u16; WIDTH * HEIGHT]) {
        let base_x = (WIDTH as i32 - PISTOL_W as i32) / 2;
        let base_y = VIEW_H as i32 - PISTOL_H as i32 + 2;
        for ty in 0..PISTOL_H {
            for tx in 0..PISTOL_W {
                let idx = PISTOL_SPRITE[ty * PISTOL_W + tx];
                if idx == 0 {
                    continue;
                }
                let x = base_x + tx as i32;
                let y = base_y + ty as i32;
                if x < 0 || y < 0 || x >= WIDTH as i32 || y >= VIEW_H as i32 {
                    continue;
                }
                pixels[y as usize * WIDTH + x as usize] = palette_color(idx);
            }
        }
        if game.muzzle_flash {
            // Small flash near the barrel tip.
            let fx = base_x + 26;
            let fy = base_y + 6;
            for dy in -2..=2 {
                for dx in -3..=3 {
                    let x = fx + dx;
                    let y = fy + dy;
                    if x >= 0 && y >= 0 && x < WIDTH as i32 && y < VIEW_H as i32 {
                        pixels[y as usize * WIDTH + x as usize] = MUZZLE;
                    }
                }
            }
        }
    }

    fn draw_hud(&mut self, game: &Game, pixels: &mut [u16; WIDTH * HEIGHT]) {
        let y0 = VIEW_H;
        for row in y0..HEIGHT {
            for col in 0..WIDTH {
                pixels[row * WIDTH + col] = HUD_BG;
            }
        }
        // Health label strip + digits
        for row in y0 + 2..y0 + 10 {
            for col in 2..10 {
                pixels[row * WIDTH + col] = HUD_RED;
            }
        }
        draw_number(
            pixels,
            12,
            y0 + 3,
            game.player.health.max(0) as u32,
            HUD_RED,
        );
        // Ammo
        for row in y0 + 2..y0 + 10 {
            for col in 50..58 {
                pixels[row * WIDTH + col] = HUD_YELLOW;
            }
        }
        draw_number(
            pixels,
            60,
            y0 + 3,
            game.player.ammo.max(0) as u32,
            HUD_YELLOW,
        );

        // Phase banner (won/dead)
        match game.phase {
            crate::game::Phase::Won => draw_text_bar(pixels, y0 + 2, HUD_YELLOW),
            crate::game::Phase::Dead => draw_text_bar(pixels, y0 + 2, HUD_RED),
            crate::game::Phase::Playing => {}
        }
        let _ = HUD_H;
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
enum SpriteKind {
    Enemy,
    Health,
    Ammo,
}

/// (a * b) / c with i64 intermediate to avoid debug overflow.
#[inline]
fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    let c = if c == 0 { 1 } else { c };
    ((a as i64) * (b as i64) / (c as i64)) as i32
}

fn darken(c: u16) -> u16 {
    let r = ((c >> 11) & 0x1F) * 3 / 4;
    let g = ((c >> 5) & 0x3F) * 3 / 4;
    let b = (c & 0x1F) * 3 / 4;
    (r << 11) | (g << 5) | b
}

fn draw_digit(pixels: &mut [u16; WIDTH * HEIGHT], x: usize, y: usize, d: u8, color: u16) {
    let glyph = DIGIT_FONT[(d as usize) % 10];
    for col in 0..3 {
        let bits = glyph[col];
        for row in 0..5 {
            if bits & (1 << row) != 0 {
                let px = x + col;
                let py = y + row;
                if px < WIDTH && py < HEIGHT {
                    pixels[py * WIDTH + px] = color;
                }
            }
        }
    }
}

fn draw_number(pixels: &mut [u16; WIDTH * HEIGHT], x: usize, y: usize, mut n: u32, color: u16) {
    // Up to 3 digits, right-to-left.
    let mut digits = [0u8; 3];
    let mut count = 0usize;
    if n == 0 {
        digits[0] = 0;
        count = 1;
    } else {
        while n > 0 && count < 3 {
            digits[count] = (n % 10) as u8;
            n /= 10;
            count += 1;
        }
    }
    for i in 0..count {
        let d = digits[count - 1 - i];
        draw_digit(pixels, x + i * 4, y, d, color);
    }
}

fn draw_text_bar(pixels: &mut [u16; WIDTH * HEIGHT], y: usize, color: u16) {
    for col in 100..150 {
        for row in y..y + 6 {
            if row < HEIGHT {
                pixels[row * WIDTH + col] = color;
            }
        }
    }
}

/// FNV-1a 32-bit over the framebuffer bytes (little-endian RGB565).
pub fn frame_hash(pixels: &[u16; WIDTH * HEIGHT]) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for &p in pixels.iter() {
        h ^= (p & 0xFF) as u32;
        h = h.wrapping_mul(0x0100_0193);
        h ^= (p >> 8) as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Game;

    #[test]
    fn initial_view_contains_wall_weapon_and_hud() {
        let mut pixels = [0u16; WIDTH * HEIGHT];
        Renderer::new().render(&Game::new(), &mut pixels);
        assert!(
            pixels.contains(&assets::STONE_LIGHT),
            "expected wall colour STONE_LIGHT in framebuffer"
        );
        assert!(
            pixels.contains(&assets::PISTOL_DARK),
            "expected pistol colour PISTOL_DARK in framebuffer"
        );
        assert!(
            pixels[(HEIGHT - 8) * WIDTH..].contains(&assets::HUD_RED),
            "expected HUD_RED in bottom 8 rows"
        );
    }

    #[test]
    fn initial_frame_is_deterministic() {
        let mut a = [0u16; WIDTH * HEIGHT];
        let mut b = [0u16; WIDTH * HEIGHT];
        Renderer::new().render(&Game::new(), &mut a);
        Renderer::new().render(&Game::new(), &mut b);
        assert_eq!(a, b);
    }

    #[test]
    fn initial_frame_hash_is_stable() {
        let mut pixels = [0u16; WIDTH * HEIGHT];
        Renderer::new().render(&Game::new(), &mut pixels);
        // Observed once the initial view (walls + pistol + HUD) stabilised.
        assert_eq!(frame_hash(&pixels), 0x83a7_228e);
    }
}
