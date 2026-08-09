//! Immutable 16×16 tile map and spawn metadata (flash-resident on device).

/// Fixed-point scale: 1.0 tile = 256 units.
pub const ONE: i32 = 256;

/// Map side length in tiles.
pub const MAP_W: usize = 16;
pub const MAP_H: usize = 16;

/// Tile codes.
pub const TILE_EMPTY: u8 = 0;
pub const TILE_WALL: u8 = 1;
pub const TILE_DOOR: u8 = 2;
pub const TILE_EXIT: u8 = 3;

/// Row-major 16×16 map. Walls form a closed arena with an interior corridor
/// layout so the player can walk, fight, and reach a marked exit.
///
/// Legend: `#` wall, ` ` empty, `D` door, `E` exit.
///
/// The default spawn faces north into a wall immediately in front (for the
/// collision unit test); other constructors reposition the player.
pub const MAP: [[u8; MAP_W]; MAP_H] = [
    // y = 0 (north)
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1],
    [1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1],
    [1, 0, 1, 0, 1, 1, 1, 2, 1, 1, 1, 0, 0, 1, 0, 1],
    [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
    [1, 0, 1, 0, 1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 0, 1],
    [1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1],
    [1, 0, 1, 1, 1, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0, 1],
    [1, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 1, 1, 1, 0, 1, 0, 1, 1, 1, 0, 1, 1, 0, 1],
    [1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1],
    [1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 0, 1, 0, 1, 1],
    [1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    [1, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 0, 0, 1],
    [1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1],
];

/// Tile at integer map coordinates. Out-of-bounds reads as wall.
#[inline]
pub fn tile_at(tx: i32, ty: i32) -> u8 {
    if tx < 0 || ty < 0 || tx >= MAP_W as i32 || ty >= MAP_H as i32 {
        return TILE_WALL;
    }
    MAP[ty as usize][tx as usize]
}

/// True when the tile blocks movement (solid wall or closed door).
#[inline]
pub fn is_blocking(tile: u8) -> bool {
    matches!(tile, TILE_WALL | TILE_DOOR)
}

/// Default player spawn: tile (8, 13), facing north (angle 0).
/// Y is set so the player's north collision edge sits on the boundary of the
/// solid wall at tile (8, 12); any forward step is rejected immediately.
pub const SPAWN_X: i32 = 8 * ONE + ONE / 2;
/// PLAYER_RADIUS is 40 — keep this in lockstep with `game::PLAYER_RADIUS`.
pub const SPAWN_Y: i32 = 13 * ONE + 40;
/// Angle units: full circle = 256. 0 = north (−Y), 64 = east (+X), 128 = south, 192 = west.
pub const SPAWN_ANGLE: i32 = 0;

/// Exit tile coordinates.
pub const EXIT_TX: i32 = 7;
pub const EXIT_TY: i32 = 14;

/// Enemy spawn (tile centers), up to 6 slots. Unused slots have hp = 0.
pub const ENEMY_SPAWNS: [(i32, i32); 6] = [
    (5 * ONE + ONE / 2, 5 * ONE + ONE / 2),
    (10 * ONE + ONE / 2, 9 * ONE + ONE / 2),
    (3 * ONE + ONE / 2, 11 * ONE + ONE / 2),
    (12 * ONE + ONE / 2, 3 * ONE + ONE / 2),
    (0, 0),
    (0, 0),
];

/// Pickup spawn (tile centers). Kind encoded separately in game state.
pub const PICKUP_SPAWNS: [(i32, i32); 6] = [
    (3 * ONE + ONE / 2, 3 * ONE + ONE / 2),
    (11 * ONE + ONE / 2, 5 * ONE + ONE / 2),
    (7 * ONE + ONE / 2, 9 * ONE + ONE / 2),
    (0, 0),
    (0, 0),
    (0, 0),
];
