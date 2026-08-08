// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Host wall-clock probe for the STM32 SPI bit engine.
//!
//! Not a correctness gate (the repo convention for perf work — see
//! `crates/core/tests/bench_walk_free_kw41z.rs`): it prints numbers a human
//! reads. Correctness of the default path is gated deterministically by
//! `tests::spi_byte_level_golden`, which pins the wire byte for byte.
//!
//! Run:
//! ```text
//! cargo test -p labwired-core --release --lib bench_spi_engine -- --ignored --nocapture
//! ```
//!
//! Method (kept IDENTICAL across commits so the numbers are comparable):
//! one `Spi` in the classic STM32 layout, BR=0 so a frame is 16 peripheral
//! clock cycles, driven one cycle at a time through `tick_elapsed(1)` — the
//! most per-tick-sensitive way the engine is ever clocked. Each round times
//! `FRAMES` frames end to end; `ROUNDS` rounds are run and the MEDIAN is
//! reported together with the min/max spread, so a single scheduling hiccup
//! cannot masquerade as a regression.

#[cfg(test)]
mod bench_spi_engine_tests {
    use crate::peripherals::spi::{Spi, SpiDevice};
    use crate::Peripheral;
    use std::time::Instant;

    /// Frames per timed round.
    const FRAMES: u32 = 200_000;
    /// Timed rounds; the median is reported.
    const ROUNDS: usize = 7;

    /// Byte-level responder — the shape of every device model shipping today.
    pub(super) struct ByteSlave;
    impl SpiDevice for ByteSlave {
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi.rotate_left(3) ^ 0x5A
        }
        fn cs_pin(&self) -> &str {
            "PA4"
        }
    }

    /// Clock `FRAMES` frames through a fresh engine carrying `device` and
    /// return nanoseconds per frame.
    pub(super) fn ns_per_frame(device: Box<dyn SpiDevice>) -> f64 {
        let mut spi = Spi::new();
        let _lines = spi.line_levels_arc();
        spi.push_device(device);
        // SPE, BR=0 (half-period = 1 cycle), mode 0, 8-bit frames.
        spi.write_u16(0x00, 1 << 6).unwrap();
        let t = Instant::now();
        for i in 0..FRAMES {
            spi.write(0x0C, (i & 0xFF) as u8).unwrap();
            for _ in 0..16 {
                spi.tick_elapsed(1);
            }
        }
        let secs = t.elapsed().as_secs_f64();
        secs * 1.0e9 / f64::from(FRAMES)
    }

    /// Median / min / max over `ROUNDS` timed rounds.
    pub(super) fn measure(label: &str, mut make: impl FnMut() -> Box<dyn SpiDevice>) -> f64 {
        let mut v: Vec<f64> = (0..ROUNDS).map(|_| ns_per_frame(make())).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = v[ROUNDS / 2];
        println!(
            "{label:<28} median {median:7.1} ns/frame   min {:7.1}  max {:7.1}  spread {:+.1}%",
            v[0],
            v[ROUNDS - 1],
            (v[ROUNDS - 1] - v[0]) / median * 100.0
        );
        median
    }

    /// Default byte-level path. THIS is the arm compared across commits: it
    /// exists unchanged in the pre-edge-sampling commit, so running the same
    /// source on both builds measures exactly what the opt-in cost the ~100
    /// labs that never ask for it.
    #[test]
    #[ignore = "host wall-clock probe; run with --release -- --ignored --nocapture"]
    fn bench_default_byte_level_path() {
        println!("\n=== STM32 SPI bit engine — host cost per frame ===");
        measure("default (byte-level)", || Box::new(ByteSlave));
    }
}
