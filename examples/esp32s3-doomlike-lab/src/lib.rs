//! Host-testable Doom-like game modules for the ESP32-S3 community lab.
//!
//! All gameplay, rendering, and input logic is integer-only and heap-free so
//! the same crate exercises on the host (unit tests) and on the S3 binary.

#![cfg_attr(not(test), no_std)]

pub mod game;
pub mod level;
