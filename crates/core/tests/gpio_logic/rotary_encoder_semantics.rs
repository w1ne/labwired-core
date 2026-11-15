// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Rotary migration semantics: first edge is one full interval after the first
//! serviced tick following a changed target; position rounds to nearest detent.
//! These assertions now exercise the generic GPIO descriptor. The separate
//! differential suite preserves the original Rust implementation verbatim.

use crate::rotary_fixture as fixture;
use fixture::RotaryHarness as RotaryEncoder;
use labwired_core::sim_input::SimInput;

/// The model's edge spacing in µs. A private constant there; restated here so
/// this file measures the behaviour rather than importing the number that
/// produces it.
const EDGE_INTERVAL_US: u64 = 2_000;
/// 1 MHz ⇒ one cycle per microsecond, so cycles and µs are the same number.
const CPU_HZ: u64 = 1_000_000;

fn enc() -> RotaryEncoder {
    RotaryEncoder::new("knob".into(), 0x4800_0410, 3, 0x4800_0410, 4, CPU_HZ)
}

/// Service the encoder every `stride` cycles from `from` to `until`, returning
/// the cycle of every LEVEL TRANSITION — which is what a firmware decoder sees.
fn edge_cycles(e: &mut RotaryEncoder, from: u64, until: u64, stride: u64) -> Vec<u64> {
    let mut edges = Vec::new();
    let mut now = from;
    while now <= until {
        let (_, (clk_changed, dt_changed)) = e.service(now);
        if clk_changed || dt_changed {
            edges.push(now);
        }
        now += stride;
    }
    edges
}

// ─── Question 1: the cadence anchors on the first SERVICED tick ────────────

/// ⚠️ ANSWER 1, BY NAME. Whatever cycle the stimulus arrives on, the first edge
/// of the move lands a FULL interval after the first tick that services it —
/// never sooner. A free-running-grid port would land the first edge early
/// whenever the stimulus arrived mid-interval, which is the behaviour this
/// asserts against.
#[test]
fn the_cadence_re_anchors_on_the_first_serviced_tick_after_a_retarget() {
    // Retarget at a cycle deliberately OFF any multiple of the interval, and
    // after the encoder has already been running long enough that a
    // free-running grid would have a stale anchor to fire against.
    for offset in [1u64, 7, 999, EDGE_INTERVAL_US - 1] {
        let mut e = enc();
        e.service(0); // settle at rest
        let retarget_at = 10 * EDGE_INTERVAL_US + offset;

        // Nothing is asked of it until the retarget.
        e.service(retarget_at - 1);
        e.set_input("position", 1.0).expect("position channel");

        // The tick that FIRST services the move is the anchor; it drives no edge.
        let anchor = retarget_at;
        let (_, changed) = e.service(anchor);
        assert_eq!(
            changed,
            (false, false),
            "offset {offset}: the anchoring tick must drive no edge"
        );

        // One cycle before a full interval: still nothing.
        let (_, changed) = e.service(anchor + EDGE_INTERVAL_US - 1);
        assert_eq!(
            changed,
            (false, false),
            "offset {offset}: no edge before a FULL interval from the anchor"
        );
        // Exactly one interval from the anchor: the first edge.
        let (levels, changed) = e.service(anchor + EDGE_INTERVAL_US);
        assert_eq!(levels, (false, true), "offset {offset}: phase 1 = 01");
        assert_eq!(
            changed,
            (true, false),
            "offset {offset}: the first edge is CLK falling, one interval from the anchor"
        );
    }
}

/// ⚠️ THE INVARIANT ANSWER 1 EXISTS TO KEEP: no gap between consecutive edges
/// is EVER shorter than one interval, the first one included. This is the
/// datasheet argument stated as a measurement, and it is what a port must
/// preserve however it spells its timer.
#[test]
fn no_inter_edge_gap_is_ever_shorter_than_one_interval() {
    for offset in [0u64, 1, 333, EDGE_INTERVAL_US - 1] {
        let mut e = enc();
        e.service(0);
        let retarget_at = 5 * EDGE_INTERVAL_US + offset;
        e.service(retarget_at - 1);
        e.set_input("position", 3.0).expect("position channel");

        // Sample far faster than the edge interval, the way a polling decoder
        // does, so every transition is seen on the cycle it happens.
        let edges = edge_cycles(&mut e, retarget_at, retarget_at + 40 * EDGE_INTERVAL_US, 1);
        assert_eq!(
            edges.len(),
            12,
            "offset {offset}: three detents is twelve phase transitions"
        );
        for pair in edges.windows(2) {
            assert!(
                pair[1] - pair[0] >= EDGE_INTERVAL_US,
                "offset {offset}: edges at {} and {} are {} cycles apart, shorter than the \
                 {EDGE_INTERVAL_US}-cycle phase an EC11 guarantees",
                pair[0],
                pair[1],
                pair[1] - pair[0]
            );
        }
        assert!(
            edges[0] - retarget_at >= EDGE_INTERVAL_US,
            "offset {offset}: the FIRST edge is the one a grid-anchored port gets wrong"
        );
    }
}

// ─── Question 2: rounding, on both sides ───────────────────────────────────

/// ⚠️ ANSWER 2, BY NAME. `set_input` lands on the NEAREST detent, symmetrically
/// about zero, and the position the encoder reports once the walk finishes is
/// that same integer. A truncating port disagrees for every fractional value.
#[test]
fn set_input_lands_on_the_nearest_detent_and_the_readback_agrees() {
    // (driven value, the detent a ROUND gives, the detent a TRUNCATE would give)
    for (driven, rounded, truncated) in [
        (2.6f64, 3i64, 2i64),
        (2.4, 2, 2),
        (2.5, 3, 2),
        (-2.6, -3, -2),
        (-2.4, -2, -2),
        (0.5, 1, 0),
        (-0.5, -1, 0),
    ] {
        let mut e = enc();
        e.service(0);
        e.set_input("position", driven).expect("position channel");
        // Walk far enough for any of these to finish.
        for k in 1..=40u64 {
            e.service(k * EDGE_INTERVAL_US);
        }
        assert!(!e.is_moving(), "{driven} detents: the walk must finish");
        assert_eq!(
            e.position_detents(),
            rounded,
            "{driven} detents must land on the NEAREST stop"
        );
        if rounded != truncated {
            assert_ne!(
                e.position_detents(),
                truncated,
                "{driven} detents: truncation would land a whole detent short — this is the \
                 disagreement `input()` currently has with `set_input`"
            );
        }
    }
}

/// The symmetry half of the argument, stated on its own: the knob must behave
/// the same either side of the origin. Truncation does not — it biases toward
/// zero, so a half-detent clockwise counts and a half-detent anticlockwise does
/// not.
#[test]
fn rounding_is_symmetric_about_the_origin_where_truncation_is_not() {
    let mut cw = enc();
    cw.service(0);
    cw.set_input("position", 1.5).unwrap();
    let mut ccw = enc();
    ccw.service(0);
    ccw.set_input("position", -1.5).unwrap();
    for k in 1..=20u64 {
        cw.service(k * EDGE_INTERVAL_US);
        ccw.service(k * EDGE_INTERVAL_US);
    }
    assert_eq!(cw.position_detents(), 2);
    assert_eq!(ccw.position_detents(), -2);
    assert_eq!(
        cw.position_detents(),
        -ccw.position_detents(),
        "the same gesture either way round must move the same number of stops"
    );
}
