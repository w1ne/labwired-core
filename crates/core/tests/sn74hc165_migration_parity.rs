// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **`sn74hc165.yaml` against the `components/sn74hc165.rs` it replaces.**
//!
//! The model is DELETED. It is kept below as an ORACLE, copied verbatim, and
//! every test drives the SAME script object through both implementations —
//! one list of `(channel, value)` stimulus plus one list of clocked bytes,
//! applied to each — rather than two scripts that look alike.
//!
//! ## What is actually under test
//!
//! 1. **The eight channels exist at all**, under the same keys and labels the
//!    deleted model published (`ch0..ch7` / `D0..D7`), from ONE declared
//!    `metadata.inputs` entry carrying `bits: { count: 8 }`.
//! 2. **`inputs: 165` reaches those channels.** This is the key the port was
//!    blocked on (PR #1186): four shipped manifests set it, descriptor seeding
//!    was one float per channel, so the port would have shipped a `config:` key
//!    that parses and changes nothing. Asserted at all 256 seed values.
//! 3. **Byte parity** of the clocked-out word across all 256 input states, and
//!    at the `>= 0.5` threshold that separates a low channel from a high one.
//! 4. **One DELIBERATE difference, named**: past the eighth bit of a held-CS
//!    burst the descriptor answers open bus where the model repeated the byte.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::Bus;
use std::collections::HashMap;
use std::path::PathBuf;

// ─── the oracle: the deleted model, verbatim ───────────────────────────────
//
// ⚠️ COPIED, not imported: the file is gone. `#![allow(dead_code)]` because
// trimming the fields no test reads (`cs_pin`, `component_id`, `latched`) would
// make this something other than what was deleted.
#[allow(dead_code)]
mod oracle {
    use labwired_core::peripherals::spi::SpiDevice;
    use std::any::Any;

    #[derive(Debug)]
    pub struct Sn74hc165 {
        cs_pin: String,
        inputs: u8,
        latched: u8,
        component_id: Option<String>,
    }

    impl Sn74hc165 {
        pub fn new(cs_pin: impl Into<String>) -> Self {
            Self {
                cs_pin: cs_pin.into(),
                inputs: 0,
                latched: 0,
                component_id: None,
            }
        }

        pub fn set_inputs(&mut self, value: u8) {
            self.inputs = value;
        }

        pub fn set_channel(&mut self, ch: u8, high: bool) {
            if ch < 8 {
                if high {
                    self.inputs |= 1 << ch;
                } else {
                    self.inputs &= !(1 << ch);
                }
            }
        }

        pub fn inputs(&self) -> u8 {
            self.inputs
        }
    }

    impl SpiDevice for Sn74hc165 {
        fn cs_pin(&self) -> &str {
            &self.cs_pin
        }

        fn cs_select(&mut self) {
            self.latched = self.inputs;
        }

        fn cs_release(&mut self) {}

        fn transfer(&mut self, _mosi: u8) -> u8 {
            self.latched = self.inputs;
            self.latched
        }

        fn as_any(&self) -> Option<&dyn Any> {
            Some(self)
        }

        fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
            Some(self)
        }
    }

    /// The deleted model's channel table, verbatim.
    pub const INPUT_CHANNELS: &[(&str, &str, &str, f64, f64)] = &[
        ("ch0", "D0", "level", 0.0, 1.0),
        ("ch1", "D1", "level", 0.0, 1.0),
        ("ch2", "D2", "level", 0.0, 1.0),
        ("ch3", "D3", "level", 0.0, 1.0),
        ("ch4", "D4", "level", 0.0, 1.0),
        ("ch5", "D5", "level", 0.0, 1.0),
        ("ch6", "D6", "level", 0.0, 1.0),
        ("ch7", "D7", "level", 0.0, 1.0),
    ];

    /// The deleted model's `SimInput::set_input`, verbatim: high at `>= 0.5`.
    pub fn set_input(d: &mut Sn74hc165, key: &str, value: f64) {
        let ch = key.strip_prefix("ch").unwrap().parse::<u8>().unwrap();
        d.set_channel(ch, value >= 0.5);
    }
}

// ─── the rig: the real attach path, the real SPI controller ────────────────

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

const RCC_APB2ENR: u64 = 0x4002_1018;

/// One `sn74hc165` on an STM32F103's `spi1`, built through
/// `SystemBus::from_config` — the real kit registry and the real attach path,
/// so a descriptor that fails to register fails here.
fn rig(config: &[(&str, serde_yaml::Value)]) -> SystemBus {
    let chip_path = repo("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut cfg = HashMap::new();
    for (k, v) in config {
        cfg.insert(k.to_string(), v.clone());
    }
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "hc165-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "dio".to_string(),
            r#type: "sn74hc165".to_string(),
            connection: "spi1".to_string(),
            channel: None,
            route: Default::default(),
            config: cfg,
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    bus.write_u32(RCC_APB2ENR, 0xFFFF).expect("ungate SPI");
    bus
}

fn s(v: &str) -> serde_yaml::Value {
    serde_yaml::Value::from(v)
}

fn i(v: i64) -> serde_yaml::Value {
    serde_yaml::Value::from(v)
}

/// Run a closure over the attached device, through the controller's own list —
/// nothing here reaches into a model field.
fn with_dev<R>(bus: &mut SystemBus, f: impl FnOnce(&mut Box<dyn SpiDevice>) -> R) -> R {
    let idx = bus
        .find_peripheral_index_by_name("spi1")
        .expect("spi1 registered");
    let any = bus.peripherals[idx].dev.as_any_mut().expect("downcastable");
    let spi = any
        .downcast_mut::<labwired_core::peripherals::spi::Spi>()
        .expect("generic Spi controller");
    f(spi.attached_devices.first_mut().expect("device attached"))
}

/// ONE script object: the stimulus to apply, then the bytes to clock. Both
/// implementations are driven through this, so there is no second script to
/// drift.
struct Script {
    stimulus: Vec<(&'static str, f64)>,
    clocks: usize,
    /// Hold CS across the whole burst (the shape no shipped manifest uses; see
    /// the named difference below).
    hold_cs: bool,
}

fn run_descriptor(bus: &mut SystemBus, script: &Script) -> Vec<u8> {
    with_dev(bus, |dev| {
        let si = dev
            .as_sim_input_mut()
            .expect("the ported part serves SimInput");
        for (key, value) in &script.stimulus {
            si.set_input(key, *value).expect("declared channel");
        }
        if script.hold_cs {
            dev.cs_select();
        }
        let out: Vec<u8> = (0..script.clocks).map(|_| dev.transfer(0x00)).collect();
        if script.hold_cs {
            dev.cs_release();
        }
        out
    })
}

fn run_oracle(script: &Script) -> Vec<u8> {
    let mut d = oracle::Sn74hc165::new("PA4");
    for (key, value) in &script.stimulus {
        oracle::set_input(&mut d, key, *value);
    }
    if script.hold_cs {
        d.cs_select();
    }
    let out: Vec<u8> = (0..script.clocks).map(|_| d.transfer(0x00)).collect();
    if script.hold_cs {
        d.cs_release();
    }
    out
}

fn stimulus_for(byte: u8) -> Vec<(&'static str, f64)> {
    const KEYS: [&str; 8] = ["ch0", "ch1", "ch2", "ch3", "ch4", "ch5", "ch6", "ch7"];
    KEYS.iter()
        .enumerate()
        .map(|(b, k)| (*k, f64::from((byte >> b) & 1)))
        .collect()
}

// ─── the channel table ─────────────────────────────────────────────────────

/// ONE declared entry with `bits: { count: 8 }` must publish exactly the eight
/// channels the deleted model published — key, label, unit and range.
#[test]
fn the_bits_group_publishes_the_deleted_models_channel_table() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    let got: Vec<(String, String, String, f64, f64)> = with_dev(&mut bus, |dev| {
        dev.as_sim_input_mut()
            .expect("SimInput")
            .input_channels()
            .iter()
            .map(|c| {
                (
                    c.key.to_string(),
                    c.label.to_string(),
                    c.unit.to_string(),
                    c.min,
                    c.max,
                )
            })
            .collect()
    });
    let want: Vec<(String, String, String, f64, f64)> = oracle::INPUT_CHANNELS
        .iter()
        .map(|(k, l, u, lo, hi)| (k.to_string(), l.to_string(), u.to_string(), *lo, *hi))
        .collect();
    assert_eq!(got, want, "the `bits:` fan-out must reproduce the table");
}

/// The NEGATIVE control for the test above: the descriptor declares ONE entry,
/// not eight. Without this, the assertion would also pass on a descriptor that
/// spelled all eight out by hand and left `inputs:` dead — which is exactly the
/// state PR #1186 refused to ship.
#[test]
fn the_descriptor_declares_one_entry_not_eight() {
    let yaml = labwired_config::embedded_device_yaml("sn74hc165").expect("embedded");
    let raw: serde_yaml::Value = serde_yaml::from_str(yaml).expect("parse");
    let inputs = raw["metadata"]["inputs"].as_sequence().expect("inputs");
    assert_eq!(
        inputs.len(),
        1,
        "one `bits:` group, not eight hand-written channels"
    );
    assert_eq!(inputs[0]["bits"]["count"].as_u64(), Some(8));
    assert_eq!(inputs[0]["bits"]["config_key"].as_str(), Some("inputs"));
}

// ─── byte parity ───────────────────────────────────────────────────────────

/// Every one of the 256 input states, clocked once, compared byte for byte.
#[test]
fn every_input_state_clocks_out_the_same_byte_as_the_deleted_model() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    for byte in 0u8..=255 {
        let script = Script {
            stimulus: stimulus_for(byte),
            clocks: 1,
            hold_cs: false,
        };
        assert_eq!(
            run_descriptor(&mut bus, &script),
            run_oracle(&script),
            "input state 0x{byte:02X}"
        );
        assert_eq!(
            run_oracle(&script),
            vec![byte],
            "oracle control: MSB-first QH..QA lands verbatim"
        );
    }
}

/// The threshold. The deleted model's `set_input` was `value >= 0.5`; the
/// descriptor's default `encode:` is scale 1 with `round: nearest` (half away
/// from zero), which is the same cut. Measured either side of it, and ON it.
#[test]
fn the_high_low_threshold_is_the_same_half_way_cut() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    for v in [0.0, 0.25, 0.49, 0.5, 0.51, 0.75, 1.0] {
        let script = Script {
            stimulus: vec![("ch0", v)],
            clocks: 1,
            hold_cs: false,
        };
        assert_eq!(
            run_descriptor(&mut bus, &script),
            run_oracle(&script),
            "ch0 = {v}"
        );
    }
}

// ─── the key this port exists to prove ─────────────────────────────────────

/// `inputs: N` — ONE integer seeding eight channels. At every one of the 256
/// values, against the deleted kit's `set_inputs(v as u8)`.
#[test]
fn the_inputs_integer_seeds_all_eight_channels() {
    for seed in 0u8..=255 {
        let mut bus = rig(&[("cs_pin", s("PA4")), ("inputs", i(i64::from(seed)))]);
        let got = with_dev(&mut bus, |dev| dev.transfer(0x00));

        let mut oracle = oracle::Sn74hc165::new("PA4");
        oracle.set_inputs(seed); // the deleted kit's `attach`, verbatim
        assert_eq!(
            got,
            oracle.transfer(0x00),
            "config `inputs: {seed}` must reach all eight channels"
        );
    }
}

/// The NEGATIVE control: a rig with NO `inputs:` key must read 0. Without it,
/// the sweep above would also pass on an engine that ignored the key and
/// happened to... no, it would not — but it would pass on one that seeded every
/// channel high, which is the other way to be wrong.
#[test]
fn without_the_inputs_key_every_channel_starts_low() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    assert_eq!(with_dev(&mut bus, |dev| dev.transfer(0x00)), 0x00);
}

/// A per-channel override still wins over the group seed, so `inputs: 0xA5`
/// plus `ch1: 1` means what it reads like.
#[test]
fn a_single_channel_key_overrides_one_bit_of_the_group_seed() {
    let mut bus = rig(&[
        ("cs_pin", s("PA4")),
        ("inputs", i(0xA5)),
        ("ch1", i(1)),
        ("ch0", i(0)),
    ]);
    assert_eq!(
        with_dev(&mut bus, |dev| dev.transfer(0x00)),
        0xA6,
        "0xA5 with bit 1 set and bit 0 cleared"
    );
}

// ─── the named difference ──────────────────────────────────────────────────

/// ⚠️ DELIBERATE, and asserted AS a difference.
///
/// Inside one HELD-CS burst the deleted model re-sampled the live inputs on
/// every `transfer` and so answered the same byte for ever; the descriptor
/// clocks the eight-bit word out once and then reads open bus. Neither is the
/// datasheet's answer — a real 74HC165 shifts whatever SER carries into stage A
/// on the ninth clock — so the model's repeat is an artefact rather than a
/// behaviour worth preserving.
///
/// Unobservable on every shipped manifest: the STM32 SPI bus never drives the
/// CS callbacks, which is the `hold_cs: false` path the parity sweep above
/// measures, and there each byte re-frames and re-samples.
#[test]
fn a_held_cs_burst_past_eight_bits_is_open_bus_not_a_repeat() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    let script = Script {
        stimulus: stimulus_for(0xA5),
        clocks: 3,
        hold_cs: true,
    };
    assert_eq!(
        run_oracle(&script),
        vec![0xA5, 0xA5, 0xA5],
        "the deleted model repeated the byte for ever"
    );
    assert_eq!(
        run_descriptor(&mut bus, &script),
        vec![0xA5, 0xFF, 0xFF],
        "the descriptor clocks the word once, then open bus"
    );
}

/// …and the shape every shipped manifest actually uses — CS never driven —
/// re-samples on every byte, exactly as the model did.
#[test]
fn without_cs_callbacks_every_byte_re_samples_the_live_inputs() {
    let mut bus = rig(&[("cs_pin", s("PA4"))]);
    let script = Script {
        stimulus: stimulus_for(0xA5),
        clocks: 3,
        hold_cs: false,
    };
    assert_eq!(run_descriptor(&mut bus, &script), run_oracle(&script));
    assert_eq!(run_oracle(&script), vec![0xA5, 0xA5, 0xA5]);
}
