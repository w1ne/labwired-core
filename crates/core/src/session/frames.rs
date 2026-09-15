// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Decoded bus traffic as a session reads it.
//!
//! A [`Frame`] is one event from the machine's shared bus trace (I²C, SPI,
//! UART, CAN) with its cycle stamp converted to virtual time at the session's
//! clock and a one-line summary attached.

use crate::bus::bus_trace::{BusPayload, BusTraceEvent};
use std::time::Duration;

/// One transacted symbol or frame.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Frame {
    /// Trace sequence number, strictly increasing across the whole session.
    pub seq: u64,
    /// Engine cycle at which it transacted.
    pub cycle: u64,
    /// `cycle` at the session clock.
    pub at: Duration,
    /// Bus instance name (`twi21`, `i2c1`, `uart20`, `fdcan1`, ...).
    pub bus: String,
    /// Human summary, the `Display` of [`BusPayload`].
    pub summary: String,
    pub payload: BusPayload,
}

pub(crate) fn from_event(ev: BusTraceEvent, cpu_hz: u64) -> Frame {
    Frame {
        seq: ev.seq,
        cycle: ev.cycle,
        at: super::duration_for_cycles(ev.cycle, cpu_hz),
        bus: ev.bus,
        summary: ev.payload.to_string(),
        payload: ev.payload,
    }
}
