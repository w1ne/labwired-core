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
//! The probes serialize against each other through [`BENCH_LOCK`]: libtest
//! runs tests in parallel by default, and two timing probes sharing the CPU
//! produce garbage (measured: a 240% spread and a nonsensical negative cost).
//! The lock makes the file correct to run as a whole, not merely correct when
//! someone remembers `--test-threads=1`.
//!
//! ── Measured, 2026-08-08, M-series macOS, `--release` ──────────────────────
//! Default (byte-level) path, the arm that matters for the ~100 labs that
//! never ask for edge sampling, compared ACROSS COMMITS by building this same
//! source twice — once at the pre-edge-sampling commit, once at the
//! implementation commit — saving both test binaries and running them
//! alternately, five interleaved rounds:
//!
//! ```text
//!   round   pre-edge-sampling   with edge sampling
//!     1          200.1 ns             183.4 ns
//!     2          199.8 ns             181.6 ns
//!     3          197.6 ns             181.8 ns
//!     4          198.4 ns             185.1 ns
//!     5          199.6 ns             180.7 ns
//! ```
//!
//! The default path did not get slower; it measured ~9% FASTER, which the
//! change cannot cause by design (it adds one `Option` test per frame). Read
//! it as "code layout moved by ~9%, and the default path is not slower" —
//! and as the reason there is no wall-clock assertion here: if unrelated
//! layout swings this number by 9%, a margin tight enough to catch a real
//! per-frame regression would fire on innocent commits. The gates that DO
//! hold the default path are deterministic: `tests::spi_byte_level_golden`
//! (the wire, byte for byte) and
//! `peripherals::spi::tests::neither_path_consults_the_device_more_than_once_per_frame`
//! (the cost shape).
//!
//! Re-measured on the final tree (after the edge model moved to shared free
//! functions and the C3 gained support) the same way, 21 interleaved rounds,
//! this time on a host two other builds were hammering (load average 74):
//! absolute values inflate to ~800 ns/frame, but the median per-round ratio
//! final/pristine is **0.886** — the same conclusion the quiet run reached,
//! which is the point of comparing ratios rather than absolute times.
//!
//! Cost of turning the option ON, same build, interleaved round by round:
//! on a quiet host, 176-183 ns/frame default vs 224-230 ns/frame edge-sampled
//! = **+25% to +31% per frame (+45 to +54 ns)**. Under load average 85 the
//! same probe reports a median of +35% over seven repeats (range +11% to
//! +43%) — noisier, same order. Quote the quiet number; treat anything
//! measured on a loaded host as an upper bound.
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
    use crate::peripherals::spi::{Spi, SpiDevice, SpiSampling};
    use crate::Peripheral;
    use std::time::Instant;

    /// Held for the duration of every timed round: two probes timing at once
    /// measure each other's CPU contention, not the engine.
    static BENCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let _serialized = BENCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Edge-sampled responder: same work per byte as [`ByteSlave`], but opted
    /// in, so the difference between the two arms is the edge machinery and
    /// nothing else.
    struct EdgeSlave;
    impl SpiDevice for EdgeSlave {
        fn sampling(&self) -> SpiSampling {
            SpiSampling::edge_mode(1)
        }
        fn transfer(&mut self, mosi: u8) -> u8 {
            mosi.rotate_left(3) ^ 0x5A
        }
        fn cs_pin(&self) -> &str {
            "PA4"
        }
    }

    /// What turning the option ON costs.
    ///
    /// The two arms alternate ROUND BY ROUND and the reported figure is the
    /// median of the per-round ratio — not the ratio of two medians. Measured
    /// arm-by-arm instead, a load spike landing between the arms reads as a
    /// cost of anything from +35% to +201% (observed, on a host two other
    /// builds were sharing). Interleaved, both arms take the spike and the
    /// ratio survives it; the median then discards the rounds where they did
    /// not take it equally.
    #[test]
    #[ignore = "host wall-clock probe; run with --release -- --ignored --nocapture"]
    fn bench_edge_sampling_cost() {
        let _serialized = BENCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut pairs: Vec<(f64, f64)> = Vec::with_capacity(ROUNDS);
        for _ in 0..ROUNDS {
            let d = ns_per_frame(Box::new(ByteSlave));
            let e = ns_per_frame(Box::new(EdgeSlave));
            pairs.push((d, e));
        }
        let median = |mut v: Vec<f64>| {
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        };
        let d_med = median(pairs.iter().map(|p| p.0).collect());
        let e_med = median(pairs.iter().map(|p| p.1).collect());
        let ratio = median(pairs.iter().map(|p| p.1 / p.0).collect());
        println!("\n=== STM32 SPI bit engine — cost of `spi_mode` (edge sampling) ===");
        println!("default (byte-level)     median {d_med:7.1} ns/frame");
        println!("opt-in  (edge-sampled)   median {e_med:7.1} ns/frame");
        println!(
            "opt-in costs {:+.1}% per frame (median of {ROUNDS} interleaved rounds)\n",
            (ratio - 1.0) * 100.0
        );
    }
}
