// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Egress bridge: stream simulated peripheral output out to a real backend.
//!
//! The deterministic sim core only enqueues [`EgressItem`]s; a worker thread
//! owned by the transport performs the blocking network write.

pub mod bus;
pub mod encoding;
pub mod tap;
pub mod transport;

use crate::network::CanFrame;

/// One unit of output captured from a simulated peripheral.
#[derive(Debug, Clone, PartialEq)]
pub enum EgressItem {
    /// A single byte transmitted on a UART TX line.
    Byte(u8),
    /// A CAN/CAN-FD frame transmitted by the firmware.
    Frame(CanFrame),
}

/// How buffered [`EgressItem`]s become an on-wire payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EncodingKind {
    /// Bytes verbatim; frames as their raw data field.
    Raw,
    /// One JSON object per item, newline-delimited.
    NdjsonTrace,
    /// One JSON object per CAN frame.
    FramesJson,
}

/// Bounded-buffer policy. On overflow the oldest item is dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferPolicy {
    pub max: usize,
}

impl Default for BufferPolicy {
    fn default() -> Self {
        Self { max: 4096 }
    }
}
