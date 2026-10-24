// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Shared helpers for the ESP32-family I²C waveform gates. The decoder is an
//! INDEPENDENT I²C decoder — protocol rules only, knowing nothing about how
//! the waveform was produced; identical rules to the STM32/S3/C3 gates.

use crate::logic_capture::LogicEdge;

/// Reconstruct `(byte, acked)` frames from recorded SCL/SDA edges.
pub fn decode(edges: &[LogicEdge], ch_scl: u32, ch_sda: u32) -> Vec<(u8, bool)> {
    let (mut scl, mut sda) = (true, true);
    let mut started = false;
    let mut bits: Vec<bool> = Vec::new();
    let mut frames = Vec::new();
    for edge in edges {
        let (prev_scl, prev_sda) = (scl, sda);
        if edge.ch == ch_scl {
            scl = edge.value;
        } else if edge.ch == ch_sda {
            sda = edge.value;
        } else {
            continue;
        }
        if edge.ch == ch_sda && prev_sda && !sda && scl {
            started = true;
            bits.clear();
            continue;
        }
        if edge.ch == ch_sda && !prev_sda && sda && scl {
            started = false;
            bits.clear();
            continue;
        }
        if started && edge.ch == ch_scl && !prev_scl && scl {
            bits.push(sda);
            if bits.len() == 9 {
                let byte = bits[..8]
                    .iter()
                    .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit));
                frames.push((byte, !bits[8]));
                bits.clear();
            }
        }
    }
    frames
}

/// Assert SCL rises once per programmed bit time. Fails when fewer than a
/// full frame of clocks was captured, or when more than the final inter-rise
/// gap deviates from `expected_bit_time`.
pub fn assert_scl_rate(edges: &[LogicEdge], ch_scl: u32, expected_bit_time: u64) {
    let rises: Vec<u64> = edges
        .iter()
        .filter(|e| e.ch == ch_scl && e.value)
        .map(|e| e.cycle)
        .collect();
    assert!(rises.len() >= 9, "at least one full frame of clocks");
    let gaps: Vec<u64> = rises.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let at_rate = gaps.iter().filter(|&&g| g == expected_bit_time).count();
    assert!(
        at_rate >= gaps.len() - 1,
        "SCL period should be {expected_bit_time} cycles, got {gaps:?}",
    );
}
