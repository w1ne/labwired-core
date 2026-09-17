// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Engine tests for the `led_strip` primitive: the descriptors it ships must
//! load, and every key that decides where a colour goes must be provably wired
//! to the decoder. Each negative control SABOTAGES one key in the shipped YAML
//! and asserts the colours MOVE — a validation error alone would prove only
//! that the engine reads the key, not that it acts on it.

use super::*;

const SHIPPED: &[&str] = &["apa102", "neopixel"];

#[test]
fn every_shipped_led_strip_descriptor_loads() {
    for t in SHIPPED {
        embedded(t).unwrap_or_else(|e| panic!("{t}: {e}"));
    }
}

#[test]
fn every_shipped_led_strip_registers_a_kit_with_the_right_transport() {
    for t in SHIPPED {
        let kit = DeclarativeLedStripKit::from_yaml(
            labwired_config::embedded_device_yaml(t).expect("embedded"),
        )
        .unwrap_or_else(|e| panic!("{t}: {e}"));
        let m = kit.metadata();
        let expect = match kit.spec().wire {
            LedStripWire::SpiFrames => Transport::Spi,
            LedStripWire::NrzGpio => Transport::GpioGroup,
        };
        assert_eq!(m.transport, expect, "{t}: transport");
    }
}

// ─── APA102: the clocked wire ──────────────────────────────────────────────

/// One start frame plus two full-brightness LED frames (red, then green),
/// latched on CS release exactly as a driver sends them.
fn drive_two_pixels(strip: &mut GenericLedStrip) {
    SpiDevice::cs_select(strip);
    for b in [0, 0, 0, 0] {
        SpiDevice::transfer(strip, b);
    }
    for b in [0xFF, 0x00, 0x00, 0xFF] {
        SpiDevice::transfer(strip, b); // brightness 0x1F, B=0, G=0, R=0xFF
    }
    for b in [0xFF, 0x00, 0xFF, 0x00] {
        SpiDevice::transfer(strip, b); // B=0, G=0xFF, R=0
    }
    SpiDevice::cs_release(strip);
}

#[test]
fn an_apa102_latches_its_frames_in_red_green_blue_order() {
    let mut strip = apa102("PA4", 8);
    drive_two_pixels(&mut strip);
    assert_eq!(
        strip.pixels(),
        vec![
            LedPixel {
                wire: [0xFF, 0, 0],
                brightness: 0x1F
            },
            LedPixel {
                wire: [0, 0xFF, 0],
                brightness: 0x1F
            },
        ],
        "red then green at full brightness"
    );
}

/// THE NEGATIVE CONTROL FOR `colour_bytes`. The APA102 sends B, G, R inside a
/// frame and the artifact publishes R, G, B. Transposing the map must move the
/// colours; if this passes with the map reversed, the key is decoration.
#[test]
fn transposing_colour_bytes_moves_the_colours() {
    let yaml = labwired_config::embedded_device_yaml("apa102").expect("embedded");
    let flipped = yaml.replace("colour_bytes: [3, 2, 1]", "colour_bytes: [1, 2, 3]");
    assert_ne!(flipped, yaml, "the sabotage did not apply");

    let first = |desc: &str| -> [u8; 3] {
        let mut s = GenericLedStrip::from_yaml(desc).expect("descriptor builds");
        drive_two_pixels(&mut s);
        s.pixels()[0].wire
    };
    assert_eq!(first(yaml), [0xFF, 0, 0], "as shipped: red");
    assert_eq!(
        first(&flipped),
        [0, 0, 0xFF],
        "transposed: the same wire bytes publish as blue"
    );
}

/// THE NEGATIVE CONTROL FOR `header_mask`, and the one place the marker's real
/// reach is written down.
///
/// APA102 §"End frame": the end frame is 32 clocks whose level the datasheet
/// leaves to the driver. Adafruit_DotStar clocks ZEROS, FastLED clocks ONES,
/// and the marker only rejects one of them:
///
/// * `0x00` fails `byte & 0xE0 == 0xE0` and STOPS the decode — that is the
///   case this asserts, and removing the marker adds a phantom black LED;
/// * `0xFF` PASSES it (0xFF & 0xE0 == 0xE0), so a ones end frame decodes as one
///   more white LED and the only thing bounding the strip is `num_pixels`.
///
/// The second half is not a bug introduced by the port: the deleted Rust model
/// tested the identical `header & 0xE0 != 0xE0`, and
/// `led_strip_migration_parity.rs` pins both implementations to it. It is
/// recorded here so nobody reads the descriptor's marker as "the end frame
/// always stops the decode", which is what it looks like and is not.
#[test]
fn a_header_marker_that_accepts_the_end_frame_invents_an_led() {
    let yaml = labwired_config::embedded_device_yaml("apa102").expect("embedded");
    // Sabotage: move the marker onto the end frame's value. `header_mask` stays
    // 0xE0, so the accepted bytes become those with the top three bits CLEAR —
    // the zeros end frame, and no real LED header.
    let moved = yaml.replace("header_value: 0xE0", "header_value: 0x00");
    assert_ne!(moved, yaml, "the sabotage did not apply");

    let count = |desc: &str, end: u8| -> usize {
        let mut s = GenericLedStrip::from_yaml(desc).expect("descriptor builds");
        SpiDevice::cs_select(&mut s);
        for b in [0, 0, 0, 0] {
            SpiDevice::transfer(&mut s, b);
        }
        for b in [0xFF, 0x00, 0x00, 0xFF] {
            SpiDevice::transfer(&mut s, b);
        }
        // The end frame the driver clocks out after the last LED.
        for _ in 0..4 {
            SpiDevice::transfer(&mut s, end);
        }
        SpiDevice::cs_release(&mut s);
        s.pixels().len()
    };
    assert_eq!(
        count(yaml, 0x00),
        1,
        "as shipped: an Adafruit_DotStar ZEROS end frame stops the decode"
    );
    assert_eq!(
        count(&moved, 0x00),
        0,
        "with the marker moved onto the end frame, every real LED frame is \
         rejected and the strip goes dark"
    );
    assert_eq!(
        count(yaml, 0xFF),
        2,
        "as shipped and as the deleted model did: a FastLED ONES end frame \
         passes the marker and decodes as one more white LED — `num_pixels` \
         is the bound"
    );

    // And "no marker at all" — the shape a descriptor without the concept
    // would have — is refused at load, naming what it would have done.
    let none = yaml.replace("header_mask: 0xE0", "header_mask: 0x00");
    let err = GenericLedStrip::from_yaml(&none)
        .expect_err("a marker that matches every byte must be refused")
        .to_string();
    assert!(
        err.contains("header_mask is 0"),
        "the refusal must name the key: {err}"
    );
}

/// A transaction that does not open with the start frame latches NOTHING, so a
/// glitchy transfer cannot blank a strip that is already showing a picture.
#[test]
fn a_transaction_without_the_start_frame_leaves_the_previous_colours() {
    let mut strip = apa102("PA4", 8);
    drive_two_pixels(&mut strip);
    let before = strip.pixels();
    SpiDevice::cs_select(&mut strip);
    for b in [0xAAu8, 0xBB, 0xCC, 0xDD, 0xEE] {
        SpiDevice::transfer(&mut strip, b);
    }
    SpiDevice::cs_release(&mut strip);
    assert_eq!(
        strip.pixels(),
        before,
        "a malformed transfer changes nothing"
    );
}

/// ⚠️ THE SUPPLY GATE. An APA102 draws every milliamp from the rail, so a
/// signals-only diagram is completely dark on a bench. Both halves: the
/// positive control, without which "unpowered is dark" would also pass on a
/// model that never latches anything.
#[test]
fn an_unpowered_apa102_latches_nothing_and_says_why() {
    let mut lit = apa102("PA4", 8);
    drive_two_pixels(&mut lit);
    assert_eq!(lit.pixels().len(), 2, "powered: the frames latch");
    assert!(lit.powered(), "no supply config at all must mean powered");

    let mut dark = apa102("PA4", 8);
    dark.set_powered(false);
    for _ in 0..5 {
        drive_two_pixels(&mut dark);
    }
    assert!(dark.pixels().is_empty(), "no supply, no light");
    let m = SpiDevice::artifacts(&dark, "strip", &crate::inspect::InspectOpts::default())
        .into_iter()
        .next()
        .expect("one artifact")
        .meta;
    assert_eq!(m["w"], 0, "an unlit strip reports zero LEDs wide");
    assert_eq!(m["brightness"], serde_json::json!([]));
    assert_eq!(m["powered"], false, "the artifact must say WHY it is dark");
}

// ─── WS2812: the single wire ───────────────────────────────────────────────

/// Clock one 24-bit GRB word onto the pad at `cpu_hz`, with the datasheet's bit
/// lengths. `t` advances by whole bit times so the decode reads real durations.
fn nrz_word(strip: &GenericLedStrip, t: &mut u64, cycles_per_us: u64, word: u32) {
    use crate::peripherals::device::GpioObserver;
    for i in (0..24).rev() {
        let high = if (word >> i) & 1 == 1 {
            (cycles_per_us * 7) / 10 // T1H ~0.7 us
        } else {
            (cycles_per_us * 35) / 100 // T0H ~0.35 us
        };
        strip.on_pin_change(strip.data_pin(), false, true, *t);
        *t += high;
        strip.on_pin_change(strip.data_pin(), true, false, *t);
        *t += (cycles_per_us * 125) / 100 - high; // pad to ~1.25 us
    }
}

const CPU_HZ: u64 = 160_000_000;
const CYCLES_PER_US: u64 = CPU_HZ / 1_000_000;

#[test]
fn a_ws2812_decodes_edge_timing_into_grb_words() {
    use crate::peripherals::device::GpioObserver;
    let strip = ws2812(48, 3, CPU_HZ);
    let mut t = 1_000u64;
    for w in [0x00FF00u32, 0xFF0000, 0x0000FF] {
        nrz_word(&strip, &mut t, CYCLES_PER_US, w);
    }
    // The reset gap: hold low well past 40 us, then one more rising edge to
    // close the frame the way the next playback would.
    t += CYCLES_PER_US * 60;
    strip.on_pin_change(48, false, true, t);
    assert_eq!(
        strip.pixels(),
        vec![
            LedPixel {
                wire: [0x00, 0xFF, 0x00],
                brightness: 0
            },
            LedPixel {
                wire: [0xFF, 0x00, 0x00],
                brightness: 0
            },
            LedPixel {
                wire: [0x00, 0x00, 0xFF],
                brightness: 0
            },
        ],
        "three GRB words, in wire order"
    );
}

/// THE NEGATIVE CONTROL FOR `high_threshold_ns`. Raising it above T1H must
/// decode every long pulse as a zero — which is a black strip for firmware that
/// is driving it correctly. If this passes with the threshold moved, the
/// timing table is not reaching the decoder.
#[test]
fn moving_the_high_threshold_changes_what_a_bit_decodes_as() {
    use crate::peripherals::device::GpioObserver;
    let yaml = labwired_config::embedded_device_yaml("neopixel").expect("embedded");
    let blind = yaml.replace("high_threshold_ns: 500", "high_threshold_ns: 900");
    assert_ne!(blind, yaml, "the sabotage did not apply");

    let decode = |desc: &str| -> Vec<LedPixel> {
        let mut s = GenericLedStrip::from_yaml(desc).expect("descriptor builds");
        s.set_data_pin(48);
        s.set_num_pixels(1);
        s.set_cpu_hz(CPU_HZ);
        let mut t = 1_000u64;
        nrz_word(&s, &mut t, CYCLES_PER_US, 0x00FF00);
        t += CYCLES_PER_US * 60;
        s.on_pin_change(48, false, true, t);
        s.pixels()
    };
    assert_eq!(
        decode(yaml)[0].wire,
        [0x00, 0xFF, 0x00],
        "as shipped: green"
    );
    assert_eq!(
        decode(&blind)[0].wire,
        [0, 0, 0],
        "a threshold above T1H reads every bit as zero — a black strip"
    );
}

/// A strip that never saw an edge reports zero decoded LEDs, not a plausible
/// pattern.
#[test]
fn a_ws2812_that_saw_no_edges_decodes_nothing() {
    let strip = ws2812(48, 8, CPU_HZ);
    assert!(strip.pixels().is_empty());
    let m = crate::inspect::DeviceEvidence::artifacts(
        &strip,
        "strip",
        &crate::inspect::InspectOpts::default(),
    )
    .into_iter()
    .next()
    .expect("one artifact")
    .meta;
    assert_eq!(m["pixels_decoded"], 0);
    assert_eq!(m["lit_pixels"], 0);
    assert_eq!(
        m["w"], 8,
        "a single-wire strip reports its configured length"
    );
}

// ─── descriptor validation ─────────────────────────────────────────────────

#[test]
fn a_wire_carrying_the_other_wires_block_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("neopixel").expect("embedded");
    let broken = yaml.replace("wire: nrz_gpio", "wire: spi_frames");
    assert_ne!(broken, yaml, "the sabotage did not apply");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(format!("{err:#}").contains("timing"), "got: {err:#}");
}

#[test]
fn a_reset_threshold_below_the_bit_threshold_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("neopixel").expect("embedded");
    let broken = yaml.replace("reset_threshold_ns: 40000", "reset_threshold_ns: 400");
    assert_ne!(broken, yaml, "the sabotage did not apply");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(
        format!("{err:#}").contains("inter-bit low would latch"),
        "got: {err:#}"
    );
}

#[test]
fn an_empty_start_frame_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("apa102").expect("embedded");
    let broken = yaml.replace("start_frame: [0x00, 0x00, 0x00, 0x00]", "start_frame: []");
    assert_ne!(broken, yaml, "the sabotage did not apply");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(format!("{err:#}").contains("start_frame"), "got: {err:#}");
}

#[test]
fn a_brightness_mask_overlapping_the_header_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("apa102").expect("embedded");
    let broken = yaml.replace("brightness_mask: 0x1F", "brightness_mask: 0x3F");
    assert_ne!(broken, yaml, "the sabotage did not apply");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(format!("{err:#}").contains("overlaps"), "got: {err:#}");
}

/// `powered` on a strip that is not `supply_gated` would be a constant `true`
/// dressed as a measurement — which is exactly what the WS2812 must NOT
/// publish, because it has never had a supply gate.
#[test]
fn a_powered_meta_key_without_a_supply_gate_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("neopixel").expect("embedded");
    let broken = yaml.replace(
        "artifact_meta: [pixels_decoded,",
        "artifact_meta: [powered, pixels_decoded,",
    );
    assert_ne!(broken, yaml, "the sabotage did not apply");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(format!("{err:#}").contains("supply_gated"), "got: {err:#}");
}

/// A clocked-SPI fact on a single-wire strip, and the reverse. The two wires
/// publish different keys and neither can borrow the other's.
#[test]
fn a_meta_key_the_wire_cannot_carry_is_refused() {
    let yaml = labwired_config::embedded_device_yaml("neopixel").expect("embedded");
    let broken = yaml.replace("artifact_meta: [pixels_decoded,", "artifact_meta: [cs_pin,");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(
        format!("{err:#}").contains("clocked-SPI fact"),
        "got: {err:#}"
    );

    let yaml = labwired_config::embedded_device_yaml("apa102").expect("embedded");
    let broken = yaml.replace("artifact_meta: [brightness,", "artifact_meta: [data_pin,");
    let err = GenericLedStrip::from_yaml(&broken).expect_err("must be refused");
    assert!(
        format!("{err:#}").contains("single-wire fact"),
        "got: {err:#}"
    );
}
