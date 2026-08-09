//! GPIO samples → action structure.
//!
//! Active-low pull-up buttons on:
//!   GPIO1 forward, GPIO2 backward, GPIO3 left, GPIO4 right,
//!   GPIO5 fire, GPIO6 use.
//!
//! Movement/turn are level-held; fire/use are rising edges (press events).

use crate::game::Actions;

/// Bitflags for raw button samples (1 = pressed/active).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawButtons(pub u8);

impl RawButtons {
    pub const NONE: Self = Self(0);
    pub const FORWARD: Self = Self(1 << 0);
    pub const BACKWARD: Self = Self(1 << 1);
    pub const LEFT: Self = Self(1 << 2);
    pub const RIGHT: Self = Self(1 << 3);
    pub const FIRE: Self = Self(1 << 4);
    pub const USE: Self = Self(1 << 5);

    #[inline]
    pub fn contains(self, bit: Self) -> bool {
        self.0 & bit.0 != 0
    }

    /// Build from active-low pin levels (`true` = pin high / released).
    /// Pressed when the pin is low.
    pub fn from_active_low(
        forward_high: bool,
        backward_high: bool,
        left_high: bool,
        right_high: bool,
        fire_high: bool,
        use_high: bool,
    ) -> Self {
        let mut b = 0u8;
        if !forward_high {
            b |= Self::FORWARD.0;
        }
        if !backward_high {
            b |= Self::BACKWARD.0;
        }
        if !left_high {
            b |= Self::LEFT.0;
        }
        if !right_high {
            b |= Self::RIGHT.0;
        }
        if !fire_high {
            b |= Self::FIRE.0;
        }
        if !use_high {
            b |= Self::USE.0;
        }
        Self(b)
    }
}

/// Tracks previous edge state for fire/use.
pub struct InputState {
    prev_fire: bool,
    prev_use: bool,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            prev_fire: false,
            prev_use: false,
        }
    }

    /// Convert a raw sample into Actions. Fire/use fire only on rising edge.
    pub fn update(&mut self, raw: RawButtons) -> Actions {
        let fire_down = raw.contains(RawButtons::FIRE);
        let use_down = raw.contains(RawButtons::USE);
        let fire_pressed = fire_down && !self.prev_fire;
        let use_pressed = use_down && !self.prev_use;
        self.prev_fire = fire_down;
        self.prev_use = use_down;
        Actions {
            forward: raw.contains(RawButtons::FORWARD),
            backward: raw.contains(RawButtons::BACKWARD),
            turn_left: raw.contains(RawButtons::LEFT),
            turn_right: raw.contains(RawButtons::RIGHT),
            fire_pressed,
            use_pressed,
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

/// GPIO pin numbers matching `system.yaml` / hardware wiring.
pub mod pins {
    pub const FORWARD: u8 = 1;
    pub const BACKWARD: u8 = 2;
    pub const LEFT: u8 = 3;
    pub const RIGHT: u8 = 4;
    pub const FIRE: u8 = 5;
    pub const USE: u8 = 6;
    pub const TFT_CS: u8 = 10;
    pub const TFT_DC: u8 = 11;
    pub const TFT_SCLK: u8 = 12;
    pub const TFT_MOSI: u8 = 13;
    pub const TFT_RESET: u8 = 14;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_fire_produces_one_press() {
        let mut input = InputState::new();
        assert!(input.update(RawButtons::FIRE).fire_pressed);
        assert!(!input.update(RawButtons::FIRE).fire_pressed);
        assert!(!input.update(RawButtons::NONE).fire_pressed);
        assert!(input.update(RawButtons::FIRE).fire_pressed);
    }

    #[test]
    fn movement_is_level_held() {
        let mut input = InputState::new();
        let a = input.update(RawButtons::FORWARD);
        assert!(a.forward);
        assert!(!a.fire_pressed);
        let b = input.update(RawButtons::FORWARD);
        assert!(b.forward);
    }

    #[test]
    fn active_low_mapping() {
        // All pins high → released.
        let raw = RawButtons::from_active_low(true, true, true, true, true, true);
        assert_eq!(raw, RawButtons::NONE);
        // Fire pin low → pressed.
        let raw = RawButtons::from_active_low(true, true, true, true, false, true);
        assert!(raw.contains(RawButtons::FIRE));
    }
}
