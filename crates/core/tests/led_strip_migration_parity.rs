// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **APA102 and WS2812: the YAML descriptors against the Rust models they
//! replace.**
//!
//! `components/apa102.rs` and `components/ws2812.rs` were deleted when the
//! `led_strip` primitive landed. This file is the evidence that nothing moved.
//! The template is `display_migration_parity.rs`, and the rule is the same: the
//! deleted models live on VERBATIM in `led_strip_oracle/`, both implementations
//! are driven by the SAME script, and three things are compared rather than one:
//!
//!   * for the clocked strip, the WIRE TRANSCRIPT — every MISO byte the device
//!     put back on the bus (an APA102 drives none, and "drives none" is itself a
//!     claim worth pinning);
//!   * the WHOLE LED COLOUR ARRAY, LED by LED, including the brightness field —
//!     not a count and not a checksum;
//!   * every field of the paint artifact's `meta`, which is what the browser's
//!     strip overlay and the CLI's `painted bytes=` line read.
//!
//! The two wires are driven differently BECAUSE THEY ARE DIFFERENT WIRES, and
//! that is the whole reason the primitive has two doors:
//!
//!   * APA102 is a [`SpiDevice`], so `common/transcript.rs` drives it with the
//!     same `Step` script every other byte-parity proof uses;
//!   * WS2812 is a `GpioObserver` on one pad, so there is no bus script to
//!     write. [`feed_edges`] replays ONE list of `(level, sim_cycle)`
//!     transitions into both implementations, which is the same idea: one
//!     stimulus, two models, compare everything.
//!
//! ── NO DELIBERATE DIFFERENCES ───────────────────────────────────────────────
//!
//! Unlike the display ports, neither strip changed behaviour. Every assertion
//! below is `old == new`. Two facts that LOOK like bugs are pinned as shared
//! behaviour instead, so a later "fix" to one implementation fails here:
//!
//!   1. **An APA102 ones end frame decodes as one more white LED.** The end
//!      frame is 32 clocks whose level the APA102 datasheet leaves to the
//!      driver. `0xFF & 0xE0 == 0xE0` passes the header marker, so FastLED's
//!      ones end frame is indistinguishable from a full-brightness white LED
//!      frame and `num_pixels` is the only bound. Adafruit_DotStar's zeros end
//!      frame does stop the decode. Both models do this;
//!      [`apa102_a_ones_end_frame_decodes_as_a_phantom_led_in_both_models`]
//!      holds them to it.
//!   2. **The two wires report `w` differently.** A clocked strip's artifact
//!      reports the number of LEDs it LATCHED (an unpowered one is zero LEDs
//!      wide); a single-wire strip reports its CONFIGURED length whatever the
//!      decoder saw. That asymmetry was already the published contract of the
//!      two deleted models and the browser reads both, so the primitive
//!      reproduces it rather than tidying it.

mod common;
mod led_strip_oracle;

use common::transcript::{run_spi, script, Step, Transcript};
use labwired_core::inspect::{Artifact, DeviceEvidence, InspectOpts};
use labwired_core::peripherals::components::declarative_led_strip::{apa102, ws2812};
use labwired_core::peripherals::components::{GenericLedStrip, LedPixel};
use labwired_core::peripherals::device::GpioObserver;
use labwired_core::peripherals::spi::SpiDevice;
use led_strip_oracle::apa102::Apa102 as OldApa102;
use led_strip_oracle::ws2812::Ws2812 as OldWs2812;
use std::sync::Arc;

const CS: &str = "PA4";
const DATA_PIN: u8 = 48;
/// The decode clock both WS2812 models are built at. 8 MHz makes the HIGH
/// threshold 4 sim cycles and the reset gap 320, which is what
/// `esp32s3_ws2812_rmt.rs` drives the real RMT at.
const CPU_HZ: u64 = 8_000_000;

fn opts() -> InspectOpts {
    InspectOpts {
        include_bytes: true,
        peripheral: None,
    }
}

// ─── comparison ────────────────────────────────────────────────────────────

/// Compare two artifacts field by field, so a failure names WHICH field moved
/// rather than printing two structs side by side. Same shape as
/// `display_migration_parity.rs`, deliberately.
fn assert_same_artifact(old: &Artifact, new: &Artifact, what: &str) {
    assert_eq!(old.kind, new.kind, "{what}: artifact kind");
    assert_eq!(old.id, new.id, "{what}: artifact id");
    let (o, n) = (
        old.meta.as_object().expect("old meta is an object"),
        new.meta.as_object().expect("new meta is an object"),
    );
    let mut keys: Vec<&String> = o.keys().chain(n.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        assert_eq!(
            o.get(k),
            n.get(k),
            "{what}: artifact meta['{k}'] differs — the Rust model said {:?}, the descriptor {:?}",
            o.get(k),
            n.get(k)
        );
    }
    assert_eq!(
        old.bytes.as_ref().map(|b| b.len()),
        new.bytes.as_ref().map(|b| b.len()),
        "{what}: artifact payload length"
    );
    if let (Some(a), Some(b)) = (&old.bytes, &new.bytes) {
        if a != b {
            let first = a
                .iter()
                .zip(b.iter())
                .position(|(x, y)| x != y)
                .expect("lengths equal and contents differ");
            panic!(
                "{what}: artifact payload differs at LED byte {first}: Rust model 0x{:02X}, \
                 descriptor 0x{:02X}",
                a[first], b[first]
            );
        }
    }
}

/// The oracle's `([r, g, b], brightness)` pixel in the descriptor's shape, so
/// the two colour arrays compare as one type.
fn old_apa_pixels(old: &OldApa102) -> Vec<LedPixel> {
    old.pixels()
        .iter()
        .map(|(rgb, brightness)| LedPixel {
            wire: *rgb,
            brightness: *brightness,
        })
        .collect()
}

/// The WS2812 oracle's `[g, r, b]` word in the descriptor's shape. The
/// single-wire artifact publishes WIRE order verbatim — re-ordering would make
/// the twin uncomparable against a capture of the data line — so this is a
/// change of type and not of bytes.
fn old_ws_pixels(old: &OldWs2812) -> Vec<LedPixel> {
    old.pixels()
        .iter()
        .map(|grb| LedPixel {
            wire: *grb,
            brightness: 0,
        })
        .collect()
}

// ─── APA102: the clocked wire ──────────────────────────────────────────────

/// Run one SPI script against both APA102 implementations and assert the
/// transcript, the colour array and the artifact all agree.
fn apa102_parity(what: &str, steps: &[Step<'_>], pixels: usize) -> (Transcript, Vec<LedPixel>) {
    let mut old = OldApa102::new(CS, pixels).with_component_id("strip");
    let mut new = apa102(CS, pixels);
    new.set_component_id("strip");

    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    assert_eq!(
        t_old,
        t_new,
        "{what}: wire transcript differs\nRust model:\n{}\ndescriptor:\n{}",
        t_old.render(),
        t_new.render()
    );

    let p_old = old_apa_pixels(&old);
    let p_new = new.pixels();
    assert_eq!(p_old, p_new, "{what}: latched LED colours differ");

    let a_old = old.artifacts("strip", &opts());
    let a_new = DeviceEvidence::artifacts(&new, "strip", &opts());
    assert_eq!(a_old.len(), a_new.len(), "{what}: artifact count");
    for (o, n) in a_old.iter().zip(a_new.iter()) {
        assert_same_artifact(o, n, what);
    }
    (t_new, p_new)
}

/// A DotStar transaction as a driver writes it: start frame, one 32-bit frame
/// per LED, then the end clocks.
fn dotstar_frame(leds: &[(u8, u8, u8, u8)], end: u8, end_bytes: usize) -> Vec<Step<'static>> {
    let mut bytes: Vec<u8> = vec![0x00, 0x00, 0x00, 0x00];
    for &(brightness, b, g, r) in leds {
        bytes.push(0xE0 | (brightness & 0x1F));
        bytes.extend([b, g, r]);
    }
    bytes.extend(std::iter::repeat_n(end, end_bytes));
    let mut steps = vec![Step::CsSelect];
    steps.extend(bytes.into_iter().map(Step::TransferByte));
    steps.push(Step::CsRelease);
    steps
}

#[test]
fn apa102_a_stock_dotstar_frame_is_byte_identical() {
    // Adafruit_DotStar's `show()`: start frame, the LEDs, then a zeros end
    // frame long enough to shift the last LED down an 8-pixel chain.
    let (_, px) = apa102_parity(
        "apa102 stock frame",
        &dotstar_frame(
            &[
                (31, 0x00, 0x00, 0xFF), // full-bright red
                (16, 0x00, 0xFF, 0x00), // half-bright green
                (1, 0xFF, 0x00, 0x00),  // dimmest blue
                (31, 0xFF, 0xFF, 0xFF), // white
            ],
            0x00,
            4,
        ),
        8,
    );
    // The positive control: without it "both agree" would also pass on two
    // models that latched nothing.
    assert_eq!(px.len(), 4, "four LED frames must decode as four LEDs");
    assert_eq!(px[0].wire, [0xFF, 0x00, 0x00], "LED 0 is red");
    assert_eq!(px[1].brightness, 16, "LED 1 keeps its brightness field");
}

#[test]
fn apa102_a_frame_longer_than_the_strip_is_clipped_identically() {
    let (_, px) = apa102_parity(
        "apa102 over-long frame",
        &dotstar_frame(
            &[
                (31, 0x00, 0x00, 0xFF),
                (31, 0x00, 0xFF, 0x00),
                (31, 0xFF, 0x00, 0x00),
            ],
            0x00,
            4,
        ),
        1,
    );
    assert_eq!(px.len(), 1, "a 1-LED strip keeps one LED");
}

#[test]
fn apa102_a_glitchy_transaction_leaves_the_previous_colours_in_both_models() {
    let mut old = OldApa102::new(CS, 2);
    let mut new = apa102(CS, 2);
    let good = dotstar_frame(&[(31, 0x00, 0x00, 0xFF), (31, 0x00, 0xFF, 0x00)], 0x00, 4);
    run_spi(&mut old, &good);
    run_spi(&mut new, &good);

    // Three shapes of malformed transaction, each of which must change nothing.
    for (what, steps) in [
        (
            "no start frame",
            script([
                vec![Step::CsSelect],
                vec![Step::TransferByte(0xAA)],
                vec![Step::TransferByte(0xBB)],
                vec![Step::CsRelease],
            ]),
        ),
        (
            "start frame but no LED frame",
            script([
                vec![Step::CsSelect],
                (0..4).map(|_| Step::TransferByte(0x00)).collect(),
                vec![Step::CsRelease],
            ]),
        ),
        (
            "an empty transaction",
            vec![Step::CsSelect, Step::CsRelease],
        ),
    ] {
        run_spi(&mut old, &steps);
        run_spi(&mut new, &steps);
        assert_eq!(
            old_apa_pixels(&old),
            new.pixels(),
            "{what}: the two models disagree"
        );
        assert_eq!(
            new.pixels().len(),
            2,
            "{what}: a malformed transfer must not blank the strip"
        );
    }
}

#[test]
fn apa102_a_ones_end_frame_decodes_as_a_phantom_led_in_both_models() {
    // ⚠️ SHARED BEHAVIOUR, NOT A PORT BUG. 0xFF & 0xE0 == 0xE0 passes the header
    // marker, so FastLED's ones end frame is a full-brightness white LED frame
    // as far as either model can tell. Pinned in both so a later "fix" to one
    // side fails here rather than silently diverging the twin from the model
    // whose captures the goldens were taken from.
    let (_, px) = apa102_parity(
        "apa102 ones end frame",
        &dotstar_frame(&[(31, 0x00, 0x00, 0xFF)], 0xFF, 4),
        8,
    );
    assert_eq!(px.len(), 2, "the ones end frame decodes as a second LED");
    assert_eq!(
        px[1],
        LedPixel {
            wire: [0xFF, 0xFF, 0xFF],
            brightness: 0x1F
        },
        "and it is white at full brightness"
    );

    // The other end-frame convention, same script shape: it stops the decode.
    let (_, px) = apa102_parity(
        "apa102 zeros end frame",
        &dotstar_frame(&[(31, 0x00, 0x00, 0xFF)], 0x00, 4),
        8,
    );
    assert_eq!(px.len(), 1, "the zeros end frame stops the decode");
}

#[test]
fn apa102_drives_no_miso_in_either_model() {
    let (t, _) = apa102_parity(
        "apa102 miso",
        &dotstar_frame(&[(31, 0x11, 0x22, 0x33)], 0x00, 4),
        4,
    );
    assert!(
        t.bytes.iter().all(|&b| b == 0x00),
        "an APA102 never drives MISO; got {}",
        t.render()
    );
    assert!(!t.bytes.is_empty(), "the script did clock bytes");
}

#[test]
fn apa102_an_unpowered_strip_latches_nothing_and_says_so_in_both_models() {
    let steps = dotstar_frame(&[(31, 0x00, 0x00, 0xFF), (31, 0x00, 0xFF, 0x00)], 0x00, 4);
    let mut old = OldApa102::new(CS, 4).with_powered(false);
    let mut new = apa102(CS, 4);
    new.set_powered(false);
    run_spi(&mut old, &steps);
    run_spi(&mut new, &steps);

    assert!(old.pixels().is_empty(), "the Rust model latches nothing");
    assert_eq!(
        old_apa_pixels(&old),
        new.pixels(),
        "unpowered colour arrays"
    );

    let a_old = old.artifacts("strip", &opts());
    let a_new = DeviceEvidence::artifacts(&new, "strip", &opts());
    assert_same_artifact(&a_old[0], &a_new[0], "apa102 unpowered");
    assert_eq!(
        a_new[0].meta.get("powered"),
        Some(&serde_json::json!(false)),
        "a dark strip must explain itself: `powered: false`, not an empty frame"
    );

    // The positive control for the gate: the same script on a POWERED strip.
    let (_, px) = apa102_parity("apa102 powered control", &steps, 4);
    assert_eq!(px.len(), 2, "the same script lights a powered strip");
}

// ─── WS2812: the single wire ───────────────────────────────────────────────

/// Replay one list of `(level, sim_cycle)` pad transitions into BOTH WS2812
/// implementations. This is the single-wire equivalent of a bus script: the
/// stimulus is edge timing on one pad, so the "transcript" is the edge list
/// itself and it is shared by construction.
fn feed_edges(old: &OldWs2812, new: &GenericLedStrip, edges: &[(bool, u64)]) {
    let mut level = false;
    for &(to, cycle) in edges {
        GpioObserver::on_pin_change(old, DATA_PIN, level, to, cycle);
        GpioObserver::on_pin_change(new, DATA_PIN, level, to, cycle);
        level = to;
    }
}

/// HIGH cycles for a 0 bit and a 1 bit at [`CPU_HZ`], both straddling the
/// 4-cycle threshold the 500 ns `high_threshold_ns` derives to.
const T0H: u64 = 2;
const T1H: u64 = 6;
/// Inter-bit LOW, well under the 320-cycle reset gap.
const TLOW: u64 = 2;

/// Append one pixel's 24 bits (MSB first) to an edge list, advancing `cycle`.
fn push_pixel(edges: &mut Vec<(bool, u64)>, cycle: &mut u64, grb: u32) {
    for i in (0..24).rev() {
        let high = if (grb >> i) & 1 != 0 { T1H } else { T0H };
        edges.push((true, *cycle));
        *cycle += high;
        edges.push((false, *cycle));
        *cycle += TLOW;
    }
}

/// A whole frame plus the reset gap that latches it.
fn ws_frame(pixels: &[u32], latch: bool) -> Vec<(bool, u64)> {
    let mut edges = Vec::new();
    let mut cycle = 100u64;
    for &p in pixels {
        push_pixel(&mut edges, &mut cycle, p);
    }
    if latch {
        // A rising edge after a gap ≥ the reset threshold is what closes a
        // frame. Nothing else does — see the decoder.
        edges.push((true, cycle + 1_000));
        edges.push((false, cycle + 1_002));
    }
    edges
}

/// Compare both WS2812 implementations after one edge list.
fn ws2812_parity(what: &str, edges: &[(bool, u64)], pixels: usize) -> Vec<LedPixel> {
    let old = OldWs2812::new(DATA_PIN, pixels, CPU_HZ).with_component_id("strip");
    let mut new = ws2812(DATA_PIN, pixels, CPU_HZ);
    new.set_component_id("strip");
    feed_edges(&old, &new, edges);

    let p_old = old_ws_pixels(&old);
    let p_new = new.pixels();
    assert_eq!(p_old, p_new, "{what}: decoded LED colours differ");

    let a_old = old.artifacts("strip", &opts());
    let a_new = DeviceEvidence::artifacts(&new, "strip", &opts());
    assert_eq!(a_old.len(), a_new.len(), "{what}: artifact count");
    for (o, n) in a_old.iter().zip(a_new.iter()) {
        assert_same_artifact(o, n, what);
    }
    p_new
}

#[test]
fn ws2812_a_three_pixel_frame_decodes_identically() {
    // The frame `esp32s3_ws2812_rmt.rs` plays through the real RMT: red, green,
    // blue in GRB wire order.
    let px = ws2812_parity(
        "ws2812 rgb frame",
        &ws_frame(&[0x00_FF_00, 0xFF_00_00, 0x00_00_FF], false),
        3,
    );
    assert_eq!(
        px.iter().map(|p| p.wire).collect::<Vec<_>>(),
        vec![[0x00, 0xFF, 0x00], [0xFF, 0x00, 0x00], [0x00, 0x00, 0xFF]],
        "the positive control: the edges really did decode to red, green, blue"
    );
}

#[test]
fn ws2812_a_reset_gap_latches_the_frame_in_both_models() {
    let px = ws2812_parity(
        "ws2812 latched frame",
        &ws_frame(&[0x11_22_33, 0x44_55_66], true),
        2,
    );
    assert_eq!(
        px.iter().map(|p| p.wire).collect::<Vec<_>>(),
        vec![[0x11, 0x22, 0x33], [0x44, 0x55, 0x66]],
        "a latched frame is still readable"
    );
}

#[test]
fn ws2812_a_second_frame_replaces_the_first_in_both_models() {
    let old = OldWs2812::new(DATA_PIN, 2, CPU_HZ);
    let mut new = ws2812(DATA_PIN, 2, CPU_HZ);
    new.set_component_id("strip");

    feed_edges(&old, &new, &ws_frame(&[0xAA_BB_CC, 0xDD_EE_FF], true));
    assert_eq!(old_ws_pixels(&old), new.pixels(), "after the first frame");

    // A second frame, offset past the first, with the reset gap in between.
    let mut second = Vec::new();
    let mut cycle = 100_000u64;
    push_pixel(&mut second, &mut cycle, 0x01_02_03);
    push_pixel(&mut second, &mut cycle, 0x04_05_06);
    second.push((true, cycle + 1_000));
    second.push((false, cycle + 1_002));
    feed_edges(&old, &new, &second);

    assert_eq!(old_ws_pixels(&old), new.pixels(), "after the second frame");
    assert_eq!(
        new.pixels().iter().map(|p| p.wire).collect::<Vec<_>>(),
        vec![[0x01, 0x02, 0x03], [0x04, 0x05, 0x06]],
        "the second frame replaced the first"
    );
}

#[test]
fn ws2812_a_frame_longer_than_the_strip_is_clipped_identically() {
    let px = ws2812_parity(
        "ws2812 over-long frame",
        &ws_frame(&[0x11_11_11, 0x22_22_22, 0x33_33_33], false),
        2,
    );
    assert_eq!(px.len(), 2, "a 2-LED strip keeps two LEDs");
}

#[test]
fn ws2812_a_strip_that_saw_no_edges_reports_nothing_in_both_models() {
    let px = ws2812_parity("ws2812 silent pad", &[], 4);
    assert!(px.is_empty(), "no edges decode to no pixels");

    // The artifact still reports the strip's CONFIGURED width — the asymmetry
    // the module note names — so the overlay draws four dark LEDs rather than
    // nothing at all.
    let new = ws2812(DATA_PIN, 4, CPU_HZ);
    let a = DeviceEvidence::artifacts(&new, "strip", &opts());
    assert_eq!(a[0].meta.get("w"), Some(&serde_json::json!(4)));
    assert_eq!(a[0].meta.get("pixels_decoded"), Some(&serde_json::json!(0)));
    assert_eq!(a[0].meta.get("lit_pixels"), Some(&serde_json::json!(0)));
}

#[test]
fn ws2812_edges_on_another_pad_are_ignored_by_both_models() {
    let old = OldWs2812::new(DATA_PIN, 2, CPU_HZ);
    let new = ws2812(DATA_PIN, 2, CPU_HZ);
    // The same waveform, on the wrong pin.
    let mut level = false;
    for &(to, cycle) in ws_frame(&[0x11_22_33, 0x44_55_66], true).iter() {
        GpioObserver::on_pin_change(&old, DATA_PIN + 1, level, to, cycle);
        GpioObserver::on_pin_change(&new, DATA_PIN + 1, level, to, cycle);
        level = to;
    }
    assert!(old.pixels().is_empty(), "the Rust model ignores other pads");
    assert_eq!(old_ws_pixels(&old), new.pixels(), "both ignore other pads");

    // The positive control: the SAME waveform on the right pad decodes.
    let px = ws2812_parity(
        "ws2812 right pad control",
        &ws_frame(&[0x11_22_33, 0x44_55_66], true),
        2,
    );
    assert_eq!(px.len(), 2, "the waveform itself was decodable");
}

#[test]
fn ws2812_the_strip_is_shared_as_an_arc_and_still_decodes() {
    // The observer hook is `&self`, so the part is held as `Arc<…>` on the bus
    // (`esp32s3_ws2812_rmt.rs`). The oracle was too; the descriptor must not
    // have quietly become `&mut self`-only.
    let old = Arc::new(OldWs2812::new(DATA_PIN, 1, CPU_HZ));
    let new = Arc::new(ws2812(DATA_PIN, 1, CPU_HZ));
    let (o, n) = (old.clone(), new.clone());
    feed_edges(&o, &n, &ws_frame(&[0x0F_F0_0F], true));
    assert_eq!(old_ws_pixels(&old), new.pixels(), "through an Arc");
    assert_eq!(new.pixels()[0].wire, [0x0F, 0xF0, 0x0F]);
}
