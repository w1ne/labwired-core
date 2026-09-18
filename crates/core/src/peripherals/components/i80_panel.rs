// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The **i80 (8080 parallel) panel seam** — one bus cycle, and nothing else.
//!
//! # Why this exists
//!
//! `peripherals/esp32s3/lcd_cam.rs` used to hold
//! `panels: Vec<Arc<Ili9341Parallel>>` and take `Arc<Ili9341Parallel>` in
//! `attach_panel`. A CHIP PERIPHERAL named a PART. That is the wrong direction
//! for a dependency and it had a concrete cost: the parallel ILI9341 could not
//! become a descriptor, because the engine would then have no type to hold.
//! #1174 named exactly this as what blocked the port, and said the seam is a
//! bus/peripheral change with its own proof rather than a rider on a panel
//! port.
//!
//! # What the i80 master actually needs
//!
//! One operation. `Esp32s3LcdCam::emit_word` applies `LCD_USER`'s byte- and
//! bit-order bits to a bus word, works out the D/C level from
//! `LCD_MISC.CD_*_SET ^ CD_IDLE_EDGE`, and hands the pair to every attached
//! panel. Everything else about a panel — its framebuffer, its command table,
//! its MADCTL, its power state, how it reports — the controller neither knows
//! nor may know: the strobe is the whole contract.
//!
//! So the trait is one method, and the port stays primitive for the same reason
//! [`DevicePins`](crate::bus::DevicePins) does: a second method handing back an
//! engine or model type would re-couple the controller to the part without
//! breaking a single build. `i80_panel_seam_stays_narrow` in
//! `crates/core/tests/esp32s3_lcd_i80_pixels.rs` reads this trait's body and
//! fails if one appears.
//!
//! # What it is NOT
//!
//! It is not "a display". A panel's pixels, geometry and packing are reported
//! through [`DeviceEvidence`](crate::inspect::DeviceEvidence) as DATA —
//! `meta.w`, `meta.h`, `meta.format`, and the bytes — which is how every other
//! panel in the tree is read back, and which is why the byte-exact i80 tests
//! no longer downcast to a concrete panel type to check what was painted.

/// A panel wired to an 8080-mode parallel master.
///
/// `&self`, not `&mut self`: a panel is shared (`Arc`) between the GPIO
/// observer that watches its pads and the peripheral that strobes its bus, so
/// it owns its own interior mutability. The controller holds it read-only.
pub trait I80Panel: std::fmt::Debug + Send + Sync {
    /// One 8080 write strobe: latch `word` on DB[15:0] with RS/DC at
    /// `dc_high`, then pulse WR.
    ///
    /// `word` has already had the master's output-format bits applied (byte
    /// swap, bit reverse, 8- vs 16-bit width). What arrives here is what the
    /// pads would carry.
    fn i80_write_word(&self, dc_high: bool, word: u16);
}
