// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Transcript-parity harness** — drive any off-chip device model through a
//! scripted bus conversation and collect the bytes it put on the wire.
//!
//! Why this is shared test support and not a private helper
//! ========================================================
//! `declarative_device_byte_parity.rs` pins a golden byte transcript for every
//! shipping declarative device, and it grew its own private script runner: a
//! handful of free functions (`read_reg`, `write_reg`, `send_cmd16`,
//! `spi_xfer`, …) that know how to frame an I²C or SPI conversation. That
//! runner is the reusable part. Porting a device to YAML means proving the new
//! descriptor byte-identical to whatever modelled it before, and every such
//! proof needs exactly this: a script, a transcript, a comparison.
//!
//! Lifting it here makes the porting harness a thing you CALL rather than a
//! thing you copy. `declarative_device_byte_parity.rs` is the proof the lift
//! changed nothing: every golden constant in that file is byte-identical
//! across this refactor, and it still fails if the shared engine moves a byte.
//!
//! The vocabulary
//! ==============
//! One [`Step`] enum covers both buses, because a script is a *conversation*
//! and the two transports differ only in framing:
//!
//! | Step | I²C | SPI |
//! |---|---|---|
//! | [`Step::Start`] | START / repeated START | — (rejected) |
//! | [`Step::Stop`] | STOP | — (rejected) |
//! | [`Step::Write`] | one byte master→device | — (rejected) |
//! | [`Step::Read`] | N bytes device→master, collected | — (rejected) |
//! | [`Step::CsSelect`] / [`Step::CsRelease`] | — (rejected) | CS↓ / CS↑ |
//! | [`Step::Transfer`] | — (rejected) | full-duplex bytes, MISO collected |
//! | [`Step::AdvanceUs`] | `advance_time_us` | `advance_time_us` |
//! | [`Step::Input`] | `set_input` | `set_input` |
//!
//! A step that does not belong to the bus being driven PANICS rather than being
//! quietly ignored: a script that silently skipped half its steps would produce
//! a shorter transcript that still compared equal to a shorter golden, which is
//! precisely the vacuous green this harness exists to make impossible.
//!
//! Only [`Step::Read`] and [`Step::Transfer`] contribute bytes to the
//! transcript — the same rule the hand-written runner followed, which is why
//! the goldens did not move.

#![allow(dead_code)]

use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;

/// One step of a scripted bus conversation. See the module note for which
/// steps are legal on which bus.
#[derive(Clone, Debug, PartialEq)]
pub enum Step<'a> {
    /// I²C START (or repeated START).
    Start,
    /// I²C STOP.
    Stop,
    /// I²C: one byte master → device.
    Write(u8),
    /// I²C: clock `n` bytes device → master, all of them into the transcript.
    Read(usize),
    /// SPI: CS goes low.
    CsSelect,
    /// SPI: CS goes high.
    CsRelease,
    /// SPI: clock these MOSI bytes; every MISO byte lands in the transcript.
    Transfer(&'a [u8]),
    /// Advance the device's own clock by this many simulated microseconds.
    /// Legal on both buses — that symmetry is the point of Phase A.
    AdvanceUs(u64),
    /// Drive a simulated input channel (`set_input`). Legal on both buses.
    Input(&'a str, f64),
}

/// What a script produced: the wire bytes, plus a rendering for humans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript {
    /// Every byte the device put on the wire, in order. THE artifact — a
    /// golden constant is compared against this.
    pub bytes: Vec<u8>,
}

impl Transcript {
    /// The golden-table literal for these bytes, so re-blessing a deliberate
    /// change is a copy-paste rather than a hand edit.
    pub fn as_literal(&self) -> String {
        let hex: Vec<String> = self.bytes.iter().map(|b| format!("0x{b:02X}")).collect();
        format!("&[{}]", hex.join(", "))
    }

    /// A readable rendering: one line per byte group with its index, for
    /// eyeballing a diff that `assert_eq!` on a 30-byte slice makes unreadable.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for (i, chunk) in self.bytes.chunks(16).enumerate() {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02X}")).collect();
            out.push_str(&format!("{:04}: {}\n", i * 16, hex.join(" ")));
        }
        out
    }
}

/// Drive an I²C device through `script` and return its byte transcript.
///
/// # Panics
/// On a SPI-only step — see the module note on why this is loud.
pub fn run_i2c(dev: &mut dyn I2cDevice, script: &[Step<'_>]) -> Transcript {
    let mut bytes = Vec::new();
    for (i, step) in script.iter().enumerate() {
        match step {
            Step::Start => dev.start(),
            Step::Stop => dev.stop(),
            Step::Write(b) => dev.write(*b),
            Step::Read(n) => bytes.extend((0..*n).map(|_| dev.read())),
            Step::AdvanceUs(us) => dev.advance_time_us(*us),
            Step::Input(key, value) => {
                let sim = dev
                    .as_sim_input_mut()
                    .unwrap_or_else(|| panic!("step {i}: device accepts no simulated input"));
                sim.set_input(key, *value)
                    .unwrap_or_else(|e| panic!("step {i}: set_input({key}, {value}): {e}"));
            }
            other => panic!("step {i}: {other:?} is a SPI step in an I²C script"),
        }
    }
    Transcript { bytes }
}

/// Drive a SPI device through `script` and return its byte transcript.
///
/// # Panics
/// On an I²C-only step — see the module note on why this is loud.
pub fn run_spi(dev: &mut dyn SpiDevice, script: &[Step<'_>]) -> Transcript {
    let mut bytes = Vec::new();
    for (i, step) in script.iter().enumerate() {
        match step {
            Step::CsSelect => dev.cs_select(),
            Step::CsRelease => dev.cs_release(),
            Step::Transfer(mosi) => bytes.extend(mosi.iter().map(|&b| dev.transfer(b))),
            Step::AdvanceUs(us) => dev.advance_time_us(*us),
            Step::Input(key, value) => {
                let sim = dev
                    .as_sim_input_mut()
                    .unwrap_or_else(|| panic!("step {i}: device accepts no simulated input"));
                sim.set_input(key, *value)
                    .unwrap_or_else(|e| panic!("step {i}: set_input({key}, {value}): {e}"));
            }
            other => panic!("step {i}: {other:?} is an I²C step in a SPI script"),
        }
    }
    Transcript { bytes }
}

// ─── script sugar for the framings every register sensor uses ──────────────

/// Point at `reg`, repeated-START into the read phase, read `n` bytes, STOP.
pub fn read_reg(reg: u8, n: usize) -> Vec<Step<'static>> {
    vec![
        Step::Start,
        Step::Write(reg),
        Step::Start,
        Step::Read(n),
        Step::Stop,
    ]
}

/// Write `bytes` into `reg` (pointer first), framed START … STOP. Contributes
/// nothing to the transcript.
pub fn write_reg(reg: u8, bytes: &[u8]) -> Vec<Step<'static>> {
    let mut steps = vec![Step::Start, Step::Write(reg)];
    steps.extend(bytes.iter().map(|&b| Step::Write(b)));
    steps.push(Step::Stop);
    steps
}

/// Send a 16-bit big-endian opcode, framed START … STOP.
pub fn send_cmd16(code: u16) -> Vec<Step<'static>> {
    vec![
        Step::Start,
        Step::Write((code >> 8) as u8),
        Step::Write((code & 0xFF) as u8),
        Step::Stop,
    ]
}

/// Send a single-byte opcode, framed START … STOP.
pub fn send_cmd8(code: u8) -> Vec<Step<'static>> {
    vec![Step::Start, Step::Write(code), Step::Stop]
}

/// Read `n` bytes from a fresh read phase (a command device has no pointer).
pub fn read_stream(n: usize) -> Vec<Step<'static>> {
    vec![Step::Start, Step::Read(n), Step::Stop]
}

/// One CS-framed SPI transfer: CS↓, clock `mosi`, CS↑.
pub fn spi_xfer(mosi: &[u8]) -> Vec<Step<'_>> {
    vec![Step::CsSelect, Step::Transfer(mosi), Step::CsRelease]
}

/// Concatenate script fragments into one script.
pub fn script<'a>(parts: impl IntoIterator<Item = Vec<Step<'a>>>) -> Vec<Step<'a>> {
    parts.into_iter().flatten().collect()
}
