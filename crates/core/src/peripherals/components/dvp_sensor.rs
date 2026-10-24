// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **DVP camera seam** — one frame of D[7:0], and nothing else.
//!
//! The receive-side twin of [`I80Panel`](super::I80Panel). A camera slave
//! (`LCD_CAM`'s CAM half on the ESP32-S3) samples D[7:0] on PCLK while HREF is
//! high, between two VSYNC pulses. What it needs from the sensor on the far end
//! of the ribbon is exactly that byte stream, in wire order, once per frame.
//! Everything else about a sensor — its SCCB register file, its windowing and
//! format logic, how it renders a scene — the controller neither knows nor may
//! know, for the same reason the i80 seam exists: a chip peripheral that names
//! a part cannot be reused by the next part.
//!
//! Timing (PCLK rate, porches, blanking) is deliberately NOT in the trait. It
//! belongs to the controller's frame state machine, which is where the
//! interrupt ordering the driver depends on (VSYNC → N × IN_SUC_EOF → VSYNC)
//! is produced. A sensor answers one question: "what does your next frame put
//! on the pins?"

/// A camera sensor wired to a DVP (8-bit parallel) camera slave.
///
/// `&self`, not `&mut self`: the sensor is shared (`Arc`) between the SCCB
/// slave that the I²C controller owns and the camera controller that pulls its
/// frames, so it owns its own interior mutability.
pub trait DvpSensor: std::fmt::Debug + Send + Sync {
    /// The bytes the sensor drives on D[7:0] for its next frame — one byte per
    /// PCLK with HREF high, all lines concatenated, in wire order.
    ///
    /// `None` while the sensor is not streaming (held in reset, powered down,
    /// or configured for an output this model cannot produce). The controller
    /// then sees no frames at all, which is what a silent sensor looks like on
    /// the pins: no VSYNC, no data.
    fn next_frame(&self) -> Option<Vec<u8>>;
}
