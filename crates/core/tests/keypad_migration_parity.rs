//! The deleted keypad model, copied verbatim, against the generic GPIO descriptor.
//! Both see identical stimuli and row snapshots; every drive on both seams is compared.
use labwired_config::DeviceDescriptor;
use labwired_core::bus::{BusResidentDevice, DevicePins};
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};
use labwired_core::sim_input::SimInput;
use labwired_core::{bus, sim_input};
use std::collections::BTreeMap;

#[allow(dead_code)]
mod oracle {
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
                key: std::borrow::Cow::Borrowed("key"),
                label: std::borrow::Cow::Borrowed("Key"),
                unit: std::borrow::Cow::Borrowed("index"),
                min: -1.0,
                max: (ROWS * COLS - 1) as f64,
                default: None,
            }];
            CH
        }

        fn set_input(
            &mut self,
            key: &str,
            value: f64,
        ) -> Result<(), crate::sim_input::SimInputError> {
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
        /// the register IO stays on the far side of the
        /// [`DevicePins`](crate::bus::DevicePins) port. An unreadable row defaults
        /// HIGH (an undriven row selects nothing). The keypad is combinational, so
        /// `now` is unused.
        fn service(&mut self, pins: &mut dyn crate::bus::DevicePins, _now: u64) {
            let row_outputs: [bool; ROWS] = std::array::from_fn(|r| {
                let (addr, bit) = self.row_odr[r];
                pins.output_bit(addr, bit).unwrap_or(true)
            });
            // Inherent `Keypad::service` (chosen over the trait method — inherent
            // methods win resolution) recomputes the columns + change flags.
            let cols = self.service(row_outputs);
            for (c, &(high, changed)) in cols.iter().enumerate().take(COLS) {
                if changed {
                    let (addr, bit) = self.col_idr[c];
                    // Both seams — see the note in `rotary_encoder.rs`. A bare
                    // `drive_idr_bit` is an MMIO store, so this matrix was inert on
                    // every part whose input word is read-only (EFR32 DIN, SAM IN,
                    // ESP32-C3): a key could be held down and no column ever fell.
                    let _ = pins.drive_input_bit(addr, bit, high);
                    pins.drive_idr_bit(addr, bit, high);
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
}

#[derive(Default, Clone)]
struct Pads {
    rows: BTreeMap<u8, bool>,
    levels: BTreeMap<u8, bool>,
    drives: Vec<(bool, u8, bool)>,
}
impl DevicePins for Pads {
    fn output_bit(&self, _: u64, bit: u8) -> Option<bool> {
        self.rows.get(&bit).copied()
    }
    fn drive_idr_bit(&mut self, _: u64, bit: u8, high: bool) {
        self.levels.insert(bit, high);
        self.drives.push((false, bit, high));
    }
    fn drive_input_bit(&mut self, _: u64, bit: u8, high: bool) -> bool {
        self.drives.push((true, bit, high));
        true
    }
}
fn descriptor() -> DeviceDescriptor {
    DeviceDescriptor::embedded("keypad").unwrap().unwrap()
}
fn device(desc: &DeviceDescriptor) -> DeclarativeGpioDevice {
    let observed = (0..4)
        .map(|i| BoundPin {
            role: format!("rows[{i}]"),
            addr: 1,
            bit: i,
        })
        .collect();
    let driven = (0..4)
        .map(|i| BoundPin {
            role: format!("cols[{i}]"),
            addr: 2,
            bit: i,
        })
        .collect();
    DeclarativeGpioDevice::new(
        "pad".into(),
        desc,
        observed,
        driven,
        1_000_000,
        std::borrow::Cow::Owned(vec![labwired_core::sim_input::InputChannel {
            key: "key".into(),
            label: "Key".into(),
            unit: "index".into(),
            min: -1.0,
            max: 15.0,
            default: None,
        }]),
    )
    .unwrap()
}
fn compare(desc: &DeviceDescriptor) -> bool {
    let mut old = oracle::Keypad::new(
        "pad".into(),
        std::array::from_fn(|r| (1, r as u8)),
        std::array::from_fn(|c| (2, c as u8)),
    );
    let mut new = device(desc);
    assert_eq!(old.needs_per_cycle_service(), new.needs_per_cycle_service());
    assert_eq!(old.input_channels(), new.input_channels());
    let mut a = Pads::default();
    let mut b = Pads::default();
    // Start unreadable, then every key/no-key, rounding edges and release.
    let values = std::iter::once(-1.0)
        .chain((0..16).map(f64::from))
        .chain([-0.6, -0.4, 0.49, 0.5, 7.49, 7.5, 14.5, -1.0]);
    for value in values {
        old.set_input("key", value).unwrap();
        new.set_input("key", value).unwrap();
        // None tests unreadable rows; all 16 masks include multi-row writes and no row selected.
        for mask in std::iter::once(None).chain((0..16u8).map(Some)) {
            a.rows = (0..4)
                .filter_map(|r| mask.map(|m| (r, m & (1 << r) != 0)))
                .collect();
            b.rows = a.rows.clone();
            for _ in 0..2 {
                a.drives.clear();
                b.drives.clear();
                BusResidentDevice::service(&mut old, &mut a, 0);
                new.service(&mut b, 0);
                if a.levels != b.levels || a.drives != b.drives {
                    return false;
                }
            }
        }
    }
    // Retarget with rows unchanged; stimulus alone must recompute columns.
    a.rows = (0..4).map(|r| (r, false)).collect();
    b.rows = a.rows.clone();
    for value in [0.0, 5.0, 10.0, 15.0, -1.0] {
        old.set_input("key", value).unwrap();
        new.set_input("key", value).unwrap();
        a.drives.clear();
        b.drives.clear();
        BusResidentDevice::service(&mut old, &mut a, 0);
        new.service(&mut b, 0);
        if a.levels != b.levels || a.drives != b.drives {
            return false;
        }
    }
    true
}
#[test]
fn every_key_row_snapshot_release_and_stimulus_matches_verbatim_oracle() {
    assert!(compare(&descriptor()));
}
#[test]
fn negative_control_wrong_column_level_is_detected() {
    let mut d = descriptor();
    for rule in &mut d.behavior.rules {
        for action in &mut rule.actions {
            if let labwired_config::Action::Pin { level, .. } = action {
                *level = "0".into();
            }
        }
    }
    assert!(!compare(&d));
}
#[test]
fn negative_control_missing_input_rule_is_detected() {
    let mut d = descriptor();
    d.behavior
        .rules
        .retain(|r| !matches!(r.on, labwired_config::Event::Input { .. }));
    assert!(!compare(&d));
}

#[test]
fn default_gpio_output_policy_preserves_pulses_and_action_order() {
    let desc = DeviceDescriptor::from_yaml(
        r#"
type: pulse-test
behavior:
  primitive: gpio_device
  outputs: ["cols[0]", "cols[1]"]
  rules:
    - on: { input: key }
      do:
        - { pin: "cols[1]", level: 1 }
        - { pin: "cols[0]", level: 1 }
        - { pin: "cols[1]", level: 0 }
metadata:
  inputs: [{ key: key, label: Key, unit: index, min: -1, max: 15 }]
"#,
    )
    .unwrap();
    let mut dev = device(&desc);
    dev.set_input("key", 0.0).unwrap();
    let mut pads = Pads::default();
    dev.service(&mut pads, 0);
    assert_eq!(
        pads.drives,
        [
            (true, 1, true),
            (false, 1, true),
            (true, 0, true),
            (false, 0, true),
            (true, 1, false),
            (false, 1, false)
        ]
    );
}
