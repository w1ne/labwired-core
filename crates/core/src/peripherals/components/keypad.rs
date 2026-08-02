// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! 4×4 matrix keypad — sixteen passive switches at row/column intersections.
//!
//! A membrane keypad has no chip: each key `(r, c)` is a momentary switch that
//! simply shorts **row `r`** to **column `c`** while pressed. Nothing is driven
//! spontaneously — the MCU animates the matrix and reads the result back:
//!
//! ```text
//!            C1   C2   C3   C4          (columns: MCU inputs, pull-up → idle HIGH)
//!          ┌─┴────┴────┴────┴─
//!   R1 ────┤  o    o    o    o          key (r,c) = a switch bridging Rr and Cc
//!   R2 ────┤  o    o    o    o
//!   R3 ────┤  o    o    o    o          (rows: MCU outputs, driven one LOW at a time)
//!   R4 ────┤  o    o    o    o
//! ```
//!
//! Firmware scans it the standard way — **drive rows, read columns**:
//!
//! 1. Drive one ROW output LOW, the other three HIGH.
//! 2. Read the four COLUMN inputs (each has a pull-up, so idle HIGH).
//! 3. A column reads LOW exactly when a pressed key bridges it to the row that
//!    is currently LOW. `(row_driven_low, col_read_low)` identifies the key.
//! 4. Repeat for each row.
//!
//! ## Why it lives on the bus (not as an MMIO peripheral)
//!
//! Like [`RotaryEncoder`](crate::peripherals::components::rotary_encoder::RotaryEncoder)
//! and [`Dht22`](crate::peripherals::components::dht22::Dht22), the keypad is a
//! device that **drives pins the MCU samples as inputs** (the columns) while
//! *observing* pins the MCU drives as outputs (the rows). It answers no register
//! read, so it can't be a memory-mapped peripheral. The
//! [`SystemBus`](crate::bus::SystemBus) holds a list of [`Keypad`] links; a
//! cheap per-tick pass reads the four row **output** (ODR) bits, recomputes the
//! four column levels, and drives each column's **input** (IDR) bit — touching
//! the bus only when a level changes, exactly as the DHT22/encoder passes do.
//!
//! ## Fidelity boundary
//!
//! This models the common **row-drive / column-read** scan, which is what the
//! MCU-GPIO surface needs to be faithful: the columns the firmware samples carry
//! the right levels for whatever the firmware drives on the rows, at any scan
//! rate, with any settling delay. The reverse scan (drive columns, read rows) is
//! symmetric on real hardware but is **not** modelled — it is out of scope for
//! the common case. Contact bounce, ghosting across multiple simultaneous
//! presses, and n-key rollover are likewise not modelled: one key is pressed at
//! a time, which is the faithful single-touch case a scan loop is written for.
//!
//! ## Stimulus
//!
//! The pressed key is host-controlled through the standard stimulus API: a
//! single float channel, `key`, whose value is the linear index `row*4 + col`
//! (0..15). A negative value (e.g. `-1`) means *no key pressed* — the idle
//! state, all columns pulled high.

/// Rows and columns in the matrix. A 4×4 keypad has four of each; the model is
/// written against these constants rather than hard-coded `4`s so the intent of
/// each loop is legible.
pub const ROWS: usize = 4;
pub const COLS: usize = 4;

/// One 4×4 matrix keypad wired to four ROW output pins and four COLUMN input
/// pins.
#[derive(Debug, Clone)]
pub struct Keypad {
    /// board_io / external-device id — targets the `key` setter.
    pub id: String,
    /// Absolute address + bit of each ROW's GPIO **output** register (ODR). The
    /// model reads these to learn which row the firmware is currently driving
    /// LOW; index `r` is row `r` (`R1`..`R4`).
    pub row_odr: [(u64, u8); ROWS],
    /// Absolute address + bit of each COLUMN's GPIO **input** register (IDR).
    /// The model drives these so the firmware reads the scan result; index `c`
    /// is column `c` (`C1`..`C4`).
    pub col_idr: [(u64, u8); COLS],

    /// The currently pressed key as `(row, col)`, or `None` when nothing is
    /// pressed (all columns idle high).
    pressed: Option<(u8, u8)>,
    /// Last column level this keypad drove onto each input register; `None`
    /// forces the first drive so the columns settle at their idle-high value at
    /// boot (the IDR bits reset to 0).
    last_col_high: [Option<bool>; COLS],
}

impl Keypad {
    pub fn new(id: String, row_odr: [(u64, u8); ROWS], col_idr: [(u64, u8); COLS]) -> Self {
        Self {
            id,
            row_odr,
            col_idr,
            pressed: None,
            last_col_high: [None; COLS],
        }
    }

    /// The currently pressed key as `(row, col)`, or `None`. Exposed for tests
    /// and UI readback.
    pub fn pressed(&self) -> Option<(u8, u8)> {
        self.pressed
    }

    /// Press key `(row, col)`. Both are taken modulo the matrix size so an
    /// out-of-range index (should never happen — the channel is range-checked)
    /// still lands on a real key rather than panicking.
    pub fn set_pressed(&mut self, key: Option<(u8, u8)>) {
        self.pressed = key.map(|(r, c)| (r % ROWS as u8, c % COLS as u8));
    }

    /// The level each column reads for the given row **output** levels: a column
    /// is LOW iff the pressed key bridges it to a row that is currently driven
    /// LOW, otherwise HIGH (its pull-up). `row_outputs[r]` is row `r`'s output
    /// level (`true` = high).
    ///
    /// Pure query — no state change — so tests can check the combinational truth
    /// table directly. [`service`](Self::service) wraps it with change tracking.
    pub fn column_levels(&self, row_outputs: [bool; ROWS]) -> [bool; COLS] {
        let mut cols = [true; COLS]; // idle: every column pulled high
        if let Some((pr, pc)) = self.pressed {
            // The pressed key shorts row `pr` to column `pc`, so that column
            // follows row `pr`'s output level — it reads LOW only while the
            // firmware is driving that row LOW.
            cols[pc as usize] = row_outputs[pr as usize];
        }
        cols
    }

    /// Service the keypad against the current row **output** levels: recompute
    /// the four column levels and report, per column, `(col_high, changed)`
    /// where `changed` is whether the level differs from the last one driven (so
    /// the bus can skip untouched columns). Mirrors
    /// [`RotaryEncoder::service`](crate::peripherals::components::rotary_encoder::RotaryEncoder::service).
    pub fn service(&mut self, row_outputs: [bool; ROWS]) -> [(bool, bool); COLS] {
        let cols = self.column_levels(row_outputs);
        let mut out = [(true, false); COLS];
        for c in 0..COLS {
            let high = cols[c];
            let changed = self.last_col_high[c] != Some(high);
            self.last_col_high[c] = Some(high);
            out[c] = (high, changed);
        }
        out
    }
}

/// Drivable pressed key, as the linear index `row*4 + col` (0..15); a negative
/// value releases (no key pressed). Keypads live directly on the bus
/// (`SystemBus::gpio_devices`), so the bus input walk reaches this impl and reports
/// each keypad under its `id` — same as the rotary encoder and DHT22.
impl crate::sim_input::SimInput for Keypad {
    fn input_channels(&self) -> &'static [crate::sim_input::InputChannel] {
        use crate::sim_input::InputChannel;
        const CH: &[InputChannel] = &[InputChannel {
            key: "key",
            label: "Key",
            unit: "index",
            min: -1.0,
            max: (ROWS * COLS - 1) as f64,
        }];
        CH
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        let idx = value.round() as i64;
        if idx < 0 {
            self.set_pressed(None);
        } else {
            let idx = idx as u8;
            self.set_pressed(Some((idx / COLS as u8, idx % COLS as u8)));
        }
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        Some(&self.id)
    }
}

impl crate::bus::BusResidentDevice for Keypad {
    /// Read the four ROW output (ODR) bits, recompute the four COLUMN levels for
    /// the pressed key, and drive each changed COLUMN input (IDR) bit. This is
    /// the body of the former `SystemBus::drive_keypad`, moved onto the device;
    /// the register IO stays on the bus via
    /// [`drive_idr_bit`](crate::bus::SystemBus). An unreadable row defaults HIGH
    /// (an undriven row selects nothing). The keypad is combinational, so `now`
    /// is unused.
    fn service(&mut self, bus: &mut crate::bus::SystemBus, _now: u64) {
        use crate::Bus; // `read_u32` is a Bus-trait method
        let row_outputs: [bool; ROWS] = std::array::from_fn(|r| {
            let (addr, bit) = self.row_odr[r];
            bus.read_u32(addr)
                .map(|v| (v >> bit) & 1 != 0)
                .unwrap_or(true)
        });
        // Inherent `Keypad::service` (chosen over the trait method — inherent
        // methods win resolution) recomputes the columns + change flags.
        let cols = self.service(row_outputs);
        for (c, &(high, changed)) in cols.iter().enumerate().take(COLS) {
            if changed {
                let (addr, bit) = self.col_idr[c];
                bus.drive_idr_bit(addr, bit, high);
            }
        }
    }

    fn as_sim_input(&mut self) -> &mut dyn crate::sim_input::SimInput {
        self
    }

    fn id(&self) -> &str {
        &self.id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row ODR bits 0..3 on one port, column IDR bits 4..7 on another — the
    /// addresses are irrelevant to the matrix logic, only the wiring matters.
    fn pad() -> Keypad {
        Keypad::new(
            "kp".into(),
            [
                (0x4800_0014, 0),
                (0x4800_0014, 1),
                (0x4800_0014, 2),
                (0x4800_0014, 3),
            ],
            [
                (0x4800_0010, 4),
                (0x4800_0010, 5),
                (0x4800_0010, 6),
                (0x4800_0010, 7),
            ],
        )
    }

    /// Firmware-style single-row scan: drive `row` LOW (others HIGH), return the
    /// four column levels the keypad reports.
    fn scan_row(kp: &Keypad, row: usize) -> [bool; COLS] {
        let outputs = std::array::from_fn(|r| r != row); // only `row` is LOW
        kp.column_levels(outputs)
    }

    #[test]
    fn idle_keypad_reads_all_columns_high() {
        let kp = pad();
        for row in 0..ROWS {
            assert_eq!(scan_row(&kp, row), [true; COLS], "row {row} idle");
        }
    }

    #[test]
    fn pressed_key_pulls_its_column_low_only_under_its_row() {
        let mut kp = pad();
        kp.set_pressed(Some((1, 2))); // key at row 1, column 2

        // Driving row 1 LOW: only column 2 reads LOW.
        assert_eq!(scan_row(&kp, 1), [true, true, false, true], "row 1 driven");

        // Driving any other row LOW: all columns stay HIGH (the pressed key does
        // not bridge that row to its column while the row is HIGH).
        for row in [0, 2, 3] {
            assert_eq!(scan_row(&kp, row), [true; COLS], "row {row} — key idle");
        }
    }

    #[test]
    fn releasing_returns_every_column_high() {
        let mut kp = pad();
        kp.set_pressed(Some((0, 0)));
        assert_eq!(scan_row(&kp, 0), [false, true, true, true]);
        kp.set_pressed(None);
        for row in 0..ROWS {
            assert_eq!(scan_row(&kp, row), [true; COLS], "released, row {row}");
        }
    }

    #[test]
    fn a_different_key_isolates_to_its_own_intersection() {
        let mut kp = pad();
        kp.set_pressed(Some((3, 0))); // row 3, column 0
        assert_eq!(scan_row(&kp, 3), [false, true, true, true], "row 3 → col 0");
        // The same column under a different row must NOT read low.
        assert_eq!(scan_row(&kp, 0), [true; COLS], "col 0 idle under row 0");
    }

    #[test]
    fn service_flags_only_the_columns_that_changed() {
        let mut kp = pad();
        kp.set_pressed(Some((1, 2)));

        // First service settles all four columns from `None` → flagged changed.
        let all_high = [true; ROWS];
        let out = kp.service(all_high);
        assert_eq!(out, [(true, true); COLS], "initial drive settles all cols");

        // Drive row 1 low: only column 2 toggles high→low.
        let mut outputs = [true; ROWS];
        outputs[1] = false;
        let out = kp.service(outputs);
        assert_eq!(
            out,
            [(true, false), (true, false), (false, true), (true, false)],
            "only column 2 changed"
        );

        // Same levels again: column 2 stays LOW, nothing is flagged changed.
        let out = kp.service(outputs);
        assert_eq!(
            out,
            [(true, false), (true, false), (false, false), (true, false)],
            "steady state → no writes"
        );
    }

    #[test]
    fn set_input_maps_the_index_to_row_and_col() {
        use crate::sim_input::SimInput;
        let mut kp = pad();
        assert_eq!(kp.input_channels()[0].key, "key");

        kp.set_input("key", 6.0).unwrap(); // 6 = row 1, col 2
        assert_eq!(kp.pressed(), Some((1, 2)));

        kp.set_input("key", 15.0).unwrap(); // last key = row 3, col 3
        assert_eq!(kp.pressed(), Some((3, 3)));

        kp.set_input("key", -1.0).unwrap(); // release
        assert_eq!(kp.pressed(), None);

        // Out of range and unknown channels are rejected, not clamped.
        assert!(kp.set_input("key", 16.0).is_err(), "index 16 out of range");
        assert!(kp.set_input("button", 1.0).is_err(), "unknown channel");
    }

    /// End-to-end through the bus with a standard firmware-style scan: build a
    /// bus with two GPIO ports (rows as outputs, columns as inputs), press a
    /// key, then drive each row LOW in turn and read the columns — recovering
    /// exactly the `(row, col)` that was pressed. This is the same row-drive /
    /// column-read loop a real matrix-keypad firmware runs.
    #[test]
    fn matrix_scan_recovers_the_key_through_the_bus() {
        use crate::bus::SystemBus;
        use crate::peripherals::gpio::{GpioPort, GpioRegisterLayout};
        use crate::Bus;

        // stm32v2 GPIO: IDR @ 0x10, ODR @ 0x14, BSRR @ 0x18.
        const GPIOA: u64 = 0x4800_0000; // rows (outputs) live here
        const GPIOB: u64 = 0x4800_0400; // columns (inputs) live here
        const ODR: u64 = GPIOA + 0x14;
        const IDR: u64 = GPIOB + 0x10;

        let mut bus = SystemBus::empty();
        bus.add_peripheral(
            "gpioa",
            GPIOA,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );
        bus.add_peripheral(
            "gpiob",
            GPIOB,
            0x400,
            None,
            Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
        );

        // Rows R1..R4 → ODR bits 0..3; columns C1..C4 → IDR bits 0..3.
        let row_odr: [(u64, u8); ROWS] = std::array::from_fn(|r| (ODR, r as u8));
        let col_idr: [(u64, u8); COLS] = std::array::from_fn(|c| (IDR, c as u8));
        bus.gpio_devices
            .push(Box::new(Keypad::new("kp".into(), row_odr, col_idr)));

        // Firmware presses key (2, 1) via the component (as a stimulus would).
        bus.gpio_devices_of_mut::<Keypad>()
            .next()
            .unwrap()
            .set_pressed(Some((2, 1)));

        // Read column `c`'s input bit.
        let read_col = |bus: &SystemBus, c: u8| (bus.read_u32(IDR).unwrap() >> c) & 1 != 0;

        // Scan: drive each row LOW (others HIGH), service the keypad, read cols.
        let mut found: Option<(u8, u8)> = None;
        for row in 0..ROWS as u8 {
            // Drive rows: `row` LOW, the rest HIGH (ODR bits 0..3).
            let odr = (0b1111u32) & !(1 << row);
            bus.write_u32(ODR, odr).unwrap();
            bus.set_current_cycle(row as u64);
            bus.service_gpio_devices();
            for col in 0..COLS as u8 {
                if !read_col(&bus, col) {
                    found = Some((row, col));
                }
            }
        }
        assert_eq!(found, Some((2, 1)), "scan recovers the pressed key");

        // Release → a full scan finds nothing.
        bus.gpio_devices_of_mut::<Keypad>()
            .next()
            .unwrap()
            .set_pressed(None);
        for row in 0..ROWS as u8 {
            let odr = (0b1111u32) & !(1 << row);
            bus.write_u32(ODR, odr).unwrap();
            bus.set_current_cycle(100 + row as u64);
            bus.service_gpio_devices();
            for col in 0..COLS as u8 {
                assert!(
                    read_col(&bus, col),
                    "released: col {col} high under row {row}"
                );
            }
        }
    }

    /// The keypad is reachable from the ONE bus stimulus walk, so `set_input` /
    /// `list_inputs` (test-script `stimuli:`, MCP, wasm) all see it.
    #[test]
    fn reachable_from_the_bus_input_walk() {
        use crate::bus::SystemBus;

        let mut bus = SystemBus::empty();
        bus.gpio_devices.push(Box::new(pad()));

        let channels = bus.list_inputs();
        assert!(
            channels
                .iter()
                .any(|(owner, ch)| owner == "kp" && ch.key == "key"),
            "key channel not discovered: {channels:?}"
        );
    }
}
