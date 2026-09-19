// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The six analog plants, against the hand-written models they replace.**
//!
//! `components/{ldr, potentiometer, ntc_thermistor, mq6, soil_moisture,
//! lipo_charger}.rs` are DELETED — 1455 lines that were the same file six
//! times: a struct holding one or two `f32` slots, a `SimInput` impl that
//! stored them, an `AnalogSource` impl that ran two lines of algebra, and a
//! `KitMetadata` literal. The algebra is datasheet language, so the algebra is
//! what the descriptor carries.
//!
//! Each `fn *_model` below is the deleted model's own arithmetic, transcribed
//! **verbatim** from the file that was removed — the same `f32` operations in
//! the same order, including every `clamp` and every `as u16`. It is the
//! ORACLE, and the point of this file is that it is not the thing under test:
//! the descriptor is, and the two are swept against each other across the whole
//! declared channel range.
//!
//! ## What the sweeps assert, and what they measured
//!
//! Each sweep compares the descriptor against TWO oracles at every sample: the
//! deleted model's own `f32` arithmetic, and the exact value of the line or
//! equation the datasheet states, in `f64`. The second one is the arbiter
//! wherever the first disagrees — it says WHICH side drifted.
//!
//! Measured on this branch (`cargo test -p labwired-core --test
//! analog_plant_migration_parity -- --nocapture`):
//!
//! | part | samples | max Δ vs the deleted model | samples that differed | max Δ vs exact |
//! |---|---|---|---|---|
//! | `ldr` | 20 001 over 0..100 000 lx | 1 mV | 2 | **0 mV** |
//! | `ldr` | 20 001 over 0..100 lx (the steep end) | 0 mV | 0 | **0 mV** |
//! | `potentiometer` | 10 001 over 0..100 % | 1 mV | 3 | **0 mV** |
//! | `ntc-thermistor` | 20 501 over -55..150 °C | 1 mV | 3 | **0 mV** |
//! | `mq-6` | 20 001 over 0..10 000 ppm | 1 mV | 7 | **0 mV** |
//! | `soil-moisture` | 10 001 over 0..100 % | 1 mV | 23 | **0 mV** |
//! | `lipo_charger` | 20 002 (both charger states) | **0 mV** | 0 | — |
//!
//! **The descriptor is exact everywhere and the deleted models were not.** Each
//! handful of disagreements is a point where the true answer is a whole
//! millivolt and an `f32` subtraction landed a few parts in 10^8 below it: the
//! soil probe's 660.0 mV at 80 % arrived as 659.99996 and truncated to 659. A
//! port that reproduced those would be copying an artefact of float ordering
//! into data anyone can check by hand, so it does not. One millivolt is 1.24
//! ADC counts at 3.3 V — the converter resolves 0.806 mV, so no firmware can
//! tell the two apart by more than its own quantiser.
//!
//! ## Deliberate differences, named
//!
//! * **`potentiometer` above 100 % / below 0 %.** The deleted model clamped the
//!   OUTPUT to `0..Vref`; the curve clamps the INPUT to its first and last
//!   points. Both report 0 mV and 3300 mV at the rails, and the stimulus
//!   channel's declared range (0..100 %) is enforced by `require_channel`
//!   before either can be reached, so no caller can tell them apart.
//! * **`lipo_charger`'s two truncations become one.** The model computed
//!   `terminal.clamp(3300, 4200) as u16` and then a u16 `/ 2`. The descriptor
//!   states `formula: "terminal_mv / 2"` with `encode: trunc`. For any
//!   non-negative real `x`, `floor(floor(x) / 2) == floor(x / 2)`, so the two
//!   are the same function — [`lipo_charger_matches_the_model`] sweeps every
//!   0.01 % of SoC in both charger states rather than leaving that as a claim.
//! * **Nothing else changed.** None of the six declared noise, bias or lag, so
//!   none of the descriptors does either — there is no behaviour dropped
//!   silently here, and [`no_analog_plant_silently_gained_or_lost_a_channel`]
//!   pins the channel tables that used to be `const INPUT_CHANNELS`.

use labwired_core::peripherals::components::declarative_analog::{
    DeclarativeAnalogDevice, DeclarativeAnalogKit,
};
use labwired_core::sim_input::SimInput;

fn kit(device_type: &str) -> DeclarativeAnalogKit {
    let yaml = labwired_config::embedded_device_yaml(device_type)
        .unwrap_or_else(|| panic!("{device_type} descriptor is not embedded"));
    DeclarativeAnalogKit::from_yaml(yaml).unwrap_or_else(|e| panic!("{device_type}.yaml: {e}"))
}

fn device(device_type: &str) -> DeclarativeAnalogDevice {
    kit(device_type).build(0).expect("device builds")
}

/// 12-bit count for 3.3 V Vref — the conversion every deleted model carried and
/// the engine now owns once.
fn count(mv: u16) -> u16 {
    ((u32::from(mv) * 4095) / 3300).min(4095) as u16
}

/// Sweep one channel over `steps + 1` points spanning `lo..=hi`, comparing the
/// descriptor against BOTH oracles: the deleted model's own `f32` arithmetic,
/// and the exact value of the line/equation the datasheet states, in `f64`.
///
/// Returns `(worst |Δ| vs the model, how many samples differed, worst |Δ| vs
/// exact)`. Both deltas are asserted at 1 mV, which is 1.24 ADC counts at
/// 3.3 V — the ADC cannot resolve better than 0.806 mV, so a disagreement of
/// one millivolt is one count either way and never two readings a firmware
/// could tell apart by more than its own quantiser.
fn sweep(
    device_type: &str,
    key: &str,
    lo: f64,
    hi: f64,
    steps: u32,
    model: impl Fn(f64) -> u16,
    exact: impl Fn(f64) -> f64,
) -> (u32, u32, u32) {
    let mut dev = device(device_type);
    let (mut worst, mut differed, mut worst_exact) = (0u32, 0u32, 0u32);
    let mut worst_at = lo;
    for i in 0..=steps {
        let x = lo + (hi - lo) * (f64::from(i) / f64::from(steps));
        dev.set_input(key, x)
            .unwrap_or_else(|e| panic!("{device_type}.{key} = {x}: {e:?}"));
        let got = dev.output_mv();

        let want = model(x);
        let delta = u32::from(got.abs_diff(want));
        if delta > 0 {
            differed += 1;
        }
        if delta > worst {
            worst = delta;
            worst_at = x;
        }
        assert!(
            delta <= 1,
            "{device_type} at {key} = {x}: descriptor {got} mV, deleted model {want} mV \
             ({} ADC counts apart)",
            count(got).abs_diff(count(want))
        );

        // The exact value is the arbiter wherever the two disagree: it says
        // WHICH of them drifted, which is the difference between a port that
        // changed the part and one that stopped copying a float artefact.
        let truth = exact(x).clamp(0.0, 3300.0) as u16;
        let delta_exact = u32::from(got.abs_diff(truth));
        worst_exact = worst_exact.max(delta_exact);
        assert!(
            delta_exact <= 1,
            "{device_type} at {key} = {x}: descriptor {got} mV, exact {:.6} mV",
            exact(x)
        );
    }
    println!(
        "  {device_type:<16} {key:<12} {} samples | vs deleted model: max {worst} mV, \
         {differed} differed | vs exact: max {worst_exact} mV (worst model Δ at {worst_at})",
        steps + 1
    );
    (worst, differed, worst_exact)
}

// ─── the oracles: the deleted models' own arithmetic ───────────────────────

/// `ldr.rs::divider_output_mv`. R(lux) = 10k * (lux/10)^-0.7 as the top leg of
/// a 10 kΩ pull-down divider.
fn ldr_model(lux: f64) -> u16 {
    let lux = lux as f32;
    let ratio = (lux / 10.0).max(0.0);
    let r_ldr = 10_000.0f32 * ratio.powf(-0.7);
    let v_out = 3300.0f32 * 10_000.0 / (r_ldr + 10_000.0);
    v_out.clamp(0.0, 3300.0) as u16
}

/// `potentiometer.rs::wiper_output_mv`.
fn potentiometer_model(position_pct: f64) -> u16 {
    (3300.0f32 * position_pct as f32 / 100.0).clamp(0.0, 3300.0) as u16
}

/// `ntc_thermistor.rs::divider_output_mv`. Beta equation, R0 = 10 kΩ at
/// 298.15 K, B = 3950, into a 10 kΩ pull-down.
fn ntc_model(temperature_c: f64) -> u16 {
    let t_k = temperature_c as f32 + 273.15;
    let exponent = 3950.0f32 * (1.0 / t_k - 1.0 / 298.15);
    let r_ntc = 10_000.0f32 * exponent.exp();
    let v_out = 3300.0f32 * 10_000.0 / (r_ntc + 10_000.0);
    v_out.clamp(0.0, 3300.0) as u16
}

/// `mq6.rs::aout_mv`.
fn mq6_model(ppm: f64) -> u16 {
    let ppm = (ppm as f32).max(0.0);
    let fraction = (ppm / 10_000.0).clamp(0.0, 1.0);
    (3300.0f32 * fraction) as u16
}

/// `soil_moisture.rs::aout_mv`.
fn soil_model(moisture_pct: f64) -> u16 {
    let moisture = (moisture_pct as f32).clamp(0.0, 100.0);
    let dry_fraction = 1.0 - (moisture / 100.0).clamp(0.0, 1.0);
    (3300.0f32 * dry_fraction) as u16
}

/// `lipo_charger.rs::battery_mv` followed by `adc_pin_mv` — BOTH truncations,
/// in the order the model performed them.
fn lipo_model(soc_pct: f64, usb_present: bool) -> u16 {
    let soc = (soc_pct as f32 / 100.0).clamp(0.0, 1.0);
    let mut mv = 3300.0f32 + (4200.0 - 3300.0) * soc;
    if usb_present {
        mv += 150.0;
    }
    let battery_mv = mv.clamp(3300.0, 4200.0) as u16;
    battery_mv / 2
}

// ─── the exact oracles: the datasheet's own algebra, in f64 ────────────────
//
// Written independently of the descriptor (the datasheet's spelling, not the
// descriptor's) so "the descriptor is the line" is a claim two implementations
// agree on rather than one restated.

fn ldr_exact(lux: f64) -> f64 {
    let r = 10_000.0 * (lux.max(0.0) / 10.0).powf(-0.7);
    3300.0 * 10_000.0 / (r + 10_000.0)
}

fn potentiometer_exact(position_pct: f64) -> f64 {
    3300.0 * position_pct / 100.0
}

fn ntc_exact(temperature_c: f64) -> f64 {
    let r = 10_000.0 * (3950.0 * (1.0 / (temperature_c + 273.15) - 1.0 / 298.15)).exp();
    3300.0 * 10_000.0 / (r + 10_000.0)
}

fn mq6_exact(ppm: f64) -> f64 {
    3300.0 * ppm.clamp(0.0, 10_000.0) / 10_000.0
}

fn soil_exact(moisture_pct: f64) -> f64 {
    3300.0 - 3300.0 * moisture_pct.clamp(0.0, 100.0) / 100.0
}

// ─── the sweeps ────────────────────────────────────────────────────────────

#[test]
fn ldr_matches_the_model_across_its_whole_lux_range() {
    println!("analog plant parity:");
    // 0..100 000 lx is the declared channel range, and a second pass over the
    // first 100 lx where the power law is nearly vertical — the region a
    // `curve:` could never have covered at any sane point count.
    let (worst, _, exact) = sweep("ldr", "lux", 0.0, 100_000.0, 20_000, ldr_model, ldr_exact);
    let (steep, _, steep_exact) = sweep("ldr", "lux", 0.0, 100.0, 20_000, ldr_model, ldr_exact);
    assert!(
        worst <= 1 && steep <= 1,
        "the f32/f64 gap against the deleted model widened: {worst} mV / {steep} mV"
    );
    assert!(
        exact <= 1 && steep_exact <= 1,
        "the descriptor drifted from the equation itself: {exact} mV / {steep_exact} mV"
    );
}

#[test]
fn potentiometer_matches_the_model_across_its_whole_travel() {
    println!("analog plant parity:");
    // 10 001 samples at 0.01 % — finer than any physical pot resolves.
    let (worst, _, exact) = sweep(
        "potentiometer",
        "position",
        0.0,
        100.0,
        10_000,
        potentiometer_model,
        potentiometer_exact,
    );
    assert!(worst <= 1, "vs the deleted model: {worst} mV");
    assert_eq!(
        exact, 0,
        "a two-point curve over a straight line IS the line — every sample must be exact"
    );
}

#[test]
fn ntc_thermistor_matches_the_model_across_its_whole_temperature_range() {
    println!("analog plant parity:");
    // -55..150 °C, 20 501 samples at 0.01 °C.
    let (worst, _, exact) = sweep(
        "ntc-thermistor",
        "temperature",
        -55.0,
        150.0,
        20_500,
        ntc_model,
        ntc_exact,
    );
    assert!(worst <= 1, "vs the deleted model: {worst} mV");
    assert!(exact <= 1, "vs the beta equation itself: {exact} mV");
}

#[test]
fn mq6_matches_the_model_across_its_whole_detection_band() {
    println!("analog plant parity:");
    let (worst, _, exact) = sweep("mq-6", "ppm", 0.0, 10_000.0, 20_000, mq6_model, mq6_exact);
    assert!(worst <= 1, "vs the deleted model: {worst} mV");
    assert_eq!(exact, 0, "a straight line, exactly");
}

#[test]
fn soil_moisture_matches_the_model_across_its_whole_range() {
    println!("analog plant parity:");
    let (worst, _, exact) = sweep(
        "soil-moisture",
        "moisture",
        0.0,
        100.0,
        10_000,
        soil_model,
        soil_exact,
    );
    assert!(worst <= 1, "vs the deleted model: {worst} mV");
    assert_eq!(exact, 0, "a straight line, exactly");
}

/// The two-channel part. Every 0.01 % of SoC in BOTH charger states, which is
/// also the proof that one `encode: trunc` reproduces the model's two integer
/// truncations.
#[test]
fn lipo_charger_matches_the_model() {
    println!("analog plant parity:");
    let mut dev = device("lipo_charger");
    let mut worst = 0u16;
    for usb in [false, true] {
        dev.set_input("usb_present", if usb { 1.0 } else { 0.0 })
            .expect("usb_present in range");
        for i in 0..=10_000u32 {
            let soc = f64::from(i) / 100.0;
            dev.set_input("soc_pct", soc).expect("soc in range");
            let (got, want) = (dev.output_mv(), lipo_model(soc, usb));
            assert_eq!(
                got, want,
                "lipo_charger at soc {soc} %, usb {usb}: descriptor {got} mV, model {want} mV"
            );
            worst = worst.max(got.abs_diff(want));
        }
    }
    println!("  lipo_charger     max |Δ| = {worst} mV over 20 002 samples (both charger states)");
    assert_eq!(worst, 0);

    // The named landmarks the deleted model's own unit tests asserted, kept so
    // a sweep that silently stopped sweeping still has something to fail on.
    for (soc, usb, pin_mv) in [
        (0.0, false, 1650u16),
        (50.0, false, 1875),
        (100.0, false, 2100),
        (50.0, true, 1950),  // 3750 + 150 = 3900, halved
        (95.0, true, 2100),  // 4155 + 150 clamps to 4200, halved
        (100.0, true, 2100), // already at the ceiling
    ] {
        dev.set_input("usb_present", if usb { 1.0 } else { 0.0 })
            .unwrap();
        dev.set_input("soc_pct", soc).unwrap();
        assert_eq!(dev.output_mv(), pin_mv, "soc {soc} %, usb {usb}");
    }
}

/// `usb_present` is a boolean carried on a float channel, and the model's test
/// for "connected" was `value >= 0.5`. The descriptor's `when:` guard states
/// the same threshold; this pins the exact step, which is the one place a
/// descriptor could have silently moved it.
#[test]
fn the_charger_threshold_is_the_models_half_way_step() {
    let mut dev = device("lipo_charger");
    dev.set_input("soc_pct", 0.0).unwrap();
    for (usb, want) in [(0.0, 1650u16), (0.4999, 1650), (0.5, 1725), (1.0, 1725)] {
        dev.set_input("usb_present", usb).unwrap();
        assert_eq!(dev.output_mv(), want, "usb_present = {usb}");
        assert_eq!(lipo_model(0.0, usb >= 0.5), want, "the model agrees");
    }
}

// ─── the channel tables, which were `const INPUT_CHANNELS` ─────────────────

/// Every deleted model carried a `pub const INPUT_CHANNELS` that backed BOTH
/// its `SimInput` impl and its `KitMetadata`. Those tables are now
/// `metadata.inputs` in the descriptor, and this is the one place they are
/// compared against what the Rust said — key, label, unit and range. A channel
/// whose key drifted would leave `set_input("lux", …)` failing at runtime for
/// every lab that drives it, with nothing else red.
#[test]
fn no_analog_plant_silently_gained_or_lost_a_channel() {
    /// key, label, unit, min, max — the five fields a deleted model's
    /// `InputChannel` literal carried.
    type Channel = (&'static str, &'static str, &'static str, f64, f64);

    let expected: &[(&str, &[Channel])] = &[
        ("ldr", &[("lux", "Illuminance", "lx", 0.0, 100_000.0)]),
        (
            "potentiometer",
            &[("position", "Position", "%", 0.0, 100.0)],
        ),
        (
            "ntc-thermistor",
            &[("temperature", "Temperature", "°C", -55.0, 150.0)],
        ),
        (
            "mq-6",
            &[("ppm", "LPG concentration", "ppm", 0.0, 10_000.0)],
        ),
        (
            "soil-moisture",
            &[("moisture", "Soil moisture", "%", 0.0, 100.0)],
        ),
        (
            "lipo_charger",
            &[
                ("soc_pct", "State of charge", "%", 0.0, 100.0),
                ("usb_present", "Charger connected", "", 0.0, 1.0),
            ],
        ),
    ];
    for (device_type, channels) in expected {
        let kit = kit(device_type);
        let got = &labwired_core::peripherals::kit::PeripheralKit::metadata(&kit).inputs;
        assert_eq!(
            got.len(),
            channels.len(),
            "{device_type} channel count: {got:?}"
        );
        for (got, (key, label, unit, min, max)) in got.iter().zip(channels.iter()) {
            assert_eq!(got.key, *key, "{device_type}");
            assert_eq!(got.label, *label, "{device_type}.{key}");
            assert_eq!(got.unit, *unit, "{device_type}.{key}");
            assert_eq!(got.min, *min, "{device_type}.{key} min");
            assert_eq!(got.max, *max, "{device_type}.{key} max");
        }
    }
}

/// The out-of-range rejection each model inherited from `require_channel`, kept
/// because the LiPo charger's own unit test asserted it: a host that drives
/// 150 % SoC gets an error, not a clamped reading it cannot see.
#[test]
fn an_out_of_range_stimulus_is_still_refused() {
    let mut dev = device("lipo_charger");
    assert!(dev.set_input("soc_pct", 150.0).is_err());
    assert!(dev.set_input("usb_present", 5.0).is_err());
    assert!(dev.set_input("bogus", 1.0).is_err());
    let mut ldr = device("ldr");
    assert!(ldr.set_input("lux", 200_000.0).is_err());
    assert!(ldr.set_input("lux", -1.0).is_err());
}

/// The rails each deleted model's own tests pinned, in one place. These are the
/// sentences from the model headers — "dark reads 0", "bright reads toward
/// Vref", "clean air is the ground rail", "bone-dry is Vref" — and a sweep that
/// agreed with a broken oracle would still have to pass these.
#[test]
fn the_rails_each_model_documented_still_hold() {
    let mut ldr = device("ldr");
    ldr.set_input("lux", 0.0).unwrap();
    assert_eq!(ldr.output_mv(), 0, "pitch dark is the ground rail");
    ldr.set_input("lux", 100_000.0).unwrap();
    assert!(
        ldr.output_mv() > 3000 && ldr.output_mv() < 3300,
        "bright sunlight approaches Vref without reaching it, got {}",
        ldr.output_mv()
    );

    let mut pot = device("potentiometer");
    pot.set_input("position", 0.0).unwrap();
    assert_eq!(pot.output_mv(), 0);
    pot.set_input("position", 100.0).unwrap();
    assert_eq!(pot.output_mv(), 3300);
    assert_eq!(pot.adc_count(), 4095);

    let mut ntc = device("ntc-thermistor");
    ntc.set_input("temperature", 25.0).unwrap();
    assert_eq!(
        ntc.output_mv(),
        1650,
        "R_ntc = R0 at 25 °C, so exactly Vref/2"
    );
    ntc.set_input("temperature", 80.0).unwrap();
    let hot = ntc.output_mv();
    ntc.set_input("temperature", -10.0).unwrap();
    assert!(hot > ntc.output_mv(), "a hotter NTC reads higher");

    let mut mq6 = device("mq-6");
    mq6.set_input("ppm", 0.0).unwrap();
    assert_eq!(mq6.output_mv(), 0, "clean air is the ground rail");
    mq6.set_input("ppm", 10_000.0).unwrap();
    assert_eq!(mq6.output_mv(), 3300);
    assert_eq!(mq6.adc_count(), 4095);

    let mut soil = device("soil-moisture");
    soil.set_input("moisture", 0.0).unwrap();
    assert_eq!(soil.output_mv(), 3300, "bone-dry is Vref");
    assert_eq!(soil.adc_count(), 4095);
    soil.set_input("moisture", 100.0).unwrap();
    assert_eq!(soil.output_mv(), 0, "saturated is the ground rail");
}
