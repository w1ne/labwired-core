// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE BUS TRACE HAS ONE HOME.
//!
//! Why this file exists
//! ====================
//! `crate::bus::bus_trace` calls itself the *universal* bus-transaction trace.
//! For a long time it was universal over exactly two buses. I²C and SPI
//! recorded into its shared ring; UART kept a `VecDeque<UartTraceEvent>` per
//! instance with its own 512-entry limit and its own `trace_seq`; FDCAN and
//! bxCAN each kept a `VecDeque<FdcanTraceFrame>` with a hand-copied 200-entry
//! limit and a third `trace_seq`. Three homes, three event shapes, three
//! sequence counters.
//!
//! That is the same shape as the seven bugs catalogued in
//! [`super::device_identity_one_home`], and unlike device identity it was
//! already broken, in three separate ways:
//!
//! 1. **Four of five UART models traced nothing at all.** The browser found
//!    UARTs with `downcast_ref::<Uart>()`. `EspUart` (ESP32-C3/S3), `Esp32Uart`,
//!    `Nrf52Uarte` and `Nrf54lUarte` are all UARTs and none of them IS a `Uart`,
//!    so on every ESP and nRF lab the analyzer's UART panel was permanently
//!    empty — with no error, because "no UART found" and "a UART that sent
//!    nothing" are the same empty array. This is the identical mistake
//!    `uart::UartStreamHost` was introduced to fix for cross-chip links; the
//!    trace path simply never got the same treatment.
//! 2. **No cycle stamp on UART or CAN.** `BusTraceEvent::cycle` promises the UI
//!    can "time-align protocol decode with sampled waveforms on one cycle axis".
//!    Only I²C and SPI could.
//! 3. **Three sequence counters ⇒ no cross-bus order.** Nothing could answer
//!    "did this UART byte precede that I²C address phase?", because the two
//!    numbers came from different counters.
//!
//! What this file gates
//! ====================
//! Two properties, one behavioural and one structural. Both are derived — the
//! expected side from live Rust data or from the source itself, never from a
//! list a person maintains and forgets.
//!
//! (1) `every_recording_peripheral_shares_the_machines_ring` — for every chip
//!     descriptor we ship, every registered peripheral that admits to recording
//!     ([`crate::Peripheral::bus_trace_handle`]) must hold the SAME ring as the
//!     bus, by `Arc` identity. This is the fails-CLOSED half. A model whose
//!     `attach_bus_trace` is never reached keeps the private handle it was born
//!     with: it records happily, nothing reads it, and the instrument shows an
//!     empty panel. Comparing contents cannot see that — two empty rings are
//!     equal — so the gate compares identity.
//!
//! (2) `no_peripheral_keeps_a_private_trace_ring` — no file under
//!     `peripherals/` may declare its own trace ring or trace-limit constant.
//!     This is the fails-if-FORKED half: it catches the next model that solves
//!     "I need a trace" by growing a `VecDeque` instead of recording into the
//!     one ring, which is how all three of the above started.
//!
//! Why a source scan for (2)
//! ========================
//! A private ring is invisible at runtime — that is the entire problem. A model
//! with its own `VecDeque<FooTraceEvent>` behaves correctly, tests green, and
//! is simply absent from the universal trace. There is no value to assert on,
//! so the gate reads the declaration instead. Same reasoning as
//! [`super::device_identity_one_home`], which reads the browser's source
//! because the lane that blocks a merge never links that crate.

use crate::bus::SystemBus;
use labwired_config::{ChipDescriptor, SystemManifest};
use std::path::PathBuf;

/// `crates/core` → repo root.
fn repo_root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn dummy_manifest(chip_path: &str) -> SystemManifest {
    SystemManifest {
        parts: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "bus-trace-one-home".to_string(),
        chip: chip_path.to_string(),
        cpu_hz: None,
        external_devices: vec![],
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        memory_overrides: Default::default(),
    }
}

/// Every chip descriptor we ship, enumerated FROM DISK so a chip added later
/// cannot escape the gate by not being listed here.
#[test]
fn every_recording_peripheral_shares_the_machines_ring() {
    let dir = repo_root("configs/chips");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {dir:?}: {e}"))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    paths.sort();

    let mut failures = Vec::new();
    let mut recorders = 0usize;
    let mut chips = 0usize;

    for path in paths {
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        // Internal harness fixtures, not silicon we ship.
        if stem.starts_with("ci-fixture-") {
            continue;
        }
        let Ok(chip) = ChipDescriptor::from_file(&path) else {
            continue; // loading is another gate's job
        };
        let abs = path.to_string_lossy().to_string();
        let Ok(bus) = SystemBus::from_config(&chip, &dummy_manifest(&abs)) else {
            continue;
        };
        chips += 1;

        for p in &bus.peripherals {
            let Some(handle) = p.dev.bus_trace_handle() else {
                continue; // carries no bus traffic
            };
            recorders += 1;
            if !handle.same_ring(&bus.bus_trace) {
                failures.push(format!(
                    "{stem}: peripheral '{}' records into a ring that is NOT the \
                     machine's. It kept the private handle it was constructed \
                     with, so everything it traces is written to a buffer nobody \
                     reads — the instrument will show an empty panel and no \
                     error. Its registration path must call \
                     `Peripheral::attach_bus_trace` (see the three funnels in \
                     `bus/construct.rs`).",
                    p.name
                ));
            }
        }
    }

    assert!(
        chips > 0,
        "no chip descriptors were built — the gate is vacuous"
    );
    assert!(
        recorders > 0,
        "no peripheral reported a trace handle across {chips} chips — either \
         `bus_trace_handle` was removed from every model, or the gate is \
         looking in the wrong place. Either way it is no longer proving \
         anything."
    );
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// Files allowed to own a trace ring, each with the reason.
///
/// Most are AIR traces, not wired buses, and they are outside this unification
/// on purpose: an over-the-air frame has no wire, no bus name and no place in a
/// wired-bus analyzer, and they already share their own one home
/// (`VirtualAirBus`). Wired and air are two honest axes; collapsing them would
/// make the model worse, not better. The IO-Link entry is different and is
/// flagged as debt in its own reason string.
const ALLOWED_PRIVATE_RINGS: &[(&str, &str)] = &[
    (
        "peripherals/ble_air.rs",
        "air trace (BLE/proprietary frames), shared via VirtualAirBus",
    ),
    (
        "peripherals/esp32c3/wifi_mac.rs",
        "air trace (802.11 frames), the WiFi analog of the BLE air bus",
    ),
    (
        "peripherals/nrf52/radio.rs",
        "air trace (VirtualAirBus itself lives here)",
    ),
    (
        "peripherals/components/iolink_master.rs",
        "IO-Link transfers are a DECODE of UART octets that are themselves in \
         the one ring, not a fourth wired bus — derivable, kept for now",
    ),
];

/// Every `.rs` file under `crates/core/src/peripherals`, recursively.
fn peripheral_sources() -> Vec<(String, String)> {
    fn walk(dir: &PathBuf, root: &PathBuf, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}")) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let src = std::fs::read_to_string(&path).expect("read source");
                out.push((format!("peripherals/{rel}"), src));
            }
        }
    }
    let root = repo_root("crates/core/src/peripherals");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out.sort();
    out
}

/// Does this line DECLARE a private trace ring or its eviction budget?
///
/// Two shapes, both taken from the three real forks this gate ends:
///   `trace: VecDeque<UartTraceEvent>,`   — the ring itself
///   `const UART_TRACE_LIMIT: usize = 512;` — its private budget
///
/// Deliberately not matched: `trace_seq`, which is meaningless on its own, and
/// any use of `BusTrace`, which is the one home and the thing we want models to
/// hold. Comment lines are skipped — prose describing a ring is not a ring.
fn declares_a_private_ring(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("//") {
        return false;
    }
    let ring = t.contains("VecDeque<") && {
        let after = t.split("VecDeque<").nth(1).unwrap_or("");
        let inner = after.split('>').next().unwrap_or("");
        inner.contains("Trace") || inner.contains("trace")
    };
    let budget = t.starts_with("const ") && t.contains("TRACE_LIMIT");
    ring || budget
}

#[test]
fn no_peripheral_keeps_a_private_trace_ring() {
    let allowed: std::collections::HashMap<&str, &str> =
        ALLOWED_PRIVATE_RINGS.iter().copied().collect();

    let sources = peripheral_sources();
    assert!(
        !sources.is_empty(),
        "no peripheral sources were scanned — the gate is vacuous"
    );

    let mut failures = Vec::new();
    for (rel, src) in &sources {
        if allowed.contains_key(rel.as_str()) {
            continue;
        }
        for (i, line) in src.lines().enumerate() {
            if declares_a_private_ring(line) {
                failures.push(format!(
                    "{rel}:{}: declares a private trace ring\n    {}\n  \
                     A peripheral must record into the machine's ONE bus trace \
                     (`Peripheral::attach_bus_trace` + `BusTrace::push`), not \
                     into a buffer of its own. A private ring is invisible to \
                     every instrument and to `bus_trace_snapshot`, and nothing \
                     at runtime can tell it apart from a bus that was simply \
                     quiet — which is how the UART and CAN traces stayed \
                     missing. If this really is an AIR trace and not a wired \
                     bus, add it to ALLOWED_PRIVATE_RINGS with the reason.",
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// The gate above must actually be able to fail. A scanner that matches nothing
/// passes every file for free, which is the vacuous-green pattern this codebase
/// has been bitten by before — so pin the detector against the exact three
/// declarations that used to exist, and against shapes it must NOT flag.
#[test]
fn the_private_ring_detector_is_not_vacuous() {
    // The three real forks, verbatim from the code this change removed.
    assert!(declares_a_private_ring(
        "    trace: VecDeque<UartTraceEvent>,"
    ));
    assert!(declares_a_private_ring(
        "    trace: VecDeque<FdcanTraceFrame>,"
    ));
    assert!(declares_a_private_ring(
        "const UART_TRACE_LIMIT: usize = 512;"
    ));

    // The one home, and things that merely mention tracing, must stay clean.
    assert!(!declares_a_private_ring(
        "    trace: crate::bus::bus_trace::BusTrace,"
    ));
    assert!(!declares_a_private_ring("    trace_name: String,"));
    assert!(!declares_a_private_ring(
        "    /// trace: VecDeque<UartTraceEvent>, (how it used to work)"
    ));
    assert!(!declares_a_private_ring("    rx_fifo: VecDeque<u8>,"));
}

/// The bug, end to end: an ESP-family UART must appear in the machine's trace.
///
/// This is the property that was broken for every ESP and nRF lab. It is
/// asserted on the SHARED ring — the same array `bus_trace_snapshot` hands the
/// browser — rather than on any per-model accessor, because "the model recorded
/// it somewhere" was never the problem. The problem was that what it recorded
/// never reached the one place the instruments read.
#[test]
fn an_esp_uart_reaches_the_shared_trace() {
    use crate::bus::bus_trace::BusPayload;

    let path = repo_root("configs/chips/esp32c3.yaml");
    let chip = ChipDescriptor::from_file(&path).expect("load esp32c3 chip yaml");
    let abs = path.to_string_lossy().to_string();
    let mut bus = SystemBus::from_config(&chip, &dummy_manifest(&abs)).expect("build c3 bus");

    assert!(
        bus.bus_trace_snapshot().is_empty(),
        "a freshly built machine has transacted nothing"
    );

    // Push a byte through UART0's FIFO and let it shift out at baud.
    let idx = bus
        .find_peripheral_index_by_name("uart0")
        .expect("the C3 carries uart0");
    bus.peripherals[idx]
        .dev
        .write_u32(0x00, b'K' as u32)
        .expect("FIFO write");
    for _ in 0..200_000 {
        bus.peripherals[idx].dev.tick();
        if !bus.bus_trace_snapshot().is_empty() {
            break;
        }
    }

    let events = bus.bus_trace_snapshot();
    let uart: Vec<_> = events
        .iter()
        .filter(|e| matches!(e.payload, BusPayload::Uart { .. }))
        .collect();
    assert!(
        !uart.is_empty(),
        "the C3's UART0 shifted a byte out and the machine's trace stayed \
         empty. This is the original defect: an `EspUart` is not a `Uart`, so \
         nothing recorded it and the analyzer had nothing to show."
    );
    assert_eq!(uart[0].bus, "uart0", "stamped with the peripheral's own id");
    match uart[0].payload {
        BusPayload::Uart { byte, .. } => assert_eq!(byte, b'K'),
        _ => unreachable!(),
    }
}
