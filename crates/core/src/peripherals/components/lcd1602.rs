// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! HD44780 character LCD (16×2) behind a PCF8574 I²C backpack.
//!
//! The module sold as "LCD1602 I²C" is two chips stacked: a PCF8574 8-bit I/O
//! expander whose P-port drives an HD44780 (or clone) LCD controller in 4-bit
//! mode. Firmware never talks to the HD44780 directly — every I²C byte is a
//! snapshot of the eight expander pins, and the LCD sees only the *edges* those
//! snapshots produce. Both layers are modelled here.
//!
//! ## PCF8574 → HD44780 wiring
//!
//! The near-universal "PCF8574T LCD backpack" pinout, which
//! `LiquidCrystal_I2C` and every clone of it assume:
//!
//! | Expander pin | LCD signal |
//! |--------------|------------|
//! | `P7`…`P4`    | `D7`…`D4` — the 4-bit data nibble |
//! | `P3`         | Backlight (1 = lit) |
//! | `P2`         | `E` — enable strobe |
//! | `P1`         | `RW` — 0 = write, 1 = read |
//! | `P0`         | `RS` — 0 = command, 1 = data |
//!
//! ## Latching
//!
//! The HD44780 samples `D7`…`D4`, `RS` and `RW` on the **falling edge of `E`**,
//! so the model tracks the previous `E` level and latches a nibble only on a
//! 1→0 transition. Bytes that merely change the backlight, or that raise `E`,
//! move no data — exactly as on real silicon, where a driver's
//! `expanderWrite`/`pulseEnable` pair produces one latch per pulse.
//!
//! Nibbles carrying `RW`=1 are LCD *reads* (the controller drives the bus, the
//! expander does not) and are not latched as writes.
//!
//! ## Nibble pairing and the 4-bit init sequence
//!
//! In 4-bit mode one 8-bit value takes two latches: high nibble then low. The
//! pairing rule is:
//!
//! * no nibble pending → the latched nibble becomes the **high** half;
//! * a nibble pending with the **same `RS`** → the pair completes, the byte is
//!   `high << 4 | low`, and it is dispatched on the `RS` captured with the
//!   *high* nibble;
//! * a nibble pending with a **different `RS`** → the stale half is dropped and
//!   the new nibble starts a fresh pair. `RS` cannot change mid-byte on a real
//!   backpack, so a mismatch means the pending half was an orphan.
//!
//! Before 4-bit mode exists the HD44780 is still in its 8-bit power-on state and
//! the init sequence (`0x3`, `0x3`, `0x3`, `0x2` — sent as *single* nibbles)
//! is interpreted by the LCD as four 8-bit function-set writes. This model pairs
//! them into `0x33` and `0x32`, which both decode as function set (`0x20`…`0x3F`)
//! and touch nothing but the 1-line/2-line flag; the real function set that
//! follows (`0x28`) sets it correctly. The count is even, so no orphan half is
//! left pending and DDRAM, the address counter and the display flags are all
//! untouched — the sequence cannot corrupt state.

use crate::peripherals::i2c::I2cDevice;
use std::any::Any;

/// Visible columns on the panel.
const COLS: usize = 16;
/// Visible rows on the panel.
const ROWS: usize = 2;

/// HD44780 DDRAM size. The real part has 80 bytes, laid out as two 40-byte
/// lines: addresses `0x00`…`0x27` (line 0) and `0x40`…`0x67` (line 1). A 16×2
/// panel shows the first 16 of each; the rest is off-screen scrollback.
const DDRAM_LEN: usize = 80;
/// Bytes per DDRAM line.
const LINE_LEN: usize = 40;
/// DDRAM address at which line 1 begins.
const LINE1_BASE: usize = 0x40;

/// HD44780 CGRAM size: 8 user characters × 8 rows.
const CGRAM_LEN: usize = 64;

// PCF8574 P-port bit masks (see the wiring table above).
const PIN_BACKLIGHT: u8 = 0x08;
const PIN_ENABLE: u8 = 0x04;
const PIN_RW: u8 = 0x02;
const PIN_RS: u8 = 0x01;

/// Blank character the HD44780 fills DDRAM with on "clear display".
const BLANK: u8 = 0x20;

/// Serialise a fixed-size byte array as a JSON array. `serde` only implements
/// `Serialize` for arrays up to 32 elements, and both RAMs here are larger.
fn serialize_bytes<S: serde::Serializer, const N: usize>(
    bytes: &[u8; N],
    ser: S,
) -> Result<S::Ok, S::Error> {
    serde::Serialize::serialize(bytes.as_slice(), ser)
}

/// LCD1602 character display: HD44780 controller behind a PCF8574 I²C backpack.
#[derive(Debug, serde::Serialize)]
pub struct Lcd1602 {
    address: u8,

    // ── PCF8574 backpack layer ──
    /// Level of `E` in the previously written P-port snapshot, so a 1→0
    /// transition can be detected.
    prev_enable: bool,
    /// High nibble latched so far, with the `RS` sampled alongside it.
    /// `None` = the next latch starts a new byte.
    pending_nibble: Option<(u8, bool)>,
    /// Backlight line (`P3`) from the most recent write.
    backlight: bool,

    // ── HD44780 controller layer ──
    /// Display data RAM, in the real two-line layout: index `0`…`39` is address
    /// `0x00`…`0x27`, index `40`…`79` is address `0x40`…`0x67`.
    #[serde(serialize_with = "serialize_bytes")]
    ddram: [u8; DDRAM_LEN],
    /// Character generator RAM — user-defined glyphs. Modelled as storage only;
    /// the text readback renders the code point, not the bitmap.
    #[serde(serialize_with = "serialize_bytes")]
    cgram: [u8; CGRAM_LEN],
    /// Address counter. Indexes CGRAM while [`Lcd1602::cgram_mode`] is set,
    /// DDRAM otherwise.
    addr: usize,
    /// True after "set CGRAM address"; cleared by "set DDRAM address".
    cgram_mode: bool,
    /// Entry mode `I/D`: true = address counter increments after each access.
    entry_increment: bool,
    /// Display on/off control `D`. False blanks the panel without clearing DDRAM.
    display_on: bool,
    /// Display on/off control `C` — cursor visible.
    cursor_on: bool,
    /// Display on/off control `B` — cursor blinking.
    blink_on: bool,
    /// Function set `N`: true = 2-line mode.
    two_line: bool,
}

impl Default for Lcd1602 {
    fn default() -> Self {
        Self::new(0x27)
    }
}

impl Lcd1602 {
    /// Power-on state per the HD44780 datasheet's initialising-by-internal-reset
    /// section: display cleared (DDRAM = spaces), address counter at 0, display
    /// off, cursor and blink off, 1-line mode, entry mode incrementing.
    pub fn new(address: u8) -> Self {
        Self {
            address,
            prev_enable: false,
            pending_nibble: None,
            backlight: false,
            ddram: [BLANK; DDRAM_LEN],
            cgram: [0; CGRAM_LEN],
            addr: 0,
            cgram_mode: false,
            entry_increment: true,
            display_on: false,
            cursor_on: false,
            blink_on: false,
            two_line: false,
        }
    }

    /// The visible 2×16 panel as 32 characters: row 0 first, then row 1, with no
    /// separator — callers slice `[..16]` and `[16..]`.
    ///
    /// Bytes outside printable ASCII (`0x20`…`0x7E`) render as a space; the
    /// HD44780 ROM maps them to katakana or user glyphs that have no faithful
    /// single-`char` form. A display that is off reads as all spaces, mirroring
    /// the way [`super::max7219::Max7219`] and [`super::tm1637_7seg::Tm1637`]
    /// blank their readback — reporting DDRAM here would render a picture the
    /// real panel is not showing.
    pub fn text(&self) -> String {
        let mut out = String::with_capacity(ROWS * COLS);
        for row in 0..ROWS {
            for col in 0..COLS {
                if !self.display_on {
                    out.push(' ');
                    continue;
                }
                let byte = self.ddram[row * LINE_LEN + col];
                out.push(if (0x20..=0x7E).contains(&byte) {
                    byte as char
                } else {
                    ' '
                });
            }
        }
        out
    }

    /// Raw DDRAM as written by the firmware, ignoring display on/off.
    ///
    /// Index `0`…`39` is HD44780 address `0x00`…`0x27` (line 0), index `40`…`79`
    /// is address `0x40`…`0x67` (line 1).
    pub fn ddram(&self) -> [u8; DDRAM_LEN] {
        self.ddram
    }

    /// Backlight line (`P3`) as last driven by the host.
    pub fn backlight(&self) -> bool {
        self.backlight
    }

    /// Display on/off control `D`.
    pub fn display_on(&self) -> bool {
        self.display_on
    }

    /// Display on/off control `C` — cursor visible.
    pub fn cursor_on(&self) -> bool {
        self.cursor_on
    }

    /// Display on/off control `B` — cursor blinking.
    pub fn blink_on(&self) -> bool {
        self.blink_on
    }

    /// Function set `N`: true = 2-line mode.
    pub fn two_line(&self) -> bool {
        self.two_line
    }

    /// Current address counter, as the HD44780 reports it (a DDRAM address when
    /// [`Lcd1602::cgram_mode`] is false, a CGRAM address when it is true).
    pub fn address_counter(&self) -> usize {
        self.addr
    }

    /// Translate an HD44780 DDRAM address into an index of the packed 80-byte
    /// array. Addresses in the gaps between the two lines (`0x28`…`0x3F` and
    /// `0x68`…`0x7F`) are not backed by memory on real silicon, so writes there
    /// are dropped.
    fn ddram_index(addr: usize) -> Option<usize> {
        match addr {
            0x00..=0x27 => Some(addr),
            LINE1_BASE..=0x67 => Some(LINE_LEN + addr - LINE1_BASE),
            _ => None,
        }
    }

    /// Latch one nibble on the falling edge of `E`, pairing it with any pending
    /// half per the rule documented at module level.
    fn latch_nibble(&mut self, nibble: u8, rs: bool) {
        match self.pending_nibble.take() {
            Some((high, high_rs)) if high_rs == rs => {
                let byte = (high << 4) | (nibble & 0x0F);
                if high_rs {
                    self.handle_data(byte);
                } else {
                    self.handle_command(byte);
                }
            }
            // Either nothing pending, or the pending half carried a different
            // RS and so was an orphan: this nibble starts a fresh byte.
            _ => self.pending_nibble = Some((nibble & 0x0F, rs)),
        }
    }

    /// Execute an HD44780 instruction (`RS`=0). Decoded by highest set bit, the
    /// way the real instruction table is organised.
    fn handle_command(&mut self, cmd: u8) {
        match cmd {
            0x00 => { /* no instruction */ }
            // Clear display: DDRAM ← spaces, address counter ← 0, I/D unchanged.
            0x01 => {
                self.ddram = [BLANK; DDRAM_LEN];
                self.addr = 0;
                self.cgram_mode = false;
            }
            // Return home: address counter ← 0, DDRAM untouched.
            0x02..=0x03 => {
                self.addr = 0;
                self.cgram_mode = false;
            }
            // Entry mode set: bit1 = I/D (increment), bit0 = S (display shift,
            // not modelled — the panel view is the DDRAM window either way).
            0x04..=0x07 => self.entry_increment = cmd & 0x02 != 0,
            // Display on/off control: bit2 = D, bit1 = C, bit0 = B.
            0x08..=0x0F => {
                self.display_on = cmd & 0x04 != 0;
                self.cursor_on = cmd & 0x02 != 0;
                self.blink_on = cmd & 0x01 != 0;
            }
            // Cursor/display shift. Shifting the *display* only moves which
            // DDRAM window the panel shows; the readback presents the unshifted
            // window, so only the cursor-move form changes observable state.
            0x10..=0x1F => {
                // bit3 = S/C (0 = cursor move, 1 = display shift),
                // bit2 = R/L (0 = left, 1 = right).
                if cmd & 0x08 == 0 {
                    let right = cmd & 0x04 != 0;
                    self.step_address(right);
                }
            }
            // Function set: bit4 = DL (bus width), bit3 = N (2-line), bit2 = F.
            0x20..=0x3F => self.two_line = cmd & 0x08 != 0,
            // Set CGRAM address.
            0x40..=0x7F => {
                self.cgram_mode = true;
                self.addr = (cmd & 0x3F) as usize;
            }
            // Set DDRAM address.
            0x80..=0xFF => {
                self.cgram_mode = false;
                self.addr = (cmd & 0x7F) as usize;
            }
        }
    }

    /// Write one byte at the address counter (`RS`=1), then advance it per the
    /// entry mode.
    fn handle_data(&mut self, byte: u8) {
        if self.cgram_mode {
            if let Some(slot) = self.cgram.get_mut(self.addr) {
                *slot = byte;
            }
        } else if let Some(idx) = Self::ddram_index(self.addr) {
            self.ddram[idx] = byte;
        }
        self.step_address(self.entry_increment);
    }

    /// Move the address counter one step, wrapping within its own address space
    /// the way the HD44780's counter rolls over.
    fn step_address(&mut self, increment: bool) {
        let modulus = if self.cgram_mode { CGRAM_LEN } else { 128 };
        self.addr = if increment {
            (self.addr + 1) % modulus
        } else {
            (self.addr + modulus - 1) % modulus
        };
    }
}

impl I2cDevice for Lcd1602 {
    fn address(&self) -> u8 {
        self.address
    }

    fn read(&mut self) -> u8 {
        // Reading the PCF8574 returns the P-port state. Drivers for this
        // backpack are write-only, and the LCD's busy flag is never polled
        // (they delay instead), so there is nothing meaningful to present.
        0
    }

    fn write(&mut self, data: u8) {
        // Every byte is a fresh snapshot of the eight expander pins.
        self.backlight = data & PIN_BACKLIGHT != 0;

        let enable = data & PIN_ENABLE != 0;
        let falling = self.prev_enable && !enable;
        self.prev_enable = enable;

        if !falling {
            return;
        }
        // RW=1 is an LCD read: the controller drives D7..D4, the expander does
        // not, so nothing is latched as a write.
        if data & PIN_RW != 0 {
            return;
        }
        self.latch_nibble(data >> 4, data & PIN_RS != 0);
    }

    fn stop(&mut self) {
        // A STOP does not deassert the expander pins — they hold their last
        // written state — so neither the E level nor a half-clocked byte is
        // reset here. Drivers routinely split a byte's two nibble pulses across
        // separate I²C transactions.
    }

    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};

pub struct Lcd1602Kit;
pub static LCD1602_KIT: Lcd1602Kit = Lcd1602Kit;

static LCD1602_METADATA: KitMetadata = KitMetadata {
    inputs: &[],
    device_type: "lcd1602",
    label: "LCD1602 Character Display (I2C)",
    summary: "16×2 HD44780 character LCD behind a PCF8574 I2C backpack.",
    detail: "The ubiquitous blue/green 1602 module with an I2C adapter soldered on. Each I2C byte \
             is a PCF8574 P-port snapshot (P7..P4 = D7..D4, P3 = backlight, P2 = E, P1 = RW, \
             P0 = RS); the HD44780 latches a nibble on every falling edge of E and two latches \
             make one instruction or character. The model tracks the full 80-byte DDRAM, the \
             address counter, entry mode, display/cursor/blink flags and the backlight line, and \
             the WASM bridge surfaces the visible 32 characters for the playground's display \
             overlay.",
    transport: Transport::I2c,
    category: Category::I2c,
    config_keys: &[ConfigKey {
        name: "i2c_address",
        ty: ConfigType::Int,
        doc: "7-bit slave address of the PCF8574 backpack. Defaults to 0x27 (PCF8574T); \
              0x3F is the other common strapping (PCF8574AT).",
    }],
    // No lab yet: no demo firmware/ELF is built or published for this module.
    // Declaring a LabRef would promise a one-click demo that 404s.
    labs: &[],
};

impl PeripheralKit for Lcd1602Kit {
    fn metadata(&self) -> &'static KitMetadata {
        &LCD1602_METADATA
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> anyhow::Result<()> {
        let address = ctx.i2c_address_or(0x27)?;
        ctx.attach_i2c_device(Box::new(Lcd1602::new(address)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::i2c::I2cDevice;

    /// Drive one nibble the way `LiquidCrystal_I2C::pulseEnable` does: present
    /// the data with `E` high, then drop `E` to latch.
    fn pulse_nibble(dev: &mut Lcd1602, nibble: u8, rs: bool) {
        let base = (nibble << 4) | PIN_BACKLIGHT | if rs { PIN_RS } else { 0 };
        dev.write(base | PIN_ENABLE);
        dev.write(base);
    }

    /// Send a full 8-bit value as its two nibbles, high half first.
    fn send_byte(dev: &mut Lcd1602, byte: u8, rs: bool) {
        pulse_nibble(dev, byte >> 4, rs);
        pulse_nibble(dev, byte & 0x0F, rs);
    }

    fn command(dev: &mut Lcd1602, cmd: u8) {
        send_byte(dev, cmd, false);
    }

    fn data(dev: &mut Lcd1602, byte: u8) {
        send_byte(dev, byte, true);
    }

    /// Turn the display on so `text()` reports DDRAM rather than a blank panel.
    fn display_on(dev: &mut Lcd1602) {
        command(dev, 0x0C);
    }

    #[test]
    fn falling_edge_latches_nibble_and_two_nibbles_make_a_byte() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);

        // High nibble of 'A' (0x41). E high then low = one latch; the byte is
        // still half-formed, so DDRAM must not have moved.
        pulse_nibble(&mut dev, 0x4, true);
        assert_eq!(
            dev.ddram()[0],
            BLANK,
            "a single nibble must not write DDRAM"
        );

        // Raising E without dropping it latches nothing either.
        dev.write(PIN_BACKLIGHT | PIN_ENABLE | PIN_RS | 0x10);
        assert_eq!(dev.ddram()[0], BLANK, "a rising edge must not latch");

        // Complete the pair with the low nibble.
        pulse_nibble(&mut dev, 0x1, true);
        assert_eq!(dev.ddram()[0], b'A', "two nibbles assemble one byte");
        assert_eq!(dev.address_counter(), 1);
    }

    #[test]
    fn writing_characters_lands_in_ddram_and_shows_in_text() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, b'H');
        data(&mut dev, b'i');

        assert_eq!(dev.ddram()[0], b'H');
        assert_eq!(dev.ddram()[1], b'i');

        let text = dev.text();
        assert_eq!(text.len(), 32, "panel is a flat 2×16 with no separator");
        assert!(text.starts_with("Hi"), "got {text:?}");
        assert_eq!(&text[..16], "Hi              ");
    }

    #[test]
    fn set_ddram_address_0xc0_moves_to_row_one() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, b'A');

        // 0x80 | 0x40 — the canonical "go to line 2, column 0".
        command(&mut dev, 0xC0);
        assert_eq!(dev.address_counter(), 0x40);
        data(&mut dev, b'B');

        // Address 0x40 is the start of the second packed line.
        assert_eq!(dev.ddram()[LINE_LEN], b'B');

        let text = dev.text();
        assert_eq!(&text[..16], "A               ");
        assert_eq!(
            &text[16..],
            "B               ",
            "row 1 occupies text()[16..]"
        );
    }

    #[test]
    fn clear_display_blanks_ddram_and_resets_the_address_counter() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, b'X');
        command(&mut dev, 0xC0);
        data(&mut dev, b'Y');
        assert_ne!(dev.address_counter(), 0);

        command(&mut dev, 0x01);
        assert_eq!(dev.address_counter(), 0, "clear returns the counter home");
        assert_eq!(
            dev.ddram(),
            [BLANK; DDRAM_LEN],
            "clear fills DDRAM with ' '"
        );
        assert_eq!(dev.text(), " ".repeat(32));
    }

    #[test]
    fn display_off_blanks_text_but_retains_ddram() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, b'O');
        data(&mut dev, b'K');
        assert!(dev.text().starts_with("OK"));

        // Display on/off control with D clear.
        command(&mut dev, 0x08);
        assert!(!dev.display_on());
        assert_eq!(dev.text(), " ".repeat(32), "panel must read blank");
        // The underlying RAM is untouched.
        assert_eq!(dev.ddram()[0], b'O');
        assert_eq!(dev.ddram()[1], b'K');

        // Back on: the stored characters reappear.
        command(&mut dev, 0x0C);
        assert!(dev.text().starts_with("OK"));
    }

    #[test]
    fn entry_mode_decrement_moves_the_address_backwards() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);

        // Park at address 5, then select decrement (entry mode set, I/D = 0).
        command(&mut dev, 0x80 | 0x05);
        command(&mut dev, 0x04);
        data(&mut dev, b'c');
        assert_eq!(dev.address_counter(), 4, "I/D=0 decrements the counter");
        data(&mut dev, b'b');
        assert_eq!(dev.address_counter(), 3);

        assert_eq!(dev.ddram()[5], b'c');
        assert_eq!(dev.ddram()[4], b'b');

        // Back to increment.
        command(&mut dev, 0x06);
        data(&mut dev, b'a');
        assert_eq!(dev.address_counter(), 4, "I/D=1 increments again");
        assert_eq!(dev.ddram()[3], b'a');
    }

    #[test]
    fn four_bit_init_nibble_sequence_does_not_corrupt_state() {
        let mut dev = Lcd1602::new(0x27);

        // The classic LiquidCrystal_I2C init: three 0x3 nibbles then 0x2, each
        // sent as a lone nibble before 4-bit mode is established.
        for nibble in [0x3, 0x3, 0x3, 0x2] {
            pulse_nibble(&mut dev, nibble, false);
        }

        // Nothing half-latched is left over, and no DDRAM/counter state moved.
        assert!(
            dev.pending_nibble.is_none(),
            "the even init sequence must leave no orphan half-byte"
        );
        assert_eq!(dev.address_counter(), 0);
        assert_eq!(dev.ddram(), [BLANK; DDRAM_LEN]);

        // The real init then continues in 4-bit mode and must land correctly.
        command(&mut dev, 0x28); // function set: 4-bit, 2-line, 5×8
        command(&mut dev, 0x0C); // display on, cursor off, blink off
        command(&mut dev, 0x06); // entry mode: increment, no shift
        command(&mut dev, 0x01); // clear

        assert!(dev.two_line(), "function set after init must take effect");
        assert!(dev.display_on());
        assert!(!dev.cursor_on());
        assert!(!dev.blink_on());

        data(&mut dev, b'R');
        data(&mut dev, b'e');
        data(&mut dev, b'a');
        data(&mut dev, b'd');
        data(&mut dev, b'y');
        assert!(dev.text().starts_with("Ready"), "got {:?}", dev.text());
    }

    #[test]
    fn backlight_line_tracks_p3() {
        let mut dev = Lcd1602::new(0x27);
        assert!(!dev.backlight(), "power-on default is unlit");

        dev.write(PIN_BACKLIGHT);
        assert!(dev.backlight());

        dev.write(0x00);
        assert!(!dev.backlight());
    }

    #[test]
    fn read_nibbles_are_not_latched_as_writes() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);

        // An RW=1 pulse: the LCD would be driving the bus, so nothing latches.
        let base = 0x40 | PIN_BACKLIGHT | PIN_RW | PIN_RS;
        dev.write(base | PIN_ENABLE);
        dev.write(base);
        assert!(dev.pending_nibble.is_none(), "RW=1 must not latch a write");

        // A normal write still pairs correctly afterwards.
        data(&mut dev, b'Z');
        assert_eq!(dev.ddram()[0], b'Z');
    }

    #[test]
    fn non_printable_bytes_render_as_spaces() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, 0x00); // custom glyph 0
        data(&mut dev, b'X');
        data(&mut dev, 0xDF); // katakana / degree sign in the HD44780 ROM

        let text = dev.text();
        assert_eq!(&text[..3], " X ", "non-ASCII code points blank out");
        assert_eq!(dev.ddram()[0], 0x00, "DDRAM keeps the raw byte");
        assert_eq!(dev.ddram()[2], 0xDF);
    }

    #[test]
    fn cgram_writes_are_parked_and_do_not_touch_ddram() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);
        data(&mut dev, b'A');

        // Set CGRAM address 0 and stream an 8-row glyph.
        command(&mut dev, 0x40);
        for row in 0..8u8 {
            data(&mut dev, row);
        }

        assert_eq!(dev.ddram()[0], b'A', "CGRAM writes must not reach DDRAM");
        assert_eq!(dev.ddram()[1], BLANK);
        assert_eq!(dev.cgram[..8], [0, 1, 2, 3, 4, 5, 6, 7]);

        // Set DDRAM address leaves CGRAM mode.
        command(&mut dev, 0x80 | 0x01);
        data(&mut dev, b'B');
        assert_eq!(dev.ddram()[1], b'B');
    }

    #[test]
    fn nibbles_split_across_i2c_transactions_still_pair() {
        let mut dev = Lcd1602::new(0x27);
        display_on(&mut dev);

        // Drivers routinely STOP between the two nibble pulses of one byte.
        pulse_nibble(&mut dev, 0x4, true);
        dev.stop();
        pulse_nibble(&mut dev, 0x2, true);
        dev.stop();

        assert_eq!(
            dev.ddram()[0],
            b'B',
            "a STOP must not drop the pending half"
        );
    }
}
