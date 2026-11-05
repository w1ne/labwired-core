// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! TM1637 protocol / panel history for Acc prove.
//!
//! The declarative TM1637 already decodes START/STOP/data/ACK clocks into GRID
//! RAM. Prove cases also need (a) the completed bus transactions as byte
//! sequences, (b) that ACK clocks were consumed with no framing errors, and
//! (c) a time-stamped ring of panel frames so a countdown can be checked to
//! ±50 ms on the TM1637 frame times rather than on serial alone.
//!
//! This recorder sits beside the rule machine: after each Pins event it
//! snapshots `in_txn` / `expect_ack` / GRID vars and appends to the rings.
//! It does **not** drive DIO for the silicon ACK (firmware never reads it);
//! it counts the ACK clocks the descriptor already consumes.

use std::collections::VecDeque;

use serde_json::{json, Value};

use super::rule_machine::RuleMachine;
use super::seven_seg_font;

/// Cap on each history ring (oldest dropped). Enough for a multi-step Acc
/// prove without ballooning the artifact.
const CAP: usize = 64;

/// Live TM1637 bus + panel history, attached only when the artifact format is
/// `tm1637_grid`.
#[derive(Debug, Default)]
pub struct Tm1637Trace {
    last_in_txn: i64,
    last_expect_ack: i64,
    seeded: bool,
    txn_bytes: Vec<u8>,
    last_grids: [u8; 4],
    last_display_on: i64,
    last_bright: i64,
    frames_seen: VecDeque<Value>,
    bus_frames: VecDeque<Value>,
    ack_count: u64,
    framing_errors: u64,
}

impl Tm1637Trace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe post-rule state after a Pins (or equivalent) event.
    pub fn observe(&mut self, machine: &RuleMachine, t_us: u64) {
        let in_txn = machine.var("in_txn");
        let expect_ack = machine.var("expect_ack");
        let bit_buf = machine.var("bit_buf") as u8;
        let grids = [
            machine.var("g0") as u8,
            machine.var("g1") as u8,
            machine.var("g2") as u8,
            machine.var("g3") as u8,
        ];
        let display_on = machine.var("display_on");
        let bright = machine.var("bright");

        if !self.seeded {
            self.last_in_txn = in_txn;
            self.last_expect_ack = expect_ack;
            self.last_grids = grids;
            self.last_display_on = display_on;
            self.last_bright = bright;
            self.seeded = true;
            return;
        }

        if self.last_in_txn == 0 && in_txn == 1 {
            // Fresh START. A prior STOP may leave a partial bit_count from the
            // CLK↑/DIO↓ prelude (the sample rule fires before STOP); that is
            // normal, not a framing error. Only a pending ACK is corrupt.
            if self.last_expect_ack != 0 {
                self.framing_errors = self.framing_errors.saturating_add(1);
            }
            self.txn_bytes.clear();
        }

        // Byte completed: close-byte rule raises expect_ack 0→1 with bit_buf set.
        if self.last_expect_ack == 0 && expect_ack == 1 {
            self.txn_bytes.push(bit_buf);
        }

        // ACK clock consumed: expect_ack 1→0 while still in the transaction.
        if self.last_expect_ack == 1 && expect_ack == 0 && in_txn == 1 {
            self.ack_count = self.ack_count.saturating_add(1);
        }

        // STOP: publish the transaction. A pending ACK (byte closed, ninth
        // clock never seen) is a framing error. A non-zero bit_count is the
        // normal STOP prelude and is not.
        if self.last_in_txn == 1 && in_txn == 0 {
            if self.last_expect_ack == 1 {
                self.framing_errors = self.framing_errors.saturating_add(1);
            }
            if !self.txn_bytes.is_empty() {
                let bytes = std::mem::take(&mut self.txn_bytes);
                self.push_bus(t_us, bytes);
            }
        }

        // Panel frame: GRID or display-control change.
        if grids != self.last_grids
            || display_on != self.last_display_on
            || bright != self.last_bright
        {
            let text: String = grids.iter().map(|b| seven_seg_font::decode(*b)).collect();
            self.push_frame(json!({
                "t_us": t_us,
                "grids": grids.iter().copied().collect::<Vec<_>>(),
                "text": text,
                "display_on": display_on != 0,
                "brightness": bright,
            }));
            self.last_grids = grids;
            self.last_display_on = display_on;
            self.last_bright = bright;
        }

        self.last_in_txn = in_txn;
        self.last_expect_ack = expect_ack;
    }

    fn push_bus(&mut self, t_us: u64, bytes: Vec<u8>) {
        if self.bus_frames.len() >= CAP {
            self.bus_frames.pop_front();
        }
        self.bus_frames.push_back(json!({ "t_us": t_us, "bytes": bytes }));
    }

    fn push_frame(&mut self, frame: Value) {
        if self.frames_seen.len() >= CAP {
            self.frames_seen.pop_front();
        }
        self.frames_seen.push_back(frame);
    }

    /// Merge history into an artifact meta map (caller owns include_bytes gate).
    pub fn write_meta(&self, meta: &mut serde_json::Map<String, Value>) {
        meta.insert(
            "grids".into(),
            json!([
                self.last_grids[0],
                self.last_grids[1],
                self.last_grids[2],
                self.last_grids[3]
            ]),
        );
        meta.insert(
            "frames_seen".into(),
            Value::Array(self.frames_seen.iter().cloned().collect()),
        );
        meta.insert(
            "bus_frames".into(),
            Value::Array(self.bus_frames.iter().cloned().collect()),
        );
        meta.insert("ack_count".into(), json!(self.ack_count));
        meta.insert("framing_errors".into(), json!(self.framing_errors));
    }
}
