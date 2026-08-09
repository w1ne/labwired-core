//! Deterministic fixed-point game rules and state (no heap, no float).

use crate::level::{
    is_blocking, tile_at, ENEMY_SPAWNS, EXIT_TX, EXIT_TY, ONE, PICKUP_SPAWNS, SPAWN_ANGLE,
    SPAWN_X, SPAWN_Y, TILE_DOOR, TILE_EMPTY, TILE_EXIT,
};

/// Full circle in angle units (matches level::SPAWN_ANGLE convention).
pub const ANGLE_FULL: i32 = 256;
/// Movement speed in fixed-point units per tick.
pub const MOVE_SPEED: i32 = 24;
/// Turn speed in angle units per tick.
pub const TURN_SPEED: i32 = 6;
/// Hitscan damage per successful fire.
pub const FIRE_DAMAGE: i32 = 20;
/// Hitscan max range in fixed-point units (~8 tiles).
pub const FIRE_RANGE: i32 = 8 * ONE;
/// Enemy contact damage per tick while overlapping the player.
pub const ENEMY_TOUCH_DAMAGE: i32 = 2;
/// Player collision radius (half-width) in fixed-point units.
pub const PLAYER_RADIUS: i32 = 40;
/// Enemy hit radius for hitscan and touch.
pub const ENEMY_RADIUS: i32 = 48;
/// Pickup collection radius.
pub const PICKUP_RADIUS: i32 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vec2 {
    pub x: i32,
    pub y: i32,
}

impl Vec2 {
    pub const ZERO: Self = Self { x: 0, y: 0 };

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Held movement and edge-triggered fire/use for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Actions {
    pub forward: bool,
    pub backward: bool,
    pub turn_left: bool,
    pub turn_right: bool,
    pub fire_pressed: bool,
    pub use_pressed: bool,
}

impl Actions {
    pub const NONE: Self = Self {
        forward: false,
        backward: false,
        turn_left: false,
        turn_right: false,
        fire_pressed: false,
        use_pressed: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Playing,
    Won,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Player {
    pub position: Vec2,
    /// Angle units; 0 = north (−Y).
    pub angle: i32,
    pub health: i32,
    pub ammo: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Enemy {
    pub position: Vec2,
    pub hp: i32,
    pub alive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickupKind {
    Health,
    Ammo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pickup {
    pub position: Vec2,
    pub kind: PickupKind,
    pub active: bool,
}

/// Door state keyed by map cell (only one interactive door in this level).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Door {
    pub tx: i32,
    pub ty: i32,
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    pub phase: Phase,
    pub player: Player,
    pub enemies: [Enemy; 6],
    pub pickups: [Pickup; 6],
    pub door: Door,
    /// True after a successful fire this tick (for muzzle flash).
    pub muzzle_flash: bool,
}

impl Game {
    /// Default level: player faces a wall so the first forward step is blocked.
    pub fn new() -> Self {
        let mut g = Self::blank_playing(
            Vec2::new(SPAWN_X, SPAWN_Y),
            SPAWN_ANGLE,
            100,
            20,
        );
        g.spawn_default_actors();
        g
    }

    /// Straight corridor with one enemy directly north of the player.
    /// Column 9 is open from y=7..9 so the hitscan has clear LOS.
    pub fn test_firing_lane() -> Self {
        let mut g = Self::blank_playing(
            Vec2::new(9 * ONE + ONE / 2, 9 * ONE + ONE / 2),
            0, // face north
            100,
            20,
        );
        g.enemies[0] = Enemy {
            position: Vec2::new(9 * ONE + ONE / 2, 7 * ONE + ONE / 2),
            hp: 40,
            alive: true,
        };
        g
    }

    /// Player standing on the exit tile, ready to use it.
    pub fn test_at_exit() -> Self {
        Self::blank_playing(
            Vec2::new(EXIT_TX * ONE + ONE / 2, EXIT_TY * ONE + ONE / 2),
            0,
            100,
            10,
        )
    }

    fn blank_playing(position: Vec2, angle: i32, health: i32, ammo: i32) -> Self {
        Self {
            phase: Phase::Playing,
            player: Player {
                position,
                angle: angle.rem_euclid(ANGLE_FULL),
                health,
                ammo,
            },
            enemies: [Enemy {
                position: Vec2::ZERO,
                hp: 0,
                alive: false,
            }; 6],
            pickups: [Pickup {
                position: Vec2::ZERO,
                kind: PickupKind::Health,
                active: false,
            }; 6],
            door: Door {
                tx: 7,
                ty: 4,
                open: false,
            },
            muzzle_flash: false,
        }
    }

    fn spawn_default_actors(&mut self) {
        for (i, &(x, y)) in ENEMY_SPAWNS.iter().enumerate() {
            if x == 0 && y == 0 {
                continue;
            }
            self.enemies[i] = Enemy {
                position: Vec2::new(x, y),
                hp: 40,
                alive: true,
            };
        }
        for (i, &(x, y)) in PICKUP_SPAWNS.iter().enumerate() {
            if x == 0 && y == 0 {
                continue;
            }
            self.pickups[i] = Pickup {
                position: Vec2::new(x, y),
                kind: if i % 2 == 0 {
                    PickupKind::Health
                } else {
                    PickupKind::Ammo
                },
                active: true,
            };
        }
    }

    /// Advance one deterministic fixed step.
    pub fn tick(&mut self, actions: Actions) {
        self.muzzle_flash = false;

        if self.phase != Phase::Playing {
            if actions.use_pressed {
                *self = Self::new();
            }
            return;
        }

        if actions.turn_left {
            self.player.angle =
                (self.player.angle - TURN_SPEED).rem_euclid(ANGLE_FULL);
        }
        if actions.turn_right {
            self.player.angle =
                (self.player.angle + TURN_SPEED).rem_euclid(ANGLE_FULL);
        }

        let (fx, fy) = angle_forward(self.player.angle);
        let mut dx = 0i32;
        let mut dy = 0i32;
        if actions.forward {
            dx += (fx * MOVE_SPEED) / ONE;
            dy += (fy * MOVE_SPEED) / ONE;
        }
        if actions.backward {
            dx -= (fx * MOVE_SPEED) / ONE;
            dy -= (fy * MOVE_SPEED) / ONE;
        }
        self.try_move(dx, dy);

        if actions.use_pressed {
            self.try_use();
        }
        if actions.fire_pressed {
            self.try_fire();
        }

        self.tick_enemies();
        self.collect_pickups();

        if self.player.health <= 0 {
            self.player.health = 0;
            self.phase = Phase::Dead;
        }
    }

    /// Axis-separated sliding collision against blocking tiles.
    fn try_move(&mut self, dx: i32, dy: i32) {
        if dx != 0 {
            let nx = self.player.position.x + dx;
            if !self.blocked_at(nx, self.player.position.y) {
                self.player.position.x = nx;
            }
        }
        if dy != 0 {
            let ny = self.player.position.y + dy;
            if !self.blocked_at(self.player.position.x, ny) {
                self.player.position.y = ny;
            }
        }
    }

    fn blocked_at(&self, x: i32, y: i32) -> bool {
        // Sample four corners of the player AABB.
        let r = PLAYER_RADIUS;
        let samples = [
            (x - r, y - r),
            (x + r, y - r),
            (x - r, y + r),
            (x + r, y + r),
        ];
        for (sx, sy) in samples {
            let tx = sx.div_euclid(ONE);
            let ty = sy.div_euclid(ONE);
            let t = self.effective_tile(tx, ty);
            if is_blocking(t) {
                return true;
            }
        }
        false
    }

    /// Map tile with door open state applied.
    pub fn effective_tile(&self, tx: i32, ty: i32) -> u8 {
        let t = tile_at(tx, ty);
        if t == TILE_DOOR && self.door.open && tx == self.door.tx && ty == self.door.ty {
            return TILE_EMPTY;
        }
        t
    }

    fn try_use(&mut self) {
        // Win if standing on exit.
        let ptx = self.player.position.x.div_euclid(ONE);
        let pty = self.player.position.y.div_euclid(ONE);
        if tile_at(ptx, pty) == TILE_EXIT {
            self.phase = Phase::Won;
            return;
        }

        // Open adjacent door if facing / near it.
        let (fx, fy) = angle_forward(self.player.angle);
        let reach_x = self.player.position.x + fx;
        let reach_y = self.player.position.y + fy;
        let rtx = reach_x.div_euclid(ONE);
        let rty = reach_y.div_euclid(ONE);
        if tile_at(rtx, rty) == TILE_DOOR
            && rtx == self.door.tx
            && rty == self.door.ty
            && !self.door.open
        {
            self.door.open = true;
        }
    }

    fn try_fire(&mut self) {
        if self.player.ammo <= 0 {
            return;
        }
        self.player.ammo -= 1;
        self.muzzle_flash = true;

        let (fx, fy) = angle_forward(self.player.angle);
        // Step along the look ray in fixed increments and pick the nearest live enemy
        // within FIRE_RANGE whose projection falls inside the hit cylinder.
        let mut best_i: Option<usize> = None;
        let mut best_dist = FIRE_RANGE + 1;

        for (i, e) in self.enemies.iter().enumerate() {
            if !e.alive || e.hp <= 0 {
                continue;
            }
            let vx = e.position.x - self.player.position.x;
            let vy = e.position.y - self.player.position.y;
            // Project onto forward axis: dist = dot(v, forward) / ONE
            let along = (vx * fx + vy * fy) / ONE;
            if along <= 0 || along > FIRE_RANGE {
                continue;
            }
            // Perpendicular distance: |v × forward| / ONE (2D cross magnitude).
            let cross = (vx * fy - vy * fx).abs() / ONE;
            if cross > ENEMY_RADIUS {
                continue;
            }
            // LOS: sample tiles along the segment.
            if !self.clear_los(self.player.position, e.position) {
                continue;
            }
            if along < best_dist {
                best_dist = along;
                best_i = Some(i);
            }
        }

        if let Some(i) = best_i {
            let e = &mut self.enemies[i];
            e.hp -= FIRE_DAMAGE;
            if e.hp <= 0 {
                e.hp = 0;
                e.alive = false;
            }
        }
    }

    fn clear_los(&self, from: Vec2, to: Vec2) -> bool {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        let steps = ((dx.abs().max(dy.abs())) / (ONE / 4)).max(1);
        for s in 1..steps {
            let x = from.x + dx * s / steps;
            let y = from.y + dy * s / steps;
            let tx = x.div_euclid(ONE);
            let ty = y.div_euclid(ONE);
            if is_blocking(self.effective_tile(tx, ty)) {
                return false;
            }
        }
        true
    }

    fn tick_enemies(&mut self) {
        let px = self.player.position.x;
        let py = self.player.position.y;
        for e in self.enemies.iter_mut() {
            if !e.alive {
                continue;
            }
            let dx = px - e.position.x;
            let dy = py - e.position.y;
            // Cheap touch damage when close (manhattan-ish via max abs).
            let adx = dx.abs();
            let ady = dy.abs();
            if adx < ENEMY_RADIUS && ady < ENEMY_RADIUS {
                self.player.health -= ENEMY_TOUCH_DAMAGE;
            } else if adx < 4 * ONE && ady < 4 * ONE {
                // Creep one unit toward the player when in range (deterministic).
                let step = 4;
                let sx = if dx > step {
                    step
                } else if dx < -step {
                    -step
                } else {
                    0
                };
                let sy = if dy > step {
                    step
                } else if dy < -step {
                    -step
                } else {
                    0
                };
                let nx = e.position.x + sx;
                let ny = e.position.y + sy;
                // Enemies also cannot walk through walls.
                let tx = nx.div_euclid(ONE);
                let ty = ny.div_euclid(ONE);
                // Door open state is on self; use tile_at + door check inline.
                let t = tile_at(tx, ty);
                let blocked = if t == TILE_DOOR {
                    // Can't borrow self.door while iterating enemies via effective_tile easily —
                    // door is Copy so read it before the loop... already open flag is Copy.
                    true // doors always block enemies for simplicity
                } else {
                    is_blocking(t)
                };
                if !blocked {
                    e.position.x = nx;
                    e.position.y = ny;
                }
            }
        }
    }

    fn collect_pickups(&mut self) {
        let px = self.player.position.x;
        let py = self.player.position.y;
        for p in self.pickups.iter_mut() {
            if !p.active {
                continue;
            }
            let dx = (p.position.x - px).abs();
            let dy = (p.position.y - py).abs();
            if dx < PICKUP_RADIUS && dy < PICKUP_RADIUS {
                match p.kind {
                    PickupKind::Health => {
                        self.player.health = (self.player.health + 25).min(100);
                    }
                    PickupKind::Ammo => {
                        self.player.ammo = (self.player.ammo + 10).min(99);
                    }
                }
                p.active = false;
            }
        }
    }
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

/// Forward unit vector for `angle` in fixed-point (length ≈ ONE).
/// 0 = north (−Y), 64 = east (+X).
pub fn angle_forward(angle: i32) -> (i32, i32) {
    let a = angle.rem_euclid(ANGLE_FULL);
    // Quarter-wave table via symmetry; 64 steps per quadrant.
    let q = a / 64;
    let r = a % 64;
    // cos/sin approximation: cos(0)=1, cos(64°steps)=0 over a quadrant with linear blend.
    // Use a compact quarter sine table at 16-entry resolution for determinism.
    let (s, c) = quarter_sin_cos(r);
    match q {
        0 => (s, -c),  // north toward east
        1 => (c, s),   // east toward south
        2 => (-s, c),  // south toward west
        _ => (-c, -s), // west toward north
    }
}

/// `r` in 0..64 over a quadrant. Returns (sin, cos) in fixed-point ONE units.
fn quarter_sin_cos(r: i32) -> (i32, i32) {
    // Linear blend is enough for movement feel and keeps the binary tiny.
    // sin goes 0→ONE, cos goes ONE→0 as r goes 0→64.
    let s = (r * ONE) / 64;
    let c = ONE - s;
    (s, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_cannot_walk_through_wall() {
        let mut game = Game::new();
        let before = game.player.position;
        game.tick(Actions {
            forward: true,
            ..Actions::NONE
        });
        assert_eq!(game.player.position, before);
    }

    #[test]
    fn fire_damages_nearest_visible_enemy() {
        let mut game = Game::test_firing_lane();
        let hp = game.enemies[0].hp;
        game.tick(Actions {
            fire_pressed: true,
            ..Actions::NONE
        });
        assert!(game.enemies[0].hp < hp);
    }

    #[test]
    fn using_exit_wins() {
        let mut game = Game::test_at_exit();
        game.tick(Actions {
            use_pressed: true,
            ..Actions::NONE
        });
        assert_eq!(game.phase, Phase::Won);
    }
}
