// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ONE way for a test to name a scratch directory it owns exclusively.
//!
//! # The bug this ends
//!
//! Integration tests here named their output directory from a wall-clock
//! reading:
//!
//! ```ignore
//! let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
//! let out_dir = std::env::temp_dir().join(format!("labwired-cow-{nonce}"));
//! ```
//!
//! `as_nanos()` returns nanoseconds but the clock does not *tick* in
//! nanoseconds. Measured on the macOS dev host: the smallest non-zero delta
//! between consecutive readings is **1000 ns**, and 80 000 samples taken across
//! four threads yielded 6 270 distinct values — a **92 % collision rate**.
//!
//! `cargo test` runs a binary's tests in parallel by default, so two tests that
//! start in the same microsecond get the same "nonce", the same directory, and
//! then race over one set of files. The observed symptom was
//! `e2e_kw41z_cow_stimulus` failing with
//! `parse result.json: expected ',' or '}' at line 616` — line 616 of a
//! 14 492-line file, i.e. a TRUNCATED read, because the sibling test's
//! `remove_dir_all` ran while this one was still reading. It is intermittent and
//! load-dependent, which is what made it read as an unrelated regression when it
//! surfaced during an unrelated change.
//!
//! `std::process::id()` is not a fix either, and several tests use it: every
//! test inside one integration binary shares a pid, so parallel siblings still
//! collide. It only separates *binaries*.
//!
//! # Why a counter and not a better clock
//!
//! Uniqueness here should not be a probability. A process-local
//! [`AtomicUsize`] cannot repeat within a process no matter how fast tests
//! start, and the pid separates processes. The timestamp is kept only so a
//! leftover directory can be dated by a human; nothing depends on it being
//! distinct.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A temp-directory path unique to this call, for the lifetime of this machine.
///
/// Unique by CONSTRUCTION — pid (separates concurrent `cargo test` binaries)
/// plus a process-local counter (separates parallel tests inside one binary).
/// Neither can repeat, so there is no collision probability to reason about.
///
/// Does not create the directory; the caller decides whether it wants
/// `create_dir_all`, and remains responsible for cleanup.
///
/// ```ignore
/// let out_dir = labwired_cli::test_support::unique_temp_dir("labwired-cow");
/// std::fs::create_dir_all(&out_dir).expect("create out dir");
/// ```
pub fn unique_temp_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(unique_name(prefix))
}

/// The bare directory name [`unique_temp_dir`] would use — for the few callers
/// that build a file name rather than a directory.
pub fn unique_name(prefix: &str) -> String {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{seq}-{stamp}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// The property the old timestamp nonce did not have.
    ///
    /// Asserted at a rate no wall clock on this host can satisfy: the measured
    /// granularity is 1 µs, so 10 000 back-to-back timestamp reads would yield
    /// on the order of tens of distinct values. A counter yields 10 000.
    #[test]
    fn names_are_unique_under_back_to_back_calls() {
        let names: HashSet<String> = (0..10_000).map(|_| unique_name("probe")).collect();
        assert_eq!(
            names.len(),
            10_000,
            "unique_name must not repeat; this is the collision that made \
             parallel tests share a scratch directory"
        );
    }

    /// Uniqueness must not depend on the caller being single-threaded — the
    /// failing case was two tests started by the harness at the same instant.
    #[test]
    fn names_are_unique_across_threads() {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| (0..2_000).map(|_| unique_name("t")).collect::<Vec<_>>())
            })
            .collect();
        let all: Vec<String> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        let unique: HashSet<&String> = all.iter().collect();
        assert_eq!(unique.len(), all.len(), "collision across threads");
    }

    #[test]
    fn paths_live_under_the_temp_dir_and_carry_the_prefix() {
        let p = unique_temp_dir("labwired-example");
        assert!(p.starts_with(std::env::temp_dir()));
        assert!(p
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("labwired-example-"));
    }
}
