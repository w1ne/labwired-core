// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `Session` over the shared builder: `expect` stream semantics on the UART,
//! virtual-time timeouts, and `run_for` exactness. See `common::arm_fixture`
//! for where the fixture comes from and when these tests skip.

mod common;

use labwired_core::session::{OpenOptions, Session, SessionError};
use labwired_core::system::builder::*;
use std::time::Duration;

/// The open session, the text the fixture's `uart_contains` expects, and the
/// board clock (`SystemManifest::cpu_hz` over `ChipDescriptor::cpu_hz`) the
/// session must have resolved.
fn open_fixture() -> Option<(Session, String, u64)> {
    let (fw, chip, manifest, expected) = common::arm_fixture()?;
    let hz = manifest.cpu_hz.unwrap_or(chip.cpu_hz);
    let blobs = BlobMap::new();
    let s = Session::open(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(&fw),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        OpenOptions::default(),
    )
    .unwrap();
    Some((s, expected, hz))
}

#[test]
fn expect_matches_fixture_banner_and_reports_virtual_time() {
    let Some((mut s, expected, _)) = open_fixture() else {
        return;
    };
    let m = s
        .expect(&regex::escape(&expected), Duration::from_secs(5))
        .unwrap();
    assert!(m.text.contains(&expected));
    assert!(m.at > Duration::ZERO && m.at < Duration::from_secs(5));
}

#[test]
fn expect_timeout_is_virtual_not_wall() {
    let Some((mut s, _, _)) = open_fixture() else {
        return;
    };
    // 10 ms, not more: the timeout is simulated time at the board's clock (80
    // MHz on this fixture, so 800k cycles), and the wall-time bound below only
    // guards against a hang. A longer budget measures the CI runner, not the
    // session.
    let t0 = std::time::Instant::now();
    let err = s
        .expect("THIS NEVER PRINTS", Duration::from_millis(10))
        .unwrap_err();
    assert!(matches!(err, SessionError::ExpectTimeout { .. }));
    assert!(s.time() >= Duration::from_millis(10));
    assert!(
        t0.elapsed() < Duration::from_secs(10),
        "10 ms of virtual time must not take long"
    );
}

#[test]
fn overshoot_bytes_are_matched_by_the_next_expect() {
    let Some((mut s, expected, _)) = open_fixture() else {
        return;
    };
    // run well past the banner so it is already in the buffer, then expect it
    s.run_for(Duration::from_millis(20)).unwrap();
    let m = s
        .expect(&regex::escape(&expected), Duration::from_millis(1))
        .unwrap();
    assert!(m.text.contains(&expected));
}

#[test]
fn run_for_advances_exactly_cycles_over_hz() {
    let Some((mut s, _, hz)) = open_fixture() else {
        return;
    };
    assert_eq!(s.cpu_hz(), hz, "session clock is the board's clock");
    let c0 = s.cycles();
    s.run_for(Duration::from_millis(10)).unwrap();
    let dc = s.cycles() - c0;
    // if the machine halts earlier this fixture is wrong for the test
    assert_eq!(dc, hz / 100, "10 ms at the board clock");
}
